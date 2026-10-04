"""Annex D.4 (b)(d)(f) for admissions: the evaluation_context_ref is derived server-side from the admission binding
(an explicit value is accepted only when equal), the Idempotency-Key header is accepted, deadline/request_digest
formats are enforced. Real PG16."""

from __future__ import annotations

import hashlib
from datetime import UTC, datetime, timedelta
from typing import Any

import pytest

from l5.test_evaluate_path import World
from l5.test_routes import _client, _h
from pulso_core_runtime.evaluation.admission import derive_context_ref

pytestmark = [pytest.mark.runtime, pytest.mark.pg]

TENANT, JOB, BINDING = "t1", "job-1", "bind-1"  # tenant/job come from the l5 test token


def expected_ref(pid: str, chash: str, attempt: int = 1, job: str = JOB) -> str:
    raw = "|".join([TENANT, job, BINDING, pid, chash, str(attempt)])
    return "evc-" + hashlib.sha256(raw.encode()).hexdigest()[:40]


def body(w: World, pid: str, chash: str, **over: Any) -> dict[str, Any]:
    """Annex D.4 admission body: NO evaluation_context_ref, Z deadline, hex request_digest."""
    b = {"schema_version": "1", "binding_ref": BINDING, "proposal_id": pid, "candidate_hash": chash,
         "suite_id": "disputas-suite", "suite_version": "1.0.0",
         "suite_digest": w.rt._suite_digest(pid, "disputas-suite", "1.0.0"), "evaluation_attempt": 1,
         "budget_ref": "bud-1",
         "deadline": (datetime.now(UTC) + timedelta(hours=1)).strftime("%Y-%m-%dT%H:%M:%SZ"),
         "request_digest": "a" * 64}
    return {**b, **over}


def test_derivation_formula_is_the_documented_one() -> None:
    assert derive_context_ref(TENANT, JOB, BINDING, "p", "c", 2) == expected_ref("p", "c", 2)


def test_server_derives_the_ref_and_the_admission_is_usable(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    r = _client(w).post("/evaluation/admissions", json=body(w, pid, chash), headers=_h("evaluation_admit"))
    assert r.status_code == 201, r.text
    ref = expected_ref(pid, chash)
    assert r.json() == {"schema_version": "1", "evaluation_context_ref": ref, "state": "admitted"}
    assert w.evaluate(pid, evaluation_context_ref=ref).verdict == "pass"


def test_an_explicit_ref_is_accepted_only_when_it_equals_the_derived_one(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    c = _client(w)
    ref = expected_ref(pid, chash)
    assert c.post("/evaluation/admissions", json=body(w, pid, chash, evaluation_context_ref=ref),
                  headers=_h("evaluation_admit")).status_code == 201
    again = c.post("/evaluation/admissions", json=body(w, pid, chash), headers=_h("evaluation_admit"))
    assert again.status_code == 200 and again.json()["evaluation_context_ref"] == ref  # same admission
    bad = c.post("/evaluation/admissions", json=body(w, pid, chash, evaluation_context_ref="client-chosen-1"),
                 headers=_h("evaluation_admit"))
    assert bad.status_code == 422 and bad.json()["code"] == "pulso:evaluation_context_invalid"
    assert w.rt.admissions.get("client-chosen-1") is None


def test_the_ref_depends_on_the_attempt_and_the_job(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    c = _client(w)
    one = c.post("/evaluation/admissions", json=body(w, pid, chash), headers=_h("evaluation_admit"))
    two = c.post("/evaluation/admissions", json=body(w, pid, chash, evaluation_attempt=2),
                 headers=_h("evaluation_admit"))
    assert one.json()["evaluation_context_ref"] != two.json()["evaluation_context_ref"]
    assert two.json()["evaluation_context_ref"] == expected_ref(pid, chash, 2)


def test_the_idempotency_key_header_is_accepted_and_validated(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    c = _client(w)
    ok = c.post("/evaluation/admissions", json=body(w, pid, chash),
                headers={**_h("evaluation_admit"), "Idempotency-Key": "adm-key-1"})
    assert ok.status_code == 201
    bad = c.post("/evaluation/admissions", json=body(w, pid, chash),
                 headers={**_h("evaluation_admit"), "Idempotency-Key": "has space"})
    assert bad.status_code == 422 and bad.json()["details"] == {"fields": ["Idempotency-Key"]}


@pytest.mark.parametrize("over", [
    {"deadline": "2030-01-01T00:00:00+00:00"}, {"deadline": "2030-01-01T00:00:00"}, {"deadline": "2030-01-01"},
    {"request_digest": "A" * 64}, {"request_digest": "a" * 63}, {"request_digest": "r" * 64},
])
def test_deadline_and_request_digest_formats_are_enforced(pg, over: dict[str, Any]) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    r = _client(w).post("/evaluation/admissions", json=body(w, pid, chash, **over), headers=_h("evaluation_admit"))
    assert r.status_code == 422 and r.json()["code"] == "pulso:invalid_request"
    assert w.rt.admissions.get(expected_ref(pid, chash)) is None
