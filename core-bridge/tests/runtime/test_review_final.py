"""Final independent review: defects found by adversarial reading of the integrated head (each test failed first)."""

from __future__ import annotations

import time
import uuid
from datetime import UTC, datetime, timedelta
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient
from pulso_core_runtime.internal.app import build_internal_app
from pulso_core_runtime.internal.auth import (
    InMemoryJtiStore,
    ServiceJwtVerifier,
    ServiceKey,
    sign_service_jwt,
)
from pulso_core_runtime.tools.context import InvocationContext, InvocationRegistry

pytestmark = pytest.mark.runtime
CP = Ed25519PrivateKey.generate()


def _client(handlers: Any = None) -> TestClient:
    keys = {"cp1": ServiceKey("control-api", "core-bridge", CP.public_key())}
    app = build_internal_app(ServiceJwtVerifier(keys, InMemoryJtiStore()), version_info=lambda: {"ok": 1},
                             handlers=handlers)
    return TestClient(app, raise_server_exceptions=False)


def _tok(purpose: str, **over: Any) -> str:
    now = int(time.time())
    claims: dict[str, Any] = {"iss": "control-api", "aud": "core-bridge", "sub": "worker:1", "tenant_id": "t1",
                              "purpose": purpose, "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex, **over}
    claims = {k: v for k, v in claims.items() if v is not ...}
    return sign_service_jwt(CP, kid="cp1", claims=claims)


def _h(token: str) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


def test_tenantless_token_is_rejected_on_tenant_routes_but_version_probe_is_exempt() -> None:
    c = _client({"POST /evaluation/arms/run": lambda r, cl: {"tenant": cl.tenant_id}})
    r = c.post("/evaluation/arms/run", json={}, headers=_h(_tok("evaluation_arm_run", tenant_id=...)))
    assert r.status_code == 403 and r.json()["details"]["reason"] == "tenant_required"
    r = c.post("/evaluation/arms/run", json={}, headers=_h(_tok("evaluation_arm_run", tenant_id="")))
    assert r.status_code == 403
    assert c.get("/version", headers=_h(_tok("version_probe", tenant_id=...))).status_code == 200


@pytest.mark.parametrize("claim", ["exp", "iat"])
def test_non_finite_and_missing_time_claims_are_rejected(claim: str) -> None:
    c = _client()
    now = int(time.time())
    bad = {"exp": now + 60, "iat": now}
    bad[claim] = float("nan")  # JSON `NaN` passes naive comparisons
    assert c.get("/version", headers=_h(_tok("version_probe", **bad))).status_code == 401
    missing = _tok("version_probe", **{claim: ...})
    assert c.get("/version", headers=_h(missing)).status_code == 401


def test_oversized_request_body_is_refused_before_parsing() -> None:
    seen: list[int] = []
    c = _client({"POST /core-tasks/invoke": lambda r, cl: seen.append(1) or {"ok": 1}})
    big = b'{"x":"' + b"a" * (2 * 1024 * 1024) + b'"}'
    r = c.post("/core-tasks/invoke", content=big, headers={**_h(_tok("core_task_invoke")),
                                                           "content-type": "application/json"})
    assert r.status_code == 413 and r.json()["code"] == "pulso:payload_too_large" and not seen
    chunked = c.post("/core-tasks/invoke", content=(b"a" * 65536 for _ in range(40)),
                     headers=_h(_tok("core_task_invoke")))
    assert chunked.status_code == 413 and not seen
    ok = c.post("/core-tasks/invoke", json={"a": 1}, headers=_h(_tok("core_task_invoke")))
    assert ok.status_code == 200 and seen


def _ic(ref: str, expires: datetime) -> InvocationContext:
    return InvocationContext(
        tenant_id="t", job_id="j", stage="scout", attempt=1, binding_ref=ref, command_key="c", request_digest="r",
        bridge_instance_id="b", expires_at=expires)


def test_registry_drops_expired_contexts_instead_of_leaking_them() -> None:
    now = [datetime(2026, 1, 1, tzinfo=UTC)]
    reg = InvocationRegistry(clock=lambda: now[0])
    for n in range(50):
        reg.register(_ic(f"old-{n}", now[0] + timedelta(minutes=1)))
    now[0] += timedelta(minutes=5)
    reg.register(_ic("fresh", now[0] + timedelta(minutes=5)))
    assert len(reg._entries) == 1 and reg.lookup("fresh").binding_ref == "fresh"
    reg.register(_ic("old-0", now[0] + timedelta(minutes=5)))  # an expired ref can be registered again
