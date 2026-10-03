"""Image contract (PL-0008): static Dockerfile/entrypoint checks always; container checks when PULSO_TEST_IMAGE is set.

Podman only (machine pulso-dev). OCI builds drop HEALTHCHECK; run with --cgroups=disabled and a read-only root fs."""

from __future__ import annotations

import base64
import os
import re
import subprocess
import uuid
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
DOCKERFILE = (ROOT / "Dockerfile").read_text() if (ROOT / "Dockerfile").exists() else ""
ENTRYPOINT = (ROOT / "docker-entrypoint.sh").read_text() if (ROOT / "docker-entrypoint.sh").exists() else ""
PODMAN = os.environ.get("PULSO_PODMAN", r"C:\Users\alexg\AppData\Local\Programs\Podman\podman.exe")
CONN = os.environ.get("PULSO_PODMAN_CONNECTION", "pulso-dev")
SEED_VAR = "PULSO_EXPORTER_KEY_CONTROL_API_SEED"
STATE_DIR = "/var/lib/pulso-platform-exporter"
needs_image = pytest.mark.skipif("PULSO_TEST_IMAGE" not in os.environ, reason="PULSO_TEST_IMAGE not set")


def _b64(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).decode().rstrip("=")


SEED = _b64(b"\x0c" * 32)


def _run(cmd: list[str], *, entrypoint: str | None = None, env: dict[str, str] | None = None,
         mounts: list[str] | None = None) -> subprocess.CompletedProcess[str]:
    argv = [PODMAN, "--connection", CONN, "run", "--rm", "--cgroups=disabled", "--read-only", "-u", "10001"]
    if entrypoint:
        argv += ["--entrypoint", entrypoint]
    for k, v in (env or {}).items():
        argv += ["-e", f"{k}={v}"]
    for m in mounts or []:
        argv += ["-v", m]
    return subprocess.run([*argv, os.environ["PULSO_TEST_IMAGE"], *cmd], capture_output=True, text=True, timeout=120)


def _volume() -> str:
    vol = f"pulso-test-pl-state-{uuid.uuid4().hex[:8]}"
    subprocess.run([PODMAN, "--connection", CONN, "volume", "create", vol], check=True, capture_output=True)
    return vol


def _drop(vol: str) -> None:
    subprocess.run([PODMAN, "--connection", CONN, "volume", "rm", "-f", vol], capture_output=True)


# ---- static ----


def test_dockerfile_exists_and_is_multistage_nonroot_uv_locked() -> None:
    assert DOCKERFILE, "platform-exporter/Dockerfile is missing"
    assert len(re.findall(r"^FROM ", DOCKERFILE, re.M)) == 2
    assert "python:3.12-slim" in DOCKERFILE
    assert re.search(r"^USER 10001", DOCKERFILE, re.M)
    assert "uv export --frozen" in DOCKERFILE and "--no-dev" in DOCKERFILE and "uv.lock" in DOCKERFILE
    for forbidden in ("COPY tests", "--group dev", "platform-sim", "HEALTHCHECK"):
        assert forbidden not in DOCKERFILE


def test_dockerfile_precreates_keys_and_state_dirs_owned_by_the_app_uid() -> None:
    assert re.search(r"install -d -m 0700 -o 10001 .*/run/pulso-keys", DOCKERFILE)
    assert re.search(rf"install -d -m 0700 -o 10001 .*{STATE_DIR}", DOCKERFILE)
    assert f"EXPORTER_STATE_PATH={STATE_DIR}/state.sqlite" in DOCKERFILE
    assert "docker-entrypoint.sh" in DOCKERFILE and "ENTRYPOINT" in DOCKERFILE


def test_dockerignore_keeps_secrets_tests_and_vcs_out_of_the_context() -> None:
    ignore = (ROOT / ".dockerignore").read_text().split()
    for entry in (".git", "tests", "docs", ".venv", "__pycache__", ".env*", "*.sqlite"):
        assert entry in ignore, entry


def test_entrypoint_is_fail_closed_scrubs_and_matches_the_core_bridge_pattern() -> None:
    assert ENTRYPOINT.startswith("#!/bin/sh") and "set -eu" in ENTRYPOINT
    assert SEED_VAR in ENTRYPOINT and "unset" in ENTRYPOINT and "/run/pulso-keys" in ENTRYPOINT
    assert "pulso:runtime_config_invalid" in ENTRYPOINT and "sys.exit(2)" in ENTRYPOINT
    assert "exec python -m platform_exporter" in ENTRYPOINT
    assert "0o400" in ENTRYPOINT and "0o700" in ENTRYPOINT


def test_names_the_infra_module_injects_are_known_to_the_image() -> None:
    main = (ROOT / "src/platform_exporter/__main__.py").read_text()
    for name in ("PLATFORM_DB_URL", "PULSO_CONTROL_API_URL", "EXPORTER_STATE_PATH", "PULSO_BINDING_REF"):
        assert name in main, name
    assert SEED_VAR in ENTRYPOINT


def test_entrypoint_materialises_the_optional_lab_broker_seed_but_only_the_control_api_key_is_required() -> None:
    assert "PULSO_EXPORTER_KEY_LAB_BROKER_SEED" in ENTRYPOINT and "exporter-lab-broker.key" in ENTRYPOINT
    assert "optional" in ENTRYPOINT.lower()


# ---- container ----


