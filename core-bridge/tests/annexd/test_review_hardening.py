"""Independent-review hardening of the Annex D alignment (claude-0020): no 500 on pathological JSON, duplicate
Idempotency-Key headers are refused, and the `|`-joined admission-ref derivation cannot be collided by a delimiter
inside a field. First RED of the review package. No PG needed: every case is refused before any store access."""

from __future__ import annotations

import time
import uuid
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from pulso_core_runtime.evaluation.admission import derive_context_ref
from pulso_core_runtime.evaluation.routes import EvaluationDeps, register
from pulso_core_runtime.internal.app import build_internal_app
from pulso_core_runtime.internal.auth import InMemoryJtiStore, ServiceJwtVerifier, ServiceKey, sign_service_jwt
from pulso_core_runtime.invoke.routes import make_handlers
from pulso_core_runtime.invoke.service import InvokeOutcome

pytestmark = pytest.mark.runtime
CP = Ed25519PrivateKey.generate()


class Arms:
    calls = 0

    def run(self, raw: Any, tenant_id: str) -> Any:
        Arms.calls += 1
        raise AssertionError("arms must not run")


class Svc:
    calls = 0

    async def invoke(self, tenant: Any, key: Any, raw: Any) -> InvokeOutcome:
        Svc.calls += 1
        return InvokeOutcome(200, {})


def _client() -> TestClient:
    handlers: dict[str, Any] = {}
    register(handlers, EvaluationDeps(runtime=None, arms=Arms(), broker=None, budgets=None))  # type: ignore[arg-type]
    handlers.update(make_handlers(Svc()))  # type: ignore[arg-type]
    keys = {"cp1": ServiceKey("control-api", "core-bridge", CP.public_key())}
    app = build_internal_app(ServiceJwtVerifier(keys, InMemoryJtiStore()), version_info=lambda: {}, handlers=handlers)
    return TestClient(app, raise_server_exceptions=False)


def _tok(purpose: str, **extra: Any) -> str:
    now = int(time.time())
    claims = {"iss": "control-api", "aud": "core-bridge", "sub": "worker:1", "tenant_id": "t1", "purpose": purpose,
              "job_id": "j1", "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex, **extra}
    return sign_service_jwt(CP, kid="cp1", claims=claims)


ROUTES = [("/evaluation/admissions", "evaluation_admit"), ("/evaluation/arms/run", "evaluation_arm_run"),
          ("/core-tasks/invoke", "core_task_invoke")]


@pytest.mark.parametrize(("path", "purpose"), ROUTES)
def test_pathologically_nested_json_is_a_422_not_a_500(path: str, purpose: str) -> None:
    deep = b"[" * 100_000 + b"]" * 100_000
    r = _client().post(path, content=deep, headers={"Authorization": f"Bearer {_tok(purpose)}",
                                                    "Content-Type": "application/json", "Idempotency-Key": "k"})
    assert r.status_code == 422 and r.json()["code"] == "pulso:invalid_request"


@pytest.mark.parametrize(("path", "purpose"), ROUTES)
def test_a_duplicated_idempotency_key_header_is_refused(path: str, purpose: str) -> None:
    r = _client().post(path, json={"job_id": "j1"}, headers=[
        ("Authorization", f"Bearer {_tok(purpose)}"), ("Idempotency-Key", "a"), ("Idempotency-Key", "b")])
    assert r.status_code == 422 and r.json()["details"] == {"fields": ["Idempotency-Key"]}
    assert Arms.calls == 0 and Svc.calls == 0


def test_derivation_refuses_the_delimiter_inside_a_field() -> None:
    # ("a|b","c") and ("a","b|c") would hash the same joined string.
    with pytest.raises(ValueError):
        derive_context_ref("t1", "j1", "a|b", "c", "h", 1)
    with pytest.raises(ValueError):
        derive_context_ref("t1", "j1", "a", "b|c", "h", 1)
    with pytest.raises(ValueError):
        derive_context_ref("t|1", "j1", "a", "b", "h", 1)


def test_the_admission_route_turns_a_delimiter_field_into_422() -> None:
    body = {"binding_ref": "a|b", "proposal_id": "c", "candidate_hash": "h", "suite_id": "s", "suite_version": "1",
            "suite_digest": "d", "evaluation_attempt": 1, "budget_ref": "b",
            "deadline": "2030-01-01T00:00:00Z", "request_digest": "a" * 64}
    r = _client().post("/evaluation/admissions", json=body,
                       headers={"Authorization": f"Bearer {_tok('evaluation_admit')}"})
    assert r.status_code == 422 and r.json()["code"] == "pulso:invalid_request"
