"""Evaluation routes: admissions (D.4) and arms (run / by-key / by-id). Admission success needs a frozen candidate
(`needs("writer")`); arms need a sealed scenario manifest (`needs("arms")`). Everything else is closed-DTO checks."""

from __future__ import annotations

import hashlib
import uuid
from datetime import UTC, datetime, timedelta

import pytest

from conformance.conftest import assert_error, assert_valid
from conformance.kit import CONTRACT, binding_ref, evaluation_context_ref, idempotency_key, schema_errors
from conformance.worlds import World

pytestmark = pytest.mark.needs("evaluation")


def nonce() -> str:
    return uuid.uuid4().hex[:10]


def deadline(hours: float = 1) -> str:
    return (datetime.now(UTC) + timedelta(hours=hours)).isoformat().replace("+00:00", "Z")


def derived_ref(world: World, body: dict, job: str = "job-adm") -> str:
    return evaluation_context_ref(world.tenant, job, body["binding_ref"], body["proposal_id"], body["candidate_hash"],
                                  body["evaluation_attempt"])


def admission_body(world: World, cand: dict, ref: str, *, attempt: int = 1, **over) -> dict:  # type: ignore[no-untyped-def]
    body = {"schema_version": "1", "binding_ref": f"bind-{ref}",
            "proposal_id": cand["proposal_id"], "candidate_hash": cand["candidate_hash"],
            "suite_id": cand["suite_id"], "suite_version": cand["suite_version"], "suite_digest": cand["suite_digest"],
            "evaluation_attempt": attempt, "budget_ref": world.budget_ref, "deadline": deadline(),
            "request_digest": "d" * 64}
    body.update(over)
    return body


def admit(world: World, body: dict, job: str = "job-adm", **kw):  # type: ignore[no-untyped-def]
    return world.api.call("POST", "/evaluation/admissions", purpose="evaluation_admit", body=body, job_id=job, **kw)


# -- admissions: decided before any candidate state ----------------------------------------------------------------
def test_admission_request_is_a_closed_dto(world: World) -> None:
    good = {"schema_version": "1", "binding_ref": "b", "proposal_id": "p",
            "candidate_hash": "c", "suite_id": "s", "suite_version": "1", "suite_digest": "d", "evaluation_attempt": 1,
            "budget_ref": "bud", "deadline": "2030-01-01T00:00:00Z", "request_digest": "a" * 64}
    assert not schema_errors("EvaluationAdmissionRequest", good)
    for bad in ({**good, "extra": 1}, {**good, "evaluation_attempt": 0}, {**good, "evaluation_context_ref": "a b"},
                {k: v for k, v in good.items() if k != "deadline"},
                {**good, "deadline": "2030-01-01T00:00:00+00:00"}, {**good, "request_digest": "A" * 64},
                {**good, "request_digest": "r"}):
        assert schema_errors("EvaluationAdmissionRequest", bad), bad


def test_admission_with_an_extra_field_is_422(world: World) -> None:
    body = {"schema_version": "1", "surprise": 1}
    assert_error(admit(world, body), 422, "pulso:invalid_request")


def test_admission_with_an_invalid_context_ref_is_422_evaluation_context_invalid(world: World) -> None:
    body = admission_body(world, {"proposal_id": "p", "candidate_hash": "c", "suite_id": "s", "suite_version": "1",
                                  "suite_digest": "d"}, "x", evaluation_context_ref="has space")
    assert_error(admit(world, body), 422, "pulso:evaluation_context_invalid")


def test_admission_requires_a_job_id_claim(world: World) -> None:
    body = admission_body(world, {"proposal_id": "p", "candidate_hash": "c", "suite_id": "s", "suite_version": "1",
                                  "suite_digest": "d"}, f"ctx-{nonce()}")
    err = assert_error(admit(world, body, job=None), 403, "pulso:auth_denied")  # type: ignore[arg-type]
    assert err["code"] == "pulso:auth_denied"


