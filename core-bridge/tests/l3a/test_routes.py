"""HTTP surface on the isolated `/internal/v1` app: auth, envelope, tenant isolation, read-only GET."""

from __future__ import annotations

import time
import uuid
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from pulso_core_runtime.credentials.issuer import CredentialIssuer, PrincipalSigner
from pulso_core_runtime.internal.app import build_internal_app
from pulso_core_runtime.internal.auth import InMemoryJtiStore, ServiceJwtVerifier, ServiceKey, sign_service_jwt
from pulso_core_runtime.invoke.routes import make_handlers
from pulso_core_runtime.store.migrations import apply_l3

from .conftest import PgDbs
from .helpers import body, build_service, idem_key

pytestmark = [pytest.mark.l3a, pytest.mark.pg]


@pytest.fixture
def world(pg: PgDbs) -> Any:
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    svc, core = build_service(pg.runtime)
    svc_key = Ed25519PrivateKey.generate()
    verifier = ServiceJwtVerifier({"cp1": ServiceKey("control-api", "core-bridge", svc_key.public_key())},
                                  InMemoryJtiStore())
    issuer = CredentialIssuer({"staff": PrincipalSigner("st1", Ed25519PrivateKey.generate()),
                               "identity": PrincipalSigner("id1", Ed25519PrivateKey.generate())})
    app = build_internal_app(verifier, version_info=lambda: {}, handlers=make_handlers(svc, issuer))

    def token(purpose: str, tenant: str = "t1") -> str:
        now = int(time.time())
        return sign_service_jwt(svc_key, kid="cp1", claims={
            "iss": "control-api", "aud": "core-bridge", "sub": "worker:1", "tenant_id": tenant, "purpose": purpose,
            "job_id": "j1", "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex})

    return TestClient(app), token, core


def test_invoke_then_read_over_http_with_tenant_isolation(world: Any) -> None:
    c, token, core = world
    k = idem_key("t1", "j1", "scout", 1, "k")
    h = {"Authorization": f"Bearer {token('core_task_invoke')}", "Idempotency-Key": k}
    r = c.post("/core-tasks/invoke", json=body(), headers=h)
    assert r.status_code == 200 and r.json()["state"] == "terminal_ok"
    run_id = r.json()["core_run_id"]
    # replay of the same JWT is rejected (jti) - the engine is never re-entered
    assert c.post("/core-tasks/invoke", json=body(), headers=h).status_code == 401
    ok = c.get(f"/core-tasks/{run_id}", headers={"Authorization": f"Bearer {token('core_task_read')}"})
    assert ok.status_code == 200 and ok.json()["core_run_id"] == run_id
    foreign = c.get(f"/core-tasks/{run_id}", headers={"Authorization": f"Bearer {token('core_task_read', 't2')}"})
    assert foreign.status_code == 404 and foreign.json()["code"] == "pulso:not_found"
    assert len(core.start_calls) == 1  # GET never starts anything


def test_wrong_purpose_digest_conflict_and_errors_use_the_envelope(world: Any) -> None:
    c, token, core = world
    k = idem_key("t1", "j1", "scout", 1, "k")
    assert c.post("/core-tasks/invoke", json=body(),
                  headers={"Authorization": f"Bearer {token('core_task_read')}", "Idempotency-Key": k}
                  ).status_code == 403
    first = c.post("/core-tasks/invoke", json=body(),
                   headers={"Authorization": f"Bearer {token('core_task_invoke')}", "Idempotency-Key": k})
    assert first.status_code == 200
    clash = c.post("/core-tasks/invoke", json=body(input={"q": "other"}),
                   headers={"Authorization": f"Bearer {token('core_task_invoke')}", "Idempotency-Key": k})
    assert clash.status_code == 409
    assert set(clash.json()) == {"schema_version", "code", "retryable", "trace_id", "details"}
    assert clash.json()["code"] == "pulso:digest_conflict" and len(core.start_calls) == 1
    bad = c.post("/core-tasks/invoke", content=b"[1]",
                 headers={"Authorization": f"Bearer {token('core_task_invoke')}", "Idempotency-Key": k})
    assert bad.status_code == 422


def test_credential_issue_route(world: Any) -> None:
    c, token, _ = world
    h = {"Authorization": f"Bearer {token('credential_issue')}"}
    r = c.post("/core-credentials/issue", json={"tenant_id": "t1", "role": "constructor", "purpose": "core_task"},
               headers=h)
    assert r.status_code == 200 and set(r.json()) == {"jws", "kid", "exp"} and r.headers["cache-control"] == "no-store"
    h = {"Authorization": f"Bearer {token('credential_issue')}"}
    denied = c.post("/core-credentials/issue", json={"tenant_id": "t1", "role": "aprobador", "purpose": "core_task"},
                    headers=h)
    assert denied.status_code == 403 and "jws" not in denied.text
    h = {"Authorization": f"Bearer {token('credential_issue')}"}
    cross = c.post("/core-credentials/issue", json={"tenant_id": "t9", "role": "constructor", "purpose": "core_task"},
                   headers=h)
    assert cross.status_code == 403 and cross.json()["code"] == "pulso:tenant_mismatch"
    h = {"Authorization": f"Bearer {token('credential_issue')}"}
    assert c.post("/core-credentials/issue", json={"tenant_id": "t1", "role": "constructor", "purpose": "x",
                                                    "extra": 1}, headers=h).status_code == 422
