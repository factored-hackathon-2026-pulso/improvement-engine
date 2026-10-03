"""Stack provisioning helpers for the E2E (host side). CLI used by e2e-core/run.ps1:

  python -m codex_standin.stack prepare  --ns NS --dir DIR        core.env lines (no secrets in git)
  python -m codex_standin.stack fixtures --ns NS --dir DIR        starts the `e2e-fixtures` DOUBLE container
  python -m codex_standin.stack cleanup  --ns NS --dir DIR [--tag TAG]

Only machine `pulso-dev` (connection passed explicitly). Everything created carries com.pulso.team=claude and
com.pulso.namespace=NS so `local/core/reset.ps1 -Namespace NS -Confirm` removes it."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

from codex_standin import jwtsvc

PODMAN = r"C:\Users\alexg\AppData\Local\Programs\Podman\podman.exe"
CONNECTION = "pulso-dev"
FIXTURE_ALIAS = "e2e-fixtures"
FIXTURE_PORT = 8700
TENANT_ID = "tenant-local"  # PULSO_TENANT_ID of the stack (compose default; the runtime requires it)
SERVICE_KID = "control-api-core-bridge"  # kid core-keygen issues for iss=control-api -> core-bridge
REPO = Path(__file__).resolve().parents[3]
SECRETS = REPO / "local" / ".secrets"


def podman(*args: str, check: bool = True, input_text: str | None = None) -> str:
    proc = subprocess.run([PODMAN, "--connection", CONNECTION, *args], capture_output=True, text=True,
                          input=input_text, timeout=600)
    if check and proc.returncode != 0:
        raise RuntimeError(f"podman {' '.join(args[:3])} failed: {(proc.stderr or proc.stdout).strip()[:400]}")
    return proc.stdout.strip()


def project(ns: str) -> str:
    return f"pulso-{ns}"


def state_of(ns: str) -> dict[str, Any]:
    return json.loads((SECRETS / ns / "state.json").read_text(encoding="utf-8-sig"))  # type: ignore[no-any-return]


def fixtures_url() -> str:
    return f"http://{FIXTURE_ALIAS}:{FIXTURE_PORT}"


LLM_KEY_VALUE = "scripted-not-a-secret"


def gateway_url() -> str:
    return fixtures_url() + "/llm"  # the runtime posts to <url>/v1/generate (agent-core llm-gateway service protocol)


BUDGETS = {"bud-e2e": {"cost_usd_max": "5", "tokens_max": 200000, "jobs_max": 50}}


def core_env_lines() -> list[str]:
    """Extra lines for the stack's core.env (the compose env file), all forwarded by compose.core.yaml to core-runtime:
    the scripted llm-gateway (AGENTCORE_LLM_GATEWAY_URL / _TOKEN of the pinned agent-core) and the eval budgets table
    inline (PULSO_EVAL_BUDGETS_JSON). Throw-away values, no image overlay needed."""
    return [f"AGENTCORE_LLM_GATEWAY_URL={gateway_url()}", f"AGENTCORE_LLM_GATEWAY_TOKEN={LLM_KEY_VALUE}",
            "PULSO_EVAL_BUDGETS_JSON=" + json.dumps(BUDGETS, separators=(",", ":"))]


def prepare(ns: str, out: Path) -> dict[str, Any]:
    """Writes the E2E inputs: the core.env LLM/budgets lines under the git-ignored local/.secrets/<ns>. No key
    material: the control-api -> bridge service key is the stack's own (control-api-core-bridge.key), read from the keys
    volume after start."""
    out.mkdir(parents=True, exist_ok=True)
    secrets = SECRETS / ns
    secrets.mkdir(parents=True, exist_ok=True)
    core_env = secrets / "core.env"
    existing = core_env.read_text(encoding="utf-8") if core_env.exists() else ""
    core_env.write_text(existing + "".join(line + "\n" for line in core_env_lines()), encoding="utf-8")
    keys = {"service_kid": SERVICE_KID, "namespace": ns, "fixtures_url": fixtures_url()}
    (out / "e2e-keys.json").write_text(json.dumps(keys), encoding="ascii")
    return keys


def read_volume_file(ns: str, name: str) -> str:
    return podman("exec", f"{project(ns)}-core-runtime-1", "cat", f"/run/pulso-keys/{name}")


def build_verify_keys(cb: dict[str, Any], service: dict[str, Any], trust: dict[str, Any]) -> dict[str, Any]:
    """Key material of the fixtures double, taken AS ISSUED by core-keygen (no issuer override: the exporter
    signs iss=core-bridge and keygen registers it so)."""
    cb_pub = jwtsvc.public_of(jwtsvc.private_from_seed(cb["key"]))
    ingest_keys = {kid: v for kid, v in service["keys"].items() if kid.startswith("exporter-")}
    return {
        "ring": {cb["kid"]: ["core-bridge", "control-api", cb_pub],
                 **{kid: [v["iss"], v["aud"], v["key"]] for kid, v in trust["keys"].items()}},
        "ingest": {"keys": ingest_keys, "binding_ref": "binding-local", "tenant_id": TENANT_ID}}


def verify_keys(ns: str) -> dict[str, Any]:
    return build_verify_keys(json.loads(read_volume_file(ns, "bridge-callback.json")),
                             json.loads(read_volume_file(ns, "service.json")),
                             json.loads(read_volume_file(ns, "lab-broker-trust.json")))


def start_fixtures(ns: str, ctx_dir: Path) -> dict[str, Any]:
    st = state_of(ns)
    keys = json.loads((ctx_dir / "e2e-keys.json").read_text(encoding="ascii"))
    # the control-api -> core-bridge seed is the stack's own (core-keygen); stand-ins read it, nothing is generated here
    keys["service_seed"] = read_volume_file(ns, "control-api-core-bridge.key").strip()
    (ctx_dir / "e2e-keys.json").write_text(json.dumps(keys), encoding="ascii")
    sim_image = next(line.split("=", 1)[1] for line in (SECRETS / ns / "ports.env").read_text().splitlines()
                     if line.startswith("PULSO_SIM_IMAGE="))
    name = f"{project(ns)}-{FIXTURE_ALIAS}-1"
    port = _free_port()
    podman("rm", "-f", name, check=False)
    podman("create", "--name", name, "--network", f"{project(ns)}_core-net", "--network-alias", FIXTURE_ALIAS,
           "--cgroups=disabled", "--pids-limit=0", "--label", "com.pulso.team=claude", "--label",
           f"com.pulso.namespace={ns}", "--label", "com.pulso.role=double", "--label",
           "com.pulso.piece=e2e-fixtures", "-p", f"127.0.0.1:{port}:{FIXTURE_PORT}",
           "-e", "PYTHONPATH=/opt/e2e:/sim:/opt/pulso/lib", "-e", f"E2E_PORT={FIXTURE_PORT}",
           "-e", "E2E_VERIFY_KEYS=" + json.dumps(verify_keys(ns), separators=(",", ":")),
           "--entrypoint", json.dumps(["python", "-m", "codex_standin.serve"]), sim_image)
    stage = Path(tempfile.mkdtemp(prefix="e2e-stage-"))
    try:
        shutil.copytree(Path(__file__).parent, stage / "opt" / "e2e" / "codex_standin",
                        ignore=shutil.ignore_patterns("__pycache__"))
        shutil.copytree(REPO / "platform-sim" / "ingest_fixture", stage / "sim" / "ingest_fixture",
                        ignore=shutil.ignore_patterns("__pycache__"))
        podman("cp", str(stage) + "\\.", f"{name}:/")
    finally:
        shutil.rmtree(stage, ignore_errors=True)
    podman("start", name)
    info = {"container": name, "host_port": port, "url": f"http://127.0.0.1:{port}", "internal_url": fixtures_url(),
            "namespace": ns, "connection": CONNECTION, "ports": st["ports"], "image": st["image"],
            "expected_image_digest": st["expected_image_digest"], "project": project(ns)}
    (ctx_dir / "e2e-env.json").write_text(json.dumps(info), encoding="ascii")
    return info


def _free_port() -> int:
    import socket
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


def cleanup(ns: str, ctx_dir: Path | None) -> None:
    podman("rm", "-f", f"{project(ns)}-{FIXTURE_ALIAS}-1", check=False)
    if ctx_dir and ctx_dir.exists():
        shutil.rmtree(ctx_dir, ignore_errors=True)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["prepare", "fixtures", "cleanup"])
    ap.add_argument("--ns", default="")
    ap.add_argument("--dir", default="")
    ap.add_argument("--base", default="")
    ap.add_argument("--tag", default="")
    a = ap.parse_args(argv)
    d = Path(a.dir) if a.dir else None
    if a.cmd == "prepare":
        assert d is not None
        prepare(a.ns, d)
        print("prepared")
    elif a.cmd == "fixtures":
        assert d is not None
        print(json.dumps(start_fixtures(a.ns, d)))
    else:
        cleanup(a.ns, d)
        print("cleaned")
    return 0


if __name__ == "__main__":
    sys.exit(main())
