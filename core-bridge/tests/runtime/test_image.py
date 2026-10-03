"""Image contract: static Dockerfile checks always; runtime checks when PULSO_TEST_IMAGE is set (podman)."""

from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

import pytest

pytestmark = pytest.mark.runtime
DOCKERFILE = (Path(__file__).resolve().parents[2] / "Dockerfile").read_text()
PODMAN = os.environ.get("PULSO_PODMAN", r"C:\Users\alexg\AppData\Local\Programs\Podman\podman.exe")


def test_dockerfile_is_multistage_nonroot_and_excludes_test_code() -> None:
    assert len(re.findall(r"^FROM ", DOCKERFILE, re.M)) == 2
    assert re.search(r"^USER 10001", DOCKERFILE, re.M)
    assert "COPY src/pulso_core_runtime" in DOCKERFILE
    for forbidden in ("COPY tests", "COPY testing", "--group dev", "docker-compose"):
        assert forbidden not in DOCKERFILE
    assert "contracts/VERSION" in DOCKERFILE and "uv export --frozen" in DOCKERFILE


def _run(cmd: list[str], *, entrypoint: str | None = None, env: dict[str, str] | None = None
         ) -> subprocess.CompletedProcess[str]:
    conn = os.environ.get("PULSO_PODMAN_CONNECTION", "pulso-dev")
    argv = [PODMAN, "--connection", conn, "run", "--rm", "--cgroups=disabled", "--read-only"]
    if entrypoint:
        argv += ["--entrypoint", entrypoint]
    for k, v in (env or {}).items():
        argv += ["-e", f"{k}={v}"]
    return subprocess.run([*argv, os.environ["PULSO_TEST_IMAGE"], *cmd], capture_output=True, text=True,
                          timeout=120)


needs_image = pytest.mark.skipif("PULSO_TEST_IMAGE" not in os.environ, reason="PULSO_TEST_IMAGE not set")


def test_dockerfile_installs_the_runtime_only_dependencies() -> None:
    reqs = (Path(__file__).resolve().parents[2] / "runtime-requirements.txt").read_text()
    assert "jsonschema" in reqs and "referencing" in reqs
    assert "runtime-requirements.txt" in DOCKERFILE and "jsonschema referencing" in DOCKERFILE


@needs_image
def test_image_imports_the_facts_whitelist() -> None:
    r = _run(["-c", "import pulso_core_runtime.facts.whitelist, jsonschema, referencing; print('ok')"],
             entrypoint="python")
    assert r.returncode == 0 and "ok" in r.stdout, r.stderr[-400:]


@needs_image
def test_image_has_no_testing_package_and_agentcore_works() -> None:
    assert _run(["-c", "import testing"], entrypoint="python").returncode != 0
    out = _run(["--help"], entrypoint="agentcore")
    assert out.returncode == 0 and "serve" in out.stdout


@needs_image
def test_image_refuses_demo_flag_with_exit_2() -> None:
    r = _run(["runtime"], env={"AGENTCORE_ALLOW_DEMO": "1"})
    assert r.returncode == 2 and "pulso:demo_double_in_real_mode" in r.stderr


@needs_image
def test_image_runs_as_non_root() -> None:
    r = _run(["-u"], entrypoint="id")
    assert r.stdout.strip() == "10001"


def test_entrypoint_offers_exactly_the_modules_that_exist() -> None:
    script = (Path(__file__).resolve().parents[2] / "docker-entrypoint.sh").read_text()
    names = set(re.findall(r"^  ([a-z|]+)\)", script, re.MULTILINE))
    assert names == {"runtime", "migrate", "agentcore", "exporter"}
    src = Path(__file__).resolve().parents[2] / "src" / "pulso_core_runtime"
    assert (src / "main.py").is_file() and (src / "exporter" / "__main__.py").is_file()
    for gone in ("seed", "bootstrap", "sweep"):
        assert gone not in script.split("case", 1)[1]


def test_build_context_is_prepared_without_upstream_dockerignore() -> None:
    """agent-core 789d6c8 ships a .dockerignore that excludes `contracts`; the build must not depend on it."""
    script = (Path(__file__).resolve().parents[2] / "scripts" / "build-image.ps1").read_text()
    assert "prepare-core-context.ps1" in script and "core=$ctx" in script and "core=$Checkout" not in script


