"""Doubles for L3a tests. `FakeCore` imitates Core's `POST /v1/runs` idempotency semantics (principal key + key,
body hash) and can fault-inject a timeout AFTER the effect; everything else here is a recorded double."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from typing import Any

import httpx
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from pulso_core_runtime.invoke.core_client import CoreResponse


def verifier_keys() -> tuple[Ed25519PrivateKey, str]:
    return Ed25519PrivateKey.generate(), "id1"


@dataclass
class FakeRunState:
    run_id: str
    status: str = "done"
    outcome: str = "completed"
    facts: dict[str, Any] = field(default_factory=dict)


class FakeCore:
    """Records `start_run` calls. `mode`: ok | timeout_after_effect | timeout_before_effect | http_500."""

    def __init__(self, release_id: str = "rel-1") -> None:
        self.release_id, self.mode = release_id, "ok"
        self.start_calls: list[dict[str, Any]] = []
        self.stored: dict[tuple[str, str], tuple[str, dict[str, Any]]] = {}
        self.runs: dict[str, FakeRunState] = {}
        self.run_facts: dict[str, Any] = {}
        self.release_override: str | None = None
        self._n = 0
        self.binder: Any = None  # simulates `pulso/bind_context` confirming the binding inside the run

    async def start_run(self, bearer: str, key: str, body: dict[str, Any]) -> CoreResponse:
        principal_id = _principal_id(bearer)
        self.start_calls.append({"key": key, "body": body, "principal": principal_id})
        h = hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()
        prior = self.stored.get((principal_id, key))
        if prior is not None:
            if prior[0] != h:
                return CoreResponse(409, {"code": "idempotency_conflict"})
            return CoreResponse(201, prior[1])
        if self.mode == "timeout_before_effect":
            raise httpx.ReadTimeout("t")
        if self.mode == "http_500":
            return CoreResponse(500, {})
        self._n += 1
        run_id = f"run-{self._n}"
        result = {"run_id": run_id, "release": self.release_override or self.release_id, "output": None,
                  "status": "done", "outcome": "completed", "handoff_ref": None, "trace_id": "t" * 32}
        self.stored[(principal_id, key)] = (h, result)
        self.runs[run_id] = FakeRunState(run_id, facts=dict(self.run_facts))
        if self.binder is not None:
            self.binder(run_id)
        if self.mode == "timeout_after_effect":
            raise httpx.ReadTimeout("t")
        return CoreResponse(201, result)

    # RunReader port
    def load_run(self, run_id: str) -> FakeRunState | None:
        return self.runs.get(run_id)

    def get_run_idempotency(self, principal_id: str, key: str) -> tuple[str, dict[str, Any]] | None:
        return self.stored.get((principal_id, key))


def _principal_id(bearer: str) -> str:
    import base64
    body = bearer.split(".")[1]
    return json.loads(base64.urlsafe_b64decode(body + "=" * (-len(body) % 4)))["id"]


@dataclass
class FakeReleases:
    status: str = "active"
    agent_id: str = "pulso-scout"
    version: str = "1.0.0"
    closure: str = "c" * 64
    release_id: str = "rel-1"
    exists: bool = True

    def check(self, release_id: str, agent_id: str, agent_version: str, closure_digest: str | None) -> str | None:
        if not self.exists or release_id != self.release_id:
            return "pulso:release_pin_unavailable"
        if self.status == "revoked":
            return "pulso:release_revoked"
        if self.status != "active" or agent_id != self.agent_id or agent_version != self.version:
            return "pulso:release_pin_unavailable"
        if closure_digest is not None and closure_digest != self.closure:
            return "pulso:release_pin_unavailable"
        return None


def now() -> datetime:
    return datetime.now(UTC)


def later(minutes: int) -> datetime:
    return now() + timedelta(minutes=minutes)