def test_admission_for_an_unknown_proposal_is_404(world: World) -> None:
    body = admission_body(world, {"proposal_id": "no-such-proposal", "candidate_hash": "c", "suite_id": "s",
                                  "suite_version": "1", "suite_digest": "d"}, f"ctx-{nonce()}")
    assert_error(admit(world, body), 404, "pulso:proposal_not_found")


def test_admission_with_an_unknown_budget_is_403(world: World) -> None:
    body = admission_body(world, {"proposal_id": "p", "candidate_hash": "c", "suite_id": "s", "suite_version": "1",
                                  "suite_digest": "d"}, f"ctx-{nonce()}", budget_ref="bud-missing-xyz")
    assert_error(admit(world, body), 403, "pulso:budget_unknown")


def test_admission_past_its_deadline_is_409_expired(world: World) -> None:
    body = admission_body(world, {"proposal_id": "p", "candidate_hash": "c", "suite_id": "s", "suite_version": "1",
                                  "suite_digest": "d"}, f"ctx-{nonce()}", deadline=deadline(-1))
    assert_error(admit(world, body), 409, "pulso:admission_expired")


# -- admissions on a frozen candidate -------------------------------------------------------------------------------
@pytest.mark.needs("evaluation", "writer")
def test_admission_is_201_then_200_on_replay_and_conflicts_on_a_different_body(world: World) -> None:
    cand = world.new_candidate(nonce())
    body = admission_body(world, cand, nonce())
    ref = derived_ref(world, body)
    first = admit(world, body)
    assert first.status_code == 201, first.text
    assert_valid("EvaluationAdmission", first.json())
    assert first.json() == {"schema_version": "1", "evaluation_context_ref": ref, "state": "admitted"}
    replay = admit(world, body)
    assert replay.status_code == 200 and replay.json() == first.json()
    other = {**body, "request_digest": "e" * 64}  # same derived ref, other digest
    assert_error(admit(world, other), 409, "pulso:idempotency_conflict")
    # an explicit ref is accepted only when it equals the derived one; a client-chosen ref is not annex D.4
    assert admit(world, {**body, "evaluation_context_ref": ref}).status_code == 200
    assert_error(admit(world, {**body, "evaluation_context_ref": "client-chosen"}), 422,
                 "pulso:evaluation_context_invalid")
    # a new attempt is a new admission with its own derived ref
    two = admit(world, {**body, "evaluation_attempt": 2})
    assert two.status_code == 201 and two.json()["evaluation_context_ref"] == derived_ref(
        world, {**body, "evaluation_attempt": 2})


@pytest.mark.needs("evaluation", "writer")
@pytest.mark.parametrize("field,code", [("candidate_hash", "pulso:candidate_changed"),
                                        ("suite_digest", "pulso:suite_mismatch")])
def test_stale_candidate_or_suite_is_409(world: World, field: str, code: str) -> None:
    cand = world.new_candidate(nonce())
    body = admission_body(world, cand, f"ctx-{nonce()}", **{field: "0" * 64})
    assert_error(admit(world, body), 409, code)


# -- arms ---------------------------------------------------------------------------------------------------------
def arm_body(world: World, key: str, mode: str = "native", **over) -> dict:  # type: ignore[no-untyped-def]
    body = {"idempotency_key": key, "binding_ref": "bind-arm", "campaign_ref": "camp-1", "case_ref": "case-1",
            "arm": "baseline", "repetition": 0, "seed": 7, "mode": mode, "agent_id": world.arm_agent,
            "target": {"kind": "published_release", "release_id": world.arm_release},
            "scenario_manifest_ref": over.pop("manifest", "art-missing"), "budget_ref": world.budget_ref,
            "seed_manifest_ref": None if mode == "native" else "seed-1"}
    body.update(over)
    return body


