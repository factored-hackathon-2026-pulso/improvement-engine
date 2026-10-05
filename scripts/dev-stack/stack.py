#!/usr/bin/env python3
"""Pulso LOCAL agent-core dev stack: up / down / health. See docs/dev/LOCAL_STACK.md.

    python scripts/dev-stack/stack.py up [--rebuild-gateway] [--reset-db]
    python scripts/dev-stack/stack.py health
    python scripts/dev-stack/stack.py down [--purge]

Secrets: values come from the two env files (PULSO_AGENT_CORE_ENV / PULSO_LLM_GATEWAY_ENV, default
<factored>/agent-core.env and llm-gateway.env). They are loaded into child-process environments only; they are never
printed, logged, written to another file or put on an argv.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from urllib.parse import urlparse

REPO = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
STATE = REPO / ".dev-stack"
CONN = os.environ.get("PULSO_PODMAN_CONNECTION", "pulso-dev-root")
PG, GW, GW_IMAGE, PG_VOL = "pulso-l3-postgres", "pulso-l3-llm-gateway", "pulso-l3-llm-gateway", "pulso-l3-pgdata"
PG_PORT, GW_PORT, CORE_PORT = 55432, 8080, 8001
AGENT_CORE_REPO = "https://github.com/pulso-factored/agent-core.git"
GATEWAY_REPO = "https://github.com/pulso-factored/llm-gateway.git"
# PULSO_REGISTRY_DIR: import another registry directory instead (EV1: agent-core tests/fixtures/registry-e2e).
REGISTRY_DIR = Path(os.environ.get("PULSO_REGISTRY_DIR") or (HERE / "registry-pulso-builder"))


def env_path(var: str, default: str) -> Path:
    # worktree layout: <factored>/worktrees/<wt>  ->  <factored>/<default>
    return Path(os.environ.get(var) or (REPO.parents[1] / default))


def load_env(path: Path) -> dict[str, str]:
    """Parse KEY=VALUE lines (optionally quoted). Values stay in memory."""
    out: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        v = v.strip()
        if len(v) >= 2 and v[0] == v[-1] and v[0] in "'\"":
            v = v[1:-1]
        if v:
            out[k.strip()] = v
    return out


def sh(args, env=None, cwd=None, check=True):
    res = subprocess.run(args, env={**os.environ, **(env or {})}, cwd=cwd, text=True, capture_output=True)
    if check and res.returncode != 0:
        tail = ((res.stderr or "") + (res.stdout or ""))[-600:]
        sys.exit(f"FAILED ({res.returncode}): {' '.join(args[:4])} ...\n{tail}")
    return res


def pm(*a, env=None, check=True):
    return sh(["podman", "--connection", CONN, *a], env=env, check=check)


def source_dir(var: str, name: str, url: str) -> Path:
    d = Path(os.environ.get(var) or (STATE / "src" / name))
    if not d.exists():
        d.parent.mkdir(parents=True, exist_ok=True)
        sh(["git", "clone", "--quiet", url, str(d)])
    return d


def wait(fn, what: str, tries: int = 40, delay: float = 2.0) -> None:
    for _ in range(tries):
        try:
            if fn():
                return
        except Exception:
            pass
        time.sleep(delay)
    sys.exit(f"timeout waiting for {what}")


def http_ok(url: str) -> bool:
    with urllib.request.urlopen(url, timeout=3) as r:
        return r.status == 200


def stop_core() -> None:
    pidf = STATE / "serve.pid"
    if pidf.exists():
        pid = pidf.read_text().strip()
        cmd = ["taskkill", "/PID", pid, "/T", "/F"] if os.name == "nt" else ["kill", pid]
        subprocess.run(cmd, capture_output=True)
        pidf.unlink()


def serve_ports() -> list[str]:
    """Port flags for `agentcore serve`. PULSO_SERVE_E2E=1 uses agent-core's e2e demo doubles (tools, classifier,
    calibration with the thresholds the registry-e2e agents reference), as scripts/e2e/serve.ps1 does."""
    if os.environ.get("PULSO_SERVE_E2E") == "1":
        return ["--tools", "testing.e2e_demo:tools", "--classifier", "testing.e2e_demo:classifier_provider",
                "--field-classifier", "testing.e2e_demo:field_classifier", "--calibration", "testing.e2e_demo:calibration"]
    return ["--field-classifier", os.environ.get("PULSO_FIELD_CLASSIFIER", "agent_core.adapters.classification:field_classifier")]


def up(args) -> None:
    ac_env = load_env(env_path("PULSO_AGENT_CORE_ENV", "agent-core.env"))
    gw_env = load_env(env_path("PULSO_LLM_GATEWAY_ENV", "llm-gateway.env"))
    # the gateway consumer token may be blank in llm-gateway.env: the same name is set in agent-core.env
    for k in ("GATEWAY_TOKEN_AGENT_CORE", "OPENROUTER_API_KEY"):
        if k not in gw_env and k in ac_env:
            gw_env[k] = ac_env[k]
    STATE.mkdir(exist_ok=True)
    # 1. Postgres 16 (credentials from the DSN in the env file, passed by inheritance, never by argv)
    dsn = urlparse(ac_env["AGENTCORE_REGISTRY_DSN"])
    user = dsn.username or "agentcore"
    pg_env = {"POSTGRES_USER": user, "POSTGRES_PASSWORD": dsn.password or "",
              "POSTGRES_DB": (dsn.path or "/agentcore").lstrip("/")}
    if args.reset_db:
        pm("rm", "-f", PG, check=False)
        pm("volume", "rm", "-f", PG_VOL, check=False)
    if pm("container", "exists", PG, check=False).returncode != 0:
        pm("run", "-d", "--pids-limit=0", "--name", PG, "-e", "POSTGRES_USER", "-e", "POSTGRES_PASSWORD", "-e", "POSTGRES_DB",
           "-p", f"127.0.0.1:{PG_PORT}:5432", "-v", f"{PG_VOL}:/var/lib/postgresql/data",
           "docker.io/library/postgres:16-alpine", env=pg_env)
    else:
        pm("start", PG, check=False)
    wait(lambda: pm("exec", PG, "pg_isready", "-U", user, check=False).returncode == 0, "postgres")
    time.sleep(3)
    has = pm("exec", PG, "psql", "-U", user, "-d", pg_env["POSTGRES_DB"], "-tAc",
             "SELECT 1 FROM pg_database WHERE datname='agentcore_eval'").stdout
    if "1" not in has:
        pm("exec", PG, "psql", "-U", user, "-d", pg_env["POSTGRES_DB"], "-c", "CREATE DATABASE agentcore_eval")
    print("postgres 16 up on", PG_PORT)
    # 2. llm-gateway (built from its Dockerfile; no Go on the host)
    gw_src = source_dir("PULSO_LLM_GATEWAY_DIR", "llm-gateway", GATEWAY_REPO)
    if args.rebuild_gateway or pm("image", "exists", GW_IMAGE, check=False).returncode != 0:
        print("building llm-gateway image (heavy, one-off) ...")
        pm("build", "-q", "-t", GW_IMAGE, str(gw_src))
    pm("rm", "-f", GW, check=False)
    names = [k for k in ("GATEWAY_CONSUMERS", "LLM_ENDPOINTS", "GATEWAY_TOKEN_AGENT_CORE", "OPENROUTER_API_KEY",
                         "JEV_API_KEY") if k in gw_env]
    pm("run", "-d", "--pids-limit=0", "--name", GW, "-p", f"127.0.0.1:{GW_PORT}:8080", *[a for k in names for a in ("-e", k)],
       GW_IMAGE, env=gw_env)
    wait(lambda: http_ok(f"http://127.0.0.1:{GW_PORT}/healthz"), "llm-gateway")
    print("llm-gateway up on", GW_PORT)
    # 3. agent-core (host process via uv)
    ac = source_dir("PULSO_AGENT_CORE_DIR", "agent-core", AGENT_CORE_REPO)
    # the LOCAL gateway only accepts the consumer token it was started with (the one named GATEWAY_TOKEN_AGENT_CORE);
    # AGENTCORE_LLM_GATEWAY_TOKEN in agent-core.env differs from it, so the core is pointed at the local pair.
    ac_env["AGENTCORE_LLM_GATEWAY_URL"] = f"http://127.0.0.1:{GW_PORT}"
    if "GATEWAY_TOKEN_AGENT_CORE" in gw_env:
        ac_env["AGENTCORE_LLM_GATEWAY_TOKEN"] = gw_env["GATEWAY_TOKEN_AGENT_CORE"]
    env = {**ac_env, "AGENTCORE_FIELD_CLASSIFICATION_FILES": str(HERE / "field-overlay.json")}
    sh(["uv", "sync", "--locked"], cwd=ac)
    sh(["uv", "run", "agentcore", "migrate"], env=env, cwd=ac)
    ident = ["uv", "run", "--project", str(ac), "python", str(HERE / "identity.py")]
    sh([*ident, "keys", "--state-dir", str(STATE)], cwd=ac)
    sh([*ident, "mint", "--state-dir", str(STATE)], cwd=ac)
    tokens = json.loads((STATE / "tokens.json").read_text(encoding="utf-8"))
    imp = sh(["uv", "run", "agentcore", "registry", "--verifier", "testing.registry_demo:demo_verifier", "import",
              str(REGISTRY_DIR)], env={**env, "AGENTCORE_CREDENTIAL": tokens["admin"]}, cwd=ac, check=False)
    out = (imp.stdout or "") + (imp.stderr or "")
    if imp.returncode != 0 and "ya tiene releases" not in out:
        sys.exit("registry import failed:\n" + out[-800:])
    print("registry:", "already imported" if imp.returncode != 0 else "pulso-builder imported")
    stop_core()
    log = open(STATE / "serve.log", "ab")
    flags = (0x00000008 | 0x00000200) if os.name == "nt" else 0  # DETACHED_PROCESS | NEW_PROCESS_GROUP
    proc = subprocess.Popen(
        ["uv", "run", "agentcore", "serve", "--port", str(CORE_PORT), "--registry-api",
         "--identity-keys", str(STATE / "identity-keys.json"), "--staff-keys", str(STATE / "staff-keys.json"),
         "--lang-thresholds", str(ac / "scripts" / "e2e" / "lang-thresholds.json"), "--agents", os.environ.get("PULSO_SERVE_AGENTS", "pulso-builder"),
         *serve_ports()],
        cwd=ac, env={**os.environ, **env}, stdout=log, stderr=log, creationflags=flags)
    (STATE / "serve.pid").write_text(str(proc.pid))
    wait(lambda: http_ok(f"http://127.0.0.1:{CORE_PORT}/healthz"), "agent-core serve", tries=60)
    print(f"agent-core up on {CORE_PORT} (pid {proc.pid}); try: python scripts/dev-stack/invoke_builder.py")


def down(args) -> None:
    stop_core()
    pm("rm", "-f", GW, PG, check=False)
    if args.purge:
        pm("volume", "rm", "-f", PG_VOL, check=False)
        pm("rmi", "-f", GW_IMAGE, check=False)
        shutil.rmtree(STATE, ignore_errors=True)
    print("stack down" + (" (purged)" if args.purge else ""))


def health(_) -> int:
    ok = True

    def line(name: str, good: bool, extra: str = "") -> None:
        nonlocal ok
        ok &= good
        print(f"{'OK  ' if good else 'FAIL'} {name} {extra}")

    for c in (PG, GW):
        r = pm("inspect", "-f", "{{.State.Status}}", c, check=False)
        line(c, r.stdout.strip() == "running", r.stdout.strip())
    for name, url in (("llm-gateway /healthz", f"http://127.0.0.1:{GW_PORT}/healthz"),
                      ("agent-core /healthz", f"http://127.0.0.1:{CORE_PORT}/healthz"),
                      ("agent-core /readyz", f"http://127.0.0.1:{CORE_PORT}/readyz")):
        try:
            line(name, http_ok(url))
        except Exception as exc:
            line(name, False, type(exc).__name__)
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    u = sub.add_parser("up")
    u.add_argument("--rebuild-gateway", action="store_true")
    u.add_argument("--reset-db", action="store_true", help="drop the Postgres volume (needed if an entity changes)")
    d = sub.add_parser("down")
    d.add_argument("--purge", action="store_true", help="also drop DB volume, gateway image and local state")
    sub.add_parser("health")
    args = ap.parse_args()
    if args.cmd == "up":
        up(args)
    elif args.cmd == "down":
        down(args)
    else:
        return health(args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
