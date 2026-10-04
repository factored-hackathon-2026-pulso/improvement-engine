"""Start the system under test: the mock, a2 (real RegistryService in the pinned venv) or an external real server."""

from __future__ import annotations

import contextlib
import os
import socket
import subprocess
import sys
import time
from collections.abc import Iterator
from pathlib import Path

import httpx

PLATFORM_SIM = Path(__file__).resolve().parents[2]
DEFAULT_CHECKOUT = Path(os.environ.get("PULSO_CORE_CHECKOUT", r"D:\.codex\factored\references\agent-core-c814c2b"))
DEFAULT_CORE_PY = Path(os.environ.get("TEMP", ".")) / "pulso-wire-venv-c814c2b" / "Scripts" / "python.exe"


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


@contextlib.contextmanager
def serve(target: str, extra_env: dict[str, str] | None = None, platform_sim: Path | None = None) -> Iterator[str]:
    """Yield the base URL for TARGET in {mock, a2}. `real` needs REGISTRY_BASE_URL (handled by the caller)."""
    port = free_port()
    env = {**os.environ, "PYTHONPATH": str(platform_sim or PLATFORM_SIM), "PYTHONIOENCODING": "utf-8", **(extra_env or {})}
    if target == "mock":
        cmd = [sys.executable, "-m", "registry_mock.main", "--port", str(port)]
    elif target == "a2":
        py = Path(os.environ.get("PULSO_CORE_PYTHON", str(DEFAULT_CORE_PY)))
        if not py.exists():
            raise RuntimeError(f"a2 needs the pinned venv python ({py}); run core-bridge/scripts/gen-wire.ps1 first")
        env["PULSO_CORE_CHECKOUT"] = str(DEFAULT_CHECKOUT)
        cmd = [str(py), "-W", "ignore", "-m", "registry_mock.a2_app", "--port", str(port)]
    else:
        raise ValueError(target)
    proc = subprocess.Popen(cmd, env=env, cwd=str(platform_sim or PLATFORM_SIM), stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    base = f"http://127.0.0.1:{port}"
    try:
        deadline = time.time() + 60
        while time.time() < deadline:
            if proc.poll() is not None:
                raise RuntimeError(f"{target} server exited: {proc.stderr.read().decode(errors='replace')[-2000:]}")
            try:
                if httpx.get(f"{base}/_sim/info", timeout=1).status_code == 200:
                    break
            except httpx.TransportError:
                time.sleep(0.2)
        else:
            raise RuntimeError(f"{target} server did not start")
        yield base
    finally:
        proc.terminate()
        try:
            proc.wait(5)
        except subprocess.TimeoutExpired:
            proc.kill()
        if proc.stderr:
            proc.stderr.close()