def run_arm(world: World, body: dict, **kw):  # type: ignore[no-untyped-def]
    return world.api.call("POST", "/evaluation/arms/run", purpose="evaluation_arm_run", body=body, **kw)


def execution_id(tenant: str, key: str) -> str:
    return "arm-" + hashlib.sha256(f"{tenant}|{key}".encode()).hexdigest()[:32]


def test_arm_request_is_closed_and_oracles_cannot_ride_along(world: World) -> None:
    body = arm_body(world, "k-1")
    assert not schema_errors("ArmRequest", body)
    assert schema_errors("ArmRequest", {**body, "oracle_gold": {"answer": 1}})
    assert_error(run_arm(world, {**body, "oracle_gold": {"answer": 1}}), 422, "pulso:invalid_request")


def test_arm_key_charset_is_enforced(world: World) -> None:
    assert_error(run_arm(world, arm_body(world, "bad key!")), 422, "pulso:idempotency_key_invalid")


def test_native_arm_with_a_bank_pointer_is_a_mixed_world_409(world: World) -> None:
    body = arm_body(world, f"k-{nonce()}", seed_manifest_ref="seed-1")
    assert_error(run_arm(world, body), 409, "pulso:mixed_world_rejected")


def test_task_arm_without_a_seed_manifest_is_409_sandbox_required(world: World) -> None:
    body = arm_body(world, f"k-{nonce()}", "task_builder", seed_manifest_ref=None)
    assert_error(run_arm(world, body), 409, "pulso:sandbox_required")


def test_arm_supersedes_only_a_reconciled_unknown_arm(world: World) -> None:
    body = arm_body(world, f"k-{nonce()}", supersedes_execution_id="arm-" + "0" * 32)
    assert_error(run_arm(world, body), 409, "pulso:supersedes_invalid")


@pytest.mark.parametrize("path", ["/evaluation/arms/arm-" + "0" * 32, "/evaluation/arms/by-key/no-such-key"])
def test_arm_readback_of_unknown_executions_is_404(world: World, path: str) -> None:
    assert_error(world.api.call("GET", path, purpose="evaluation_arm_read"), 404, "pulso:not_found")


@pytest.mark.needs("evaluation", "arms")
def test_native_arm_runs_replays_and_reads_back_by_id_and_by_key(world: World) -> None:
    ref, key = f"art-{nonce()}", f"k-{nonce()}"
    world.seal_manifest(ref)
    body = arm_body(world, key, manifest=ref)
    first = run_arm(world, body)
    assert first.status_code == 200, first.text
    report = first.json()
    assert_valid("ArmReport", report)
    assert report["status"] == "completed" and report["execution_id"] == execution_id(world.tenant, key)
    assert report["usage"] is None or isinstance(report["usage"], dict)
    assert report["cost_known"] in (True, False)
    replay = run_arm(world, body)
    assert replay.status_code == 200 and replay.json() == report, "same key + digest returns the stored report"
    for path in (f"/evaluation/arms/{report['execution_id']}", f"/evaluation/arms/by-key/{key}"):
        got = world.api.call("GET", path, purpose="evaluation_arm_read")
        assert got.status_code == 200 and got.json() == report
    foreign = world.api.call("GET", f"/evaluation/arms/{report['execution_id']}", purpose="evaluation_arm_read",
                             tenant=world.other_tenant)
    assert_error(foreign, 404, "pulso:not_found")
    assert_error(run_arm(world, {**body, "seed": 8}), 409, "pulso:idempotency_conflict")


