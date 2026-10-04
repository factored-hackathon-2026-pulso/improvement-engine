"""Annex D / V3 31.5 alignment (a): class (i) service tokens are worker tokens (`sub=worker:<id>`) and the `job_id`
claim must equal the body `job_id` on invoke. First RED of the Annex D alignment package."""

from __future__ import annotations

import time
import uuid
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from pulso_core_runtime.internal.app import build_internal_app
from pulso_core_runtime.internal.auth import InMemoryJtiStore, ServiceJwtVerifier, ServiceKey, sign_service_jwt
from pulso_core_runtime.invoke.routes import make_handlers
from pulso_core_runtime.invoke.service import InvokeOutcome

pytestmark = pytest.mark.runtime

CP = Ed25519PrivateKey.generate()


class StubService:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []

    async def invoke(self, tenant: str | None, key: str | None, raw: dict[str, Any]) -> InvokeOutcome:
        self.calls.append(raw)
        return InvokeOutcome(200, {"schema_version": "1", "state": "terminal_ok"})


def _client() -> tuple[TestClient, StubService]:
    stub = StubService()
    keys = {"cp1": ServiceKey("control-api", "core-bridge", CP.public_key())}
    app = build_internal_app(ServiceJwtVerifier(keys, InMemoryJtiStore()), version_info=lambda: {"ok": 1},
                             handlers=make_handlers(stub))  # type: ignore[arg-type]
    return TestClient(app, raise_server_exceptions=False), stub


def _token(purpose: str, *, drop: tuple[str, ...] = (), **extra: Any) -> str:
    now = int(time.time())
    claims = {"iss": "control-api", "aud": "core-bridge", "sub": "worker:1", "tenant_id": "t1", "purpose": purpose,
              "job_id": "j1", "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex, **extra}
    for k in drop:
        claims.pop(k)
    return sign_service_jwt(CP, kid="cp1", claims=claims)


def _bearer(token: str) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


@pytest.mark.parametrize("sub", ["bridge:1", "w", "worker", "worker:", "Worker:1", "user:worker:1"])
def test_a_non_worker_subject_is_403_auth_denied(sub: str) -> None:
    c, _ = _client()
    r = c.get("/version", headers=_bearer(_token("version_probe", sub=sub)))
    assert r.status_code == 403 and r.json()["code"] == "pulso:auth_denied"
    assert r.json()["details"] == {"reason": "sub_not_worker"}


def test_a_denied_subject_does_not_burn_its_jti() -> None:
    c, _ = _client()
    assert c.get("/version", headers=_bearer(_token("version_probe", sub="x", jti="same"))).status_code == 403
    assert c.get("/version", headers=_bearer(_token("version_probe", jti="same"))).status_code == 200


def test_invoke_token_job_must_equal_the_body_job() -> None:
    c, stub = _client()
    r = c.post("/core-tasks/invoke", json={"job_id": "j-other", "tenant_id": "t1"},
               headers={**_bearer(_token("core_task_invoke")), "Idempotency-Key": "k"})
    assert r.status_code == 403 and r.json()["code"] == "pulso:auth_denied"
    assert r.json()["details"] == {"reason": "job_mismatch"} and stub.calls == []


def test_invoke_without_a_job_claim_is_job_mismatch() -> None:
    c, stub = _client()
    r = c.post("/core-tasks/invoke", json={"job_id": "j1", "tenant_id": "t1"},
               headers={**_bearer(_token("core_task_invoke", drop=("job_id",))), "Idempotency-Key": "k"})
    assert r.status_code == 403 and r.json()["details"] == {"reason": "job_mismatch"} and stub.calls == []


def test_invoke_with_matching_job_reaches_the_service() -> None:
    c, stub = _client()
    r = c.post("/core-tasks/invoke", json={"job_id": "j1", "tenant_id": "t1"},
               headers={**_bearer(_token("core_task_invoke")), "Idempotency-Key": "k"})
    assert r.status_code == 200 and len(stub.calls) == 1