def test_prepare_core_context_exports_contracts_version(tmp_path: Path) -> None:
    import os
    import shutil
    import subprocess

    checkout = Path(os.environ.get("PULSO_CORE_CHECKOUT", r"D:\.codex\factored\references\agent-core-789d6c8"))
    pwsh = shutil.which("pwsh")
    if pwsh is None or not (checkout / ".git").exists():
        pytest.skip("pwsh or the pinned checkout is not available")
    sha = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    script = Path(__file__).resolve().parents[2] / "scripts" / "prepare-core-context.ps1"
    out = tmp_path / "ctx"
    r = subprocess.run([pwsh, "-NoProfile", "-File", str(script), "-Checkout", str(checkout), "-PinSha", sha, "-Out", str(out)],
                       capture_output=True, text=True)
    assert r.returncode == 0, r.stderr
    assert (out / "contracts" / "VERSION").is_file() and not (out / ".dockerignore").exists()
    assert (out / "pyproject.toml").is_file() and (out / "uv.lock").is_file()
    bad = subprocess.run([pwsh, "-NoProfile", "-File", str(script), "-Checkout", str(checkout), "-PinSha", "0" * 40,
                          "-Out", str(tmp_path / "x")], capture_output=True, text=True)
    assert bad.returncode != 0


def test_concurrent_prepares_for_the_same_sha_do_not_collide() -> None:
    import shutil
    import subprocess

    checkout = Path(os.environ.get("PULSO_CORE_CHECKOUT", r"D:\.codex\factored\references\agent-core-789d6c8"))
    pwsh = shutil.which("pwsh")
    if pwsh is None or not (checkout / ".git").exists():
        pytest.skip("pwsh or the pinned checkout is not available")
    sha = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    script = Path(__file__).resolve().parents[2] / "scripts" / "prepare-core-context.ps1"
    argv = [pwsh, "-NoProfile", "-File", str(script), "-Checkout", str(checkout), "-PinSha", sha]
    procs = [subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for _ in range(2)]
    results = [p.communicate() for p in procs]
    dirs = [Path(o.strip().splitlines()[-1]) for o, _ in results]
    try:
        assert [p.returncode for p in procs] == [0, 0], [e[-300:] for _, e in results]
        assert dirs[0] != dirs[1]
        for d in dirs:
            assert (d / "contracts" / "VERSION").is_file()
    finally:
        for d in dirs:
            shutil.rmtree(d, ignore_errors=True)


def test_build_image_cleans_up_its_context() -> None:
    script = (Path(__file__).resolve().parents[2] / "scripts" / "build-image.ps1").read_text()
    assert "finally" in script and "Remove-Item" in script and "$ctx" in script.split("finally", 1)[1]


# ---- key delivery from env (Fargate-injected secrets, ADR 0009) ----

import base64
import json


def _b64(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).decode().rstrip("=")


def _signer(n: int) -> str:
    return json.dumps({"kid": f"k{n}", "key": _b64(bytes([n]) * 32)})


def _runtime_key_env() -> dict[str, str]:
    env = {f"PULSO_BRIDGE_{n}_SIGNER_JSON": _signer(i + 1) for i, n in
           enumerate(("IDENTITY", "STAFF", "CALLBACK", "EXECUTOR"))}
    env["CORE_IDENTITY_KEYS_JSON"] = json.dumps({"principal_keys": {"k": _b64(b"\x09" * 32)}})
    env["CORE_STAFF_KEYS_JSON"] = json.dumps({"staff_keys": {"k": _b64(b"\x0a" * 32)}})
    env["PULSO_SERVICE_KEYS_JSON"] = json.dumps(
        {"keys": {"s1": {"iss": "control-api", "aud": "pulso-core-runtime", "key": _b64(b"\x0b" * 32)}}})
    return env


def test_entrypoint_script_scrubs_and_documents_every_key_variable() -> None:
    script = (Path(__file__).resolve().parents[2] / "docker-entrypoint.sh").read_text()
    readme = (Path(__file__).resolve().parents[2] / "README.md").read_text()
    for name in (*_runtime_key_env(), "PULSO_EXPORTER_KEY_CONTROL_API_SEED", "PULSO_EXPORTER_KEY_LAB_BROKER_SEED"):
        assert name in script and name in readme, name
    assert "unset" in script and "/run/pulso-keys" in script


@needs_image
def test_runtime_without_any_key_env_fails_closed_naming_variables_not_values() -> None:
    r = _run(["runtime"], env={})
    assert r.returncode == 2 and "pulso:runtime_config_invalid" in r.stderr
    assert "PULSO_BRIDGE_EXECUTOR_SIGNER_JSON" in r.stderr