@pytest.mark.needs("evaluation", "arms")
def test_arm_failures_inside_the_run_are_reported_not_raised(world: World) -> None:
    ref = f"art-{nonce()}"
    world.seal_manifest(ref)
    no_budget = run_arm(world, arm_body(world, f"k-{nonce()}", manifest=ref, budget_ref="bud-missing-xyz"))
    assert no_budget.status_code == 200, no_budget.text
    assert_valid("ArmReport", no_budget.json())
    assert no_budget.json()["status"] == "failed_infra" and no_budget.json()["reason"] == "budget_unknown"
    no_manifest = run_arm(world, arm_body(world, f"k-{nonce()}", manifest="art-not-there"))
    assert no_manifest.json()["status"] == "failed_infra" and no_manifest.json()["reason"] == "manifest_missing"
    assert set(CONTRACT["arm_reasons"]) >= {"budget_unknown", "manifest_missing"}


@pytest.mark.needs("evaluation", "arms")
def test_arm_run_by_path_id_is_the_same_operation(world: World) -> None:
    ref, key = f"art-{nonce()}", f"k-{nonce()}"
    world.seal_manifest(ref)
    resp = world.api.call("POST", f"/evaluation/arms/{execution_id(world.tenant, key)}/run",
                          purpose="evaluation_arm_run", body=arm_body(world, key, manifest=ref))
    assert resp.status_code == 200, resp.text
    assert_valid("ArmReport", resp.json())


def test_binding_ref_helper_is_stable(world: World) -> None:
    k = idempotency_key("t", "j", "writer", 2, "x")
    assert binding_ref("t", k) == hashlib.sha256(f"t|{k}".encode()).hexdigest()


# -- arms: annex D.4 names, header key, deadline format -------------------------------------------------------------
def annex_arm_body(world: World, **over) -> dict:  # type: ignore[no-untyped-def]
    body = arm_body(world, "unused", **over.pop("base", {}))
    for k in ("mode", "agent_id", "seed_manifest_ref", "idempotency_key"):
        body.pop(k)
    body.update({"execution_profile": "evolution_task", "sandbox_session_ref": None,
                 "deadline": "2030-01-01T00:00:00Z"})
    body.update(over)
    return body


def test_arm_request_schema_takes_annex_names_and_deprecated_aliases(world: World) -> None:
    assert not schema_errors("ArmRequest", annex_arm_body(world))  # no mode/agent_id/seed_manifest_ref/body key
    assert not schema_errors("ArmRequest", arm_body(world, "k-1"))  # deprecated spelling still valid
    assert schema_errors("ArmRequest", annex_arm_body(world, deadline="2030-01-01T00:00:00+00:00"))
    assert schema_errors("ArmRequest", annex_arm_body(world, execution_profile="other"))
    no_profile = annex_arm_body(world)
    del no_profile["execution_profile"]
    assert schema_errors("ArmRequest", no_profile)


def test_arm_idempotency_key_header_must_equal_the_body_key(world: World) -> None:
    body = arm_body(world, "k-body")
    assert_error(run_arm(world, body, headers={"Idempotency-Key": "k-other"}), 422, "pulso:invalid_request")
    assert_error(run_arm(world, annex_arm_body(world), headers={"Idempotency-Key": "bad key!"}), 422,
                 "pulso:invalid_request")


def test_arm_with_a_non_z_deadline_is_422(world: World) -> None:
    body = annex_arm_body(world, deadline="2030-01-01T00:00:00+00:00")
    assert_error(run_arm(world, body, headers={"Idempotency-Key": f"k-{nonce()}"}), 422, "pulso:invalid_request")


@pytest.mark.needs("evaluation", "arms")
def test_arm_runs_with_annex_names_and_the_key_in_the_header_only(world: World) -> None:
    ref, key = f"art-{nonce()}", f"k-{nonce()}"
    world.seal_manifest(ref)
    body = annex_arm_body(world, scenario_manifest_ref=ref)
    first = run_arm(world, body, headers={"Idempotency-Key": key})
    assert first.status_code == 200, first.text
    assert_valid("ArmReport", first.json())
    assert first.json()["execution_id"] == execution_id(world.tenant, key)
    replay = run_arm(world, body, headers={"Idempotency-Key": key})
    assert replay.status_code == 200 and replay.json() == first.json()