@needs_image
def test_image_runs_as_uid_10001_with_pre_created_dirs() -> None:
    r = _run(["-c", f"id -u; stat -c %a:%u /run/pulso-keys {STATE_DIR}"], entrypoint="sh")
    assert r.returncode == 0 and r.stdout.split() == ["10001", "700:10001", "700:10001"], (r.stdout, r.stderr[-300:])


@needs_image
def test_state_dir_is_writable_on_an_ephemeral_volume_under_a_read_only_root() -> None:
    vol = _volume()
    try:
        code = f"from platform_exporter import ExporterState; ExporterState('{STATE_DIR}/state.sqlite'); print('wrote')"
        r = _run(["-c", code], entrypoint="python", mounts=[f"{vol}:{STATE_DIR}"])
        assert r.returncode == 0 and "wrote" in r.stdout, r.stderr[-500:]
    finally:
        _drop(vol)


@needs_image
def test_installed_set_has_no_test_or_sim_code() -> None:
    code = "import importlib.util as u; print([m for m in ('tests','platform_live','fastapi','pytest') if u.find_spec(m)])"
    r = _run(["-c", code], entrypoint="python")
    assert r.returncode == 0 and r.stdout.strip() == "[]", (r.stdout, r.stderr[-300:])


@needs_image
def test_missing_seed_fails_closed_naming_the_variable_not_a_value() -> None:
    r = _run([], env={})
    assert r.returncode == 2 and "pulso:runtime_config_invalid" in r.stderr and SEED_VAR in r.stderr, r.stderr[-400:]


@needs_image
def test_malformed_seed_fails_closed_without_echoing_it() -> None:
    r = _run([], env={SEED_VAR: _b64(b"short")})
    assert r.returncode == 2 and SEED_VAR in r.stderr and _b64(b"short") not in r.stderr, r.stderr[-400:]


@needs_image
def test_key_file_is_0400_owned_by_the_app_uid_and_env_is_scrubbed_before_exec() -> None:
    fake = ('#!/bin/sh\n[ "$1" = "-m" ] || exec /usr/local/bin/python "$@"\n'
            'env | grep -c -e _SEED= || true\nstat -c %%a:%%u /run/pulso-keys/*\nenv | grep PULSO_EXPORTER_KEY_CONTROL_API=\n')
    probe = (f"mkdir /tmp/b && printf '{fake}' > /tmp/b/python && chmod +x /tmp/b/python && "
             "PATH=/tmp/b:$PATH exec /usr/local/bin/docker-entrypoint.sh")
    r = _run(["-c", probe], entrypoint="sh", env={SEED_VAR: SEED})
    lines = r.stdout.split()
    assert r.returncode == 0, (r.stdout, r.stderr)
    assert lines[0] == "0" and lines[1] == "400:10001", r.stdout
    assert lines[2] == "PULSO_EXPORTER_KEY_CONTROL_API=/run/pulso-keys/exporter-control-api.key", r.stdout
    assert SEED not in r.stdout and SEED not in r.stderr


@needs_image
def test_real_entrypoint_gets_past_key_materialisation_and_into_the_exporter() -> None:
    vol = _volume()
    try:
        env = {SEED_VAR: SEED, "PLATFORM_DB_URL": "/nonexistent/platform.sqlite",
               "PULSO_CONTROL_API_URL": "http://127.0.0.1:9", "PULSO_TENANT_ID": "t1", "PLATFORM_INSTANCE": "plat",
               "PULSO_BINDING_REF": "b1"}
        r = _run([], env=env, mounts=[f"{vol}:{STATE_DIR}"])
        assert "pulso:runtime_config_invalid" not in r.stderr, r.stderr[-400:]
        assert "PermissionError" not in r.stderr and SEED not in r.stdout + r.stderr
        assert r.returncode != 0, r.stderr[-400:]
    finally:
        _drop(vol)



def _probe_script(extra_lines: str) -> str:
    fake = ('#!/bin/sh\n[ "$1" = "-m" ] || exec /usr/local/bin/python "$@"\n'
            'env | grep -c -e _SEED= || true\n' + extra_lines)
    return (f"mkdir /tmp/b && printf '{fake}' > /tmp/b/python && chmod +x /tmp/b/python && "
            "PATH=/tmp/b:$PATH exec /usr/local/bin/docker-entrypoint.sh")


@needs_image
def test_optional_lab_broker_seed_is_materialised_scrubbed_and_validated() -> None:
    lab = "PULSO_EXPORTER_KEY_LAB_BROKER_SEED"
    seed2 = _b64(b"\x0d" * 32)
    probe = _probe_script("stat -c %%a /run/pulso-keys/exporter-lab-broker.key\n"
                          "env | grep PULSO_EXPORTER_KEY_LAB_BROKER=\n")
    r = _run(["-c", probe], entrypoint="sh", env={SEED_VAR: SEED, lab: seed2})
    assert r.returncode == 0, (r.stdout, r.stderr)
    assert r.stdout.split() == ["0", "400", "PULSO_EXPORTER_KEY_LAB_BROKER=/run/pulso-keys/exporter-lab-broker.key"]
    assert seed2 not in r.stdout + r.stderr
    bad = _run([], env={SEED_VAR: SEED, lab: _b64(b"short")})
    assert bad.returncode == 2 and lab in bad.stderr and _b64(b"short") not in bad.stderr, bad.stderr[-300:]
