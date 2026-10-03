"""Deployment tenant allow-list on `/internal/v1`: a service JWT whose tenant claim is outside the configured set is
refused 403 `pulso:tenant_mismatch` on every tenant route, before any handler runs."""

from __future__ import annotations

import time
import uuid
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from pulso_core_runtime.internal.app import ROUTES, build_internal_app
from pulso_core_runtime.internal.auth import InMemoryJtiStore, ServiceJwtVerifier, ServiceKey, sign_service_jwt

pytestmark = pytest.mark.runtime
CP = Ed25519PrivateKey.generate()


def _client(allowed: Any, calls: list[str]) -> TestClient:
    verifier = ServiceJwtVerifier({"cp1": ServiceKey("control-api", "core-bridge", CP.public_key())}, InMemoryJtiStore())
    handlers = {f"{r.method} {r.path}": (lambda request, claims, p=r.path: calls.append(p) or {"ok": True})
                for r in ROUTES}
    return TestClient(build_internal_app(verifier, version_info=lambda: {}, handlers=handlers,
                                         allowed_tenants=allowed), raise_server_exceptions=False)


def _tok(purpose: str, tenant: str) -> dict[str, str]:
    now = int(time.time())
    return {"Authorization": "Bearer " + sign_service_jwt(CP, kid="cp1", claims={
        "iss": "control-api", "aud": "core-bridge", "sub": "worker:w", "tenant_id": tenant, "purpose": purpose,
        "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex})}


def _path(p: str) -> str:
    return p.replace("{task_id}", "x").replace("{arm_id}", "x").replace("{key}", "x")


@pytest.mark.parametrize("route", [r for r in ROUTES if r.path != "/version"], ids=lambda r: f"{r.method} {r.path}")
def test_foreign_tenant_is_403_and_never_reaches_the_handler(route: Any) -> None:
    calls: list[str] = []
    c = _client(frozenset({"t1"}), calls)
    purpose = next(iter(route.purposes))
    r = c.request(route.method, _path(route.path), headers=_tok(purpose, "t-other"))
    assert r.status_code == 403 and r.json()["code"] == "pulso:tenant_mismatch" and calls == []
    ok = c.request(route.method, _path(route.path), headers=_tok(purpose, "t1"))
    assert ok.status_code == 200 and calls == [route.path]


def test_allow_list_may_hold_several_tenants_and_none_means_unenforced() -> None:
    calls: list[str] = []
    c = _client(frozenset({"a", "b"}), calls)
    assert c.get("/core-tasks/x", headers=_tok("core_task_read", "b")).status_code == 200
    assert c.get("/core-tasks/x", headers=_tok("core_task_read", "c")).status_code == 403
    assert _client(None, calls).get("/core-tasks/x", headers=_tok("core_task_read", "zz")).status_code == 200


def test_allowed_tenants_come_from_env_and_are_required() -> None:
    from pulso_core_runtime.invoke.wiring import allowed_tenants_from_env
    assert allowed_tenants_from_env({"PULSO_TENANT_ID": "t1"}) == frozenset({"t1"})
    assert allowed_tenants_from_env({"PULSO_TENANT_ID": "t1", "PULSO_ALLOWED_TENANTS": "a, b,,"}) == frozenset(
        {"t1", "a", "b"})
    with pytest.raises(ValueError, match="PULSO_TENANT_ID"):
        allowed_tenants_from_env({"PULSO_TENANT_ID": " "})
