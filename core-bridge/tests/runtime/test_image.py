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
