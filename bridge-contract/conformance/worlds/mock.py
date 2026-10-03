"""World: platform-sim `bridge_mock` (label `contract_mock`), spawned as a real HTTP process (or CONTRACT_BASE_URL).

The mock predates annex D/A03 and the runtime's final DTOs; this world adapts ONLY what is needed to talk to it
(its key material, its clock, its purpose names, its invocation DTO). Everything else the suite asserts is the
contract, so each real difference surfaces as a known-different case in `known_different.py`."""

from __future__ import annotations

import contextlib
import hashlib
import os
import socket
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, ClassVar

import httpx

from conformance.kit import Api, Signer, jcs, sha256_hex
from conformance.worlds import World

PLATFORM_SIM = Path(__file__).resolve().parents[3] / "platform-sim"
PURPOSES = {"core_task_invoke": "task_invoke", "core_task_read": "task_read", "alias_read": "state_read",
            "authoring_dry_run": "authoring_dry_run", "version_probe": "version_read",
            "credential_issue": "credential_issue"}
SIM_EPOCH_TS = 1767225600.0  # registry_mock.jws.SIM_EPOCH (2026-01-01T00:00:00Z); the mock verifies against it


class MockWorld(World):
    target = "mock"
    doubles: ClassVar[list[str]] = ["platform-sim bridge_mock (runtime_profile=contract_mock; no Core, no registry, in-memory state)"]
    caps: ClassVar[set[str]] = {"invoke", "credentials"}

    def __init__(self, base_url: str | None) -> None:
        self._proc: subprocess.Popen[bytes] | None = None
        if not base_url:
            with socket.socket() as s:
                s.bind(("127.0.0.1", 0))
                port = int(s.getsockname()[1])
            env = {**os.environ, "PYTHONPATH": str(PLATFORM_SIM), "PYTHONIOENCODING": "utf-8"}
            self._proc = subprocess.Popen([sys.executable, "-m", "bridge_mock.main", "--port", str(port)], env=env,
                                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            base_url = f"http://127.0.0.1:{port}"
            deadline = time.time() + 30
            while time.time() < deadline:
                with contextlib.suppress(httpx.TransportError):
                    if httpx.get(f"{base_url}/_sim/info", timeout=1).status_code == 200:
                        break
                time.sleep(0.2)
            else:
                raise RuntimeError("bridge mock did not start")
        self.base_url = base_url
        seed = hashlib.sha256(b"pulso bridge-sim TEST ONLY key: control-api").digest()  # public label, sim-only
        self.signer = Signer("sim-control-api-1", seed, now=lambda: SIM_EPOCH_TS, purpose_map=PURPOSES)
        self.scout_release = "rel-mock-0001"
        self.writer_release = "rel-mock-0001"
        self.api = Api(base_url, self.signer, self.tenant)

    def now(self) -> float:
        return SIM_EPOCH_TS

    def invocation(self, stage: str, job: str, logical: str, *, attempt: int = 1, **over: Any) -> dict[str, Any]:
        body: dict[str, Any] = {
            "schema_version": "1", "tenant_id": self.tenant, "job_id": job, "stage": stage, "attempt": attempt,
            "agent_id": f"pulso-{stage.replace('_', '-')}", "agent_version": self.agent_version,
            "release_id": self.scout_release, "input": {"signal": "x"}, "input_refs": [], "config_ref": "cfg-1",
            "lab_grant_ref": "grant-1", "memory_snapshot_ref": None, "information_partition": "train",
            "cutoff": "2026-01-01T00:00:00Z", "deadline": "2026-01-01T01:00:00Z", "budget_ref": "bud-1"}
        for k, v in over.items():
            if k in body:  # keys the mock DTO does not have (logical_key, extract_manifest_ref...) are not sent
                body[k] = v
        body["request_digest"] = sha256_hex(jcs({k: v for k, v in body.items() if k != "request_digest"}))
        return body

    def invoke(self, body: dict[str, Any], job: str | None = None, *, key: str | None = None, **kw: Any) -> httpx.Response:
        job = job or body["job_id"]
        key = key or f"{body['tenant_id']}:{body['job_id']}:{body['stage']}:{body['attempt']}"
        kw.pop("with_digest", None)
        purpose = kw.pop("purpose", "core_task_invoke")
        return self.api.call("POST", "/core-tasks/invoke", purpose=purpose, body=body,
                             headers={"Idempotency-Key": key, **kw.pop("headers", {})}, job_id=job, **kw)

    def close(self) -> None:
        self.api.close()
        if self._proc is not None:
            self._proc.terminate()
            with contextlib.suppress(subprocess.TimeoutExpired):
                self._proc.wait(5)


def start(base_url: str | None) -> MockWorld:
    return MockWorld(base_url)
