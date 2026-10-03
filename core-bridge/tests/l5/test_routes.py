"""HTTP surface of the evaluation module mounted on the real L2 `/internal/v1` sub-app (auth + envelope)."""

from __future__ import annotations

import time
import uuid
from datetime import UTC, datetime, timedelta
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from l5.test_arms import FakeArtifacts, req
from l5.test_evaluate_path import FixedBudgets, World
from pulso_core_runtime.evaluation.arms import ArmRunner
from pulso_core_runtime.evaluation.report import PgArmStore
from pulso_core_runtime.evaluation.routes import EvaluationDeps, register
from pulso_core_runtime.evaluation.targets import TargetLoader
from pulso_core_runtime.internal.app import build_internal_app
from pulso_core_runtime.internal.auth import InMemoryJtiStore, ServiceJwtVerifier, ServiceKey, sign_service_jwt

pytestmark = [pytest.mark.runtime, pytest.mark.pg]
CP = Ed25519PrivateKey.generate()


def _token(purpose: str, **extra: Any) -> str:
    now = int(time.time())
    claims = {"iss": "control-api", "aud": "core-bridge", "sub": "worker:1", "tenant_id": "t1", "purpose": purpose,
              "job_id": "job-1", "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex, **extra}
    return sign_service_jwt(CP, kid="cp1", claims=claims)


def _client(w: World) -> TestClient:
    arms = ArmRunner(store=PgArmStore(w.pg.runtime), broker=w.broker, artifacts=FakeArtifacts(),
                     budgets=FixedBudgets(), loader=TargetLoader(w.store), composition=w.rt.composition,
                     gate=w.rt.evaluation_gate, sandbox=None)
    handlers: dict[str, Any] = {}
    register(handlers, EvaluationDeps(runtime=w.rt, arms=arms, broker=w.broker, budgets=FixedBudgets(),
                                      now=lambda: w.clock_now))
    keys = {"cp1": ServiceKey("control-api", "core-bridge", CP.public_key())}
    app = build_internal_app(ServiceJwtVerifier(keys, InMemoryJtiStore()), version_info=lambda: {},
                             handlers=handlers)
    return TestClient(app, raise_server_exceptions=False)


def _h(purpose: str) -> dict[str, str]:
    return {"Authorization": f"Bearer {_token(purpose)}"}


def _body(w: World, pid: str, chash: str, ref: str = "ctx-http-1", **over: Any) -> dict[str, Any]:
    return {"evaluation_context_ref": ref, "binding_ref": "bind-1", "proposal_id": pid, "candidate_hash": chash,
            "suite_id": "disputas-suite", "suite_version": "1.0.0",
            "suite_digest": w.rt._suite_digest(pid, "disputas-suite", "1.0.0"), "evaluation_attempt": 1,
            "budget_ref": "bud-1", "deadline": (datetime.now(UTC) + timedelta(hours=1)).isoformat(),
            "request_digest": "r" * 64, **over}


def test_admission_lifecycle_over_http_then_flow_evaluate(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    c = _client(w)
    r = c.post("/evaluation/admissions", json=_body(w, pid, chash), headers=_h("evaluation_admit"))
    assert r.status_code == 201 and r.json()["state"] == "admitted"
    again = c.post("/evaluation/admissions", json=_body(w, pid, chash), headers=_h("evaluation_admit"))
    assert again.status_code == 200
    other = c.post("/evaluation/admissions", json=_body(w, pid, chash, request_digest="z" * 64),
                   headers=_h("evaluation_admit"))
    assert other.status_code == 409 and other.json()["code"] == "pulso:idempotency_conflict"
    assert w.evaluate(pid, evaluation_context_ref="ctx-http-1").verdict == "pass"  # the admission is usable


@pytest.mark.parametrize("mutate,status,code", [
    ({"evaluation_context_ref": "bad ref"}, 422, "pulso:evaluation_context_invalid"),
    ({"evaluation_context_ref": "x" * 201}, 422, "pulso:evaluation_context_invalid"),
    ({"candidate_hash": "0" * 64}, 409, "pulso:candidate_changed"),
    ({"suite_digest": "f" * 64}, 409, "pulso:suite_mismatch"),
    ({"budget_ref": "bud-missing"}, 403, "pulso:budget_unknown"),
    ({"deadline": "2001-01-01T00:00:00+00:00"}, 409, "pulso:admission_expired"),
    ({"proposal_id": "p-ghost"}, 404, "pulso:proposal_not_found"),
    ({"evaluation_attempt": 0}, 422, "pulso:invalid_request"),
    ({"surprise": 1}, 422, "pulso:invalid_request"),
])
def test_admission_rejections(pg, mutate: dict[str, Any], status: int, code: str) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    r = _client(w).post("/evaluation/admissions", json=_body(w, pid, chash, **mutate),
                        headers=_h("evaluation_admit"))
    assert (r.status_code, r.json()["code"]) == (status, code)
    assert w.rt.admissions.get("ctx-http-1") is None  # nothing was persisted


def test_admission_broker_denied_and_wrong_purpose(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    c = _client(w)
    assert c.post("/evaluation/admissions", json=_body(w, pid, chash),
                  headers=_h("core_task_invoke")).status_code == 403  # purpose enforced by app.py
    w.broker.allow = False
    r = c.post("/evaluation/admissions", json=_body(w, pid, chash), headers=_h("evaluation_admit"))
    assert r.status_code == 403 and r.json()["code"] == "pulso:broker_denied"
    assert c.post("/evaluation/admissions", json=_body(w, pid, chash)).status_code == 401


def test_arm_run_and_reads_over_http(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    c = _client(w)
    r = c.post("/evaluation/arms/run/run", json=req(), headers=_h("evaluation_arm_run"))
    assert r.status_code == 200 and r.json()["status"] == "completed"
    eid = r.json()["execution_id"]
    got = c.get(f"/evaluation/arms/{eid}", headers=_h("evaluation_arm_read"))
    assert got.status_code == 200 and got.json() == r.json()
    assert c.get("/evaluation/arms/nope", headers=_h("evaluation_arm_read")).status_code == 404
    bad = c.post("/evaluation/arms/x/run", json={**req(), "gold": "x"}, headers=_h("evaluation_arm_run"))
    assert bad.status_code == 422 and bad.json()["code"] == "pulso:invalid_request"