@needs_image
def test_runtime_missing_one_signer_fails_closed() -> None:
    env = _runtime_key_env()
    del env["PULSO_BRIDGE_CALLBACK_SIGNER_JSON"]
    r = _run(["runtime"], env=env)
    assert r.returncode == 2 and "PULSO_BRIDGE_CALLBACK_SIGNER_JSON" in r.stderr
    assert _b64(b"\x01" * 32) not in r.stderr and _b64(b"\x09" * 32) not in r.stderr


@needs_image
def test_runtime_rejects_malformed_signer_without_echoing_it() -> None:
    env = _runtime_key_env()
    env["PULSO_BRIDGE_STAFF_SIGNER_JSON"] = json.dumps({"kid": "k", "key": _b64(b"short")})
    r = _run(["runtime"], env=env)
    assert r.returncode == 2 and "PULSO_BRIDGE_STAFF_SIGNER_JSON" in r.stderr
    assert _b64(b"short") not in r.stderr


@needs_image
def test_runtime_refuses_executor_key_equal_to_callback_key() -> None:
    env = _runtime_key_env()
    env["PULSO_BRIDGE_EXECUTOR_SIGNER_JSON"] = json.dumps({"kid": "other", "key": _b64(bytes([3]) * 32)})
    r = _run(["runtime"], env=env)
    assert r.returncode == 2 and "pulso:runtime_config_invalid" in r.stderr
    assert "EXECUTOR" in r.stderr and "CALLBACK" in r.stderr and _b64(bytes([3]) * 32) not in r.stderr


@needs_image
def test_runtime_with_env_keys_only_gets_past_key_materialisation() -> None:
    """No files mounted: main starts, reads every key file and only then stops at the first non-key config gap."""
    env = _runtime_key_env() | {"PULSO_LLM_MODE": "disabled"}
    r = _run(["runtime"], env=env)
    assert "is missing or malformed" not in r.stderr, r.stderr[-500:]
    assert "signer file unreadable" not in r.stderr and "service keys" not in r.stderr, r.stderr[-500:]
    assert r.returncode == 2 and "PULSO_LAB_BROKER_URL is empty" in r.stderr, (r.returncode, r.stderr[-500:])


@needs_image
def test_key_env_vars_are_scrubbed_and_files_are_0400_before_exec() -> None:
    env = _runtime_key_env() | {"PULSO_EXPORTER_KEY_CONTROL_API_SEED": _b64(b"\x0c" * 32),
                                "PULSO_EXPORTER_KEY_LAB_BROKER_SEED": _b64(b"\x0d" * 32)}
    fake = ('#!/bin/sh\n[ "$1" = "-m" ] || exec /usr/local/bin/python "$@"\n'
            'env | grep -c -e _JSON= -e _SEED= || true\nstat -c %%a:%%u /run/pulso-keys/*\n')
    probe = (f"mkdir /tmp/b && printf '{fake}' > /tmp/b/python && chmod +x /tmp/b/python && "
             "PATH=/tmp/b:$PATH exec /usr/local/bin/docker-entrypoint.sh runtime")
    r = _run(["-c", probe], entrypoint="sh", env=env)
    lines = r.stdout.split()
    assert r.returncode == 0 and lines[0] == "0", (r.stdout, r.stderr)
    assert len(lines) == 8, r.stdout  # 7 runtime key files
    assert all(line == "400:10001" for line in lines[1:]) and len(lines) > 1, r.stdout


@needs_image
def test_exporter_requires_both_key_seeds_and_never_echoes_them() -> None:
    r = _run(["exporter"], env={"PULSO_EXPORTER_KEY_CONTROL_API_SEED": _b64(b"\x0c" * 32)})
    assert r.returncode == 2 and "pulso:runtime_config_invalid" in r.stderr
    assert "PULSO_EXPORTER_KEY_LAB_BROKER_SEED" in r.stderr and _b64(b"\x0c" * 32) not in r.stderr
    ok = _run(["exporter"], env={"PULSO_EXPORTER_KEY_CONTROL_API_SEED": _b64(b"\x0c" * 32),
                                 "PULSO_EXPORTER_KEY_LAB_BROKER_SEED": _b64(b"\x0d" * 32)})
    assert "pulso:runtime_config_invalid" not in ok.stderr and "PULSO_EXPORTER_KEY_" not in ok.stderr, ok.stderr[-400:]


@needs_image
def test_migrate_needs_no_key_material() -> None:
    r = _run(["migrate", "--help"], env={})
    assert "pulso:runtime_config_invalid" not in r.stderr
