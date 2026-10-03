"""Happy path against the REAL runtime: pin -> scout -> verifier (other agent) -> builder_design -> writer ->
admission -> evaluate-only invocation -> arms with the fixture bank -> report. Model = scripted double."""

from __future__ import annotations

from typing import Any

import pytest

from codex_standin.dto import idempotency_key, request_digest
from codex_standin.engine import TENANT
from codex_standin.fixtures_app import lab_refs

pytestmark = pytest.mark.live


def _ok(stage: Any) -> dict[str, Any]:
    assert stage.response.status_code == 200, stage.response.text
    assert stage.out["state"] == "terminal_ok" and stage.out["outcome"] == "completed", stage.out
    return stage.out  # type: ignore[no-any-return]


def test_pin_release_ids_match_the_manifest_and_are_active_in_the_registry(stack: Any) -> None:
    e = stack.engine
    rows = dict(stack.runtime_db.rows("select release_id, status from reg_release_status"))
    for agent, release in e.releases.items():
        assert rows.get(release) == "active", (agent, release)
    ver = stack.bridge.version()
    assert ver["contracts_version"] == "1.3.0" and ver["runtime_profile"] == "agent_core_real"


def test_scout_runs_the_real_flow_with_a_scripted_model_and_binds_exactly_once(stack: Any, pipeline: Any, effect: Any) -> None:
    out = _ok(pipeline.scout)
    assert out["receipt"]["release_id"] == stack.engine.releases["pulso-scout"]
    hyp = pipeline.scout_facts["pulso_hypotheses"]["value"]["hypotheses"][0]
    ref = hyp["evidence_refs"][0]
    assert ref["id"] == lab_refs(TENANT)["result"] and ref["digest"] == lab_refs(TENANT)["digest"]  # fetched, not invented
    st = stack.engine.state()
    job = f"{TENANT}|job-scout-{pipeline.n}"
    assert st["binding_effects"][job] == 1
    # the idempotency key and request digest the stand-in computed are the ones the runtime stored
    assert idempotency_key(TENANT, f"job-scout-{pipeline.n}", "scout", 1, "scout") == pipeline.scout.key
    row = stack.runtime_db.rows("select request_digest, state, release_id from pulso_bridge.receipts where "
                                "tenant_id=%s and job_id=%s", TENANT, f"job-scout-{pipeline.n}")
    assert len(row) == 1  # exactly one receipt for the one invocation
    assert row[0][0] == request_digest(pipeline.scout.body) and row[0][1] == "terminal_ok"  # digests agree    effect("scout_binding_effects", st["binding_effects"][job])


def test_verifier_is_a_different_agent_release_and_job_and_retrieves_its_own_evidence(stack: Any, pipeline: Any) -> None:
    out = _ok(pipeline.verifier)
    assert out["receipt"]["release_id"] == stack.engine.releases["pulso-verifier"] != stack.engine.releases["pulso-scout"]
    assert pipeline.verifier.body["agent_id"] != pipeline.scout.body["agent_id"]
    assert pipeline.verifier.body["job_id"] != pipeline.scout.body["job_id"]
    assert out["task_binding_ref"] != pipeline.scout.out["task_binding_ref"]
    a = pipeline.verifier_facts["pulso_verification"]["value"]["assessments"][0]
    assert a["hypothesis_id"] == "h1" and a["verdict"] == "supported"
    st = stack.engine.state()
    lab_reads = [r for r in st["requests"] if r["route"] == "lab_result"]
    assert len(lab_reads) >= 2  # scout and verifier each fetched evidence through their own binding
    art = [r for r in st["requests"] if r["route"] == "artifact" and r["body"]["id"] == f"hyp-{pipeline.n}"]
    assert art, "the verifier reads the sealed hypotheses artifact through the broker"


def test_builder_design_produces_a_do_nothing_alternative_set(pipeline: Any) -> None:
    _ok(pipeline.design)
    spec = pipeline.design_facts["pulso_change_spec"]["value"]
    assert {a["kind"] for a in spec["alternatives"]} >= {"do_nothing", "proposed_change"}


def test_writer_commits_create_put_freeze_with_derived_keys_and_exactly_one_proposal(stack: Any, pipeline: Any, effect: Any) -> None:
    _ok(pipeline.writer)
    wr = pipeline.writer_facts["pulso_writer_receipts"]["value"]
    assert [r["op"] for r in wr["write_receipts"]] == ["create_proposal", "put_draft", "freeze"]
    assert all(r["verified"] for r in wr["write_receipts"]) and wr["native_evaluation"] is None
    db = stack.runtime_db
    assert db.one("select count(*) from reg_proposals where proposal_json::json->>'title' = %s", pipeline.title) == 1
    ops = [r[0] for r in db.rows("select op from reg_draft_writes where proposal_id=%s order by created_at",
                                 pipeline.proposal_id)]
    assert ops == ["create_proposal", "put_draft", "freeze"]  # one effect per committed write
    assert db.one("select proposal_json::json->>'state' from reg_proposals where proposal_id=%s",
                  pipeline.proposal_id) == "candidate"
    authz = [r["body"]["operation"] for r in stack.engine.state()["requests"] if r["route"] == "authz"]
    for op in ("registry/create_proposal", "registry/put_draft", "registry/freeze"):
        assert op in authz  # every effect was authorised by the broker first
    assert db.one("select count(*) from reg_eval_runs where proposal_id=%s", pipeline.proposal_id) == 0
    effect("writer_registry_writes", ops)


def test_evaluation_admission_is_created_and_a_replay_is_the_same_admission(stack: Any, pipeline: Any) -> None:
    assert pipeline.admit.status_code == 201, pipeline.admit.text
    assert pipeline.admit.json()["state"] == "admitted"
    again = stack.bridge.admit(TENANT, pipeline.writer.body["job_id"], pipeline.admission_body)
    assert again.status_code == 200 and again.json()["state"] == "admitted"
    assert stack.runtime_db.one("select count(*) from pulso_bridge.eval_admissions where evaluation_context_ref=%s",
                                pipeline.ctx_ref) == 1
    stale = {**pipeline.admission_body, "evaluation_context_ref": pipeline.ctx_ref + "-b",
             "candidate_hash": "0" * 64}
    r = stack.bridge.admit(TENANT, pipeline.writer.body["job_id"], stale)
    assert r.status_code == 409 and r.json()["code"] == "pulso:candidate_changed"
    unknown_budget = {**pipeline.admission_body, "evaluation_context_ref": pipeline.ctx_ref + "-c",
                      "budget_ref": "bud-nope"}
    assert stack.bridge.admit(TENANT, pipeline.writer.body["job_id"], unknown_budget).status_code == 403


def test_evaluate_only_invocation_is_unreachable_with_the_seeded_writer_flow_and_fails_closed(stack: Any, pipeline: Any, gap: Any, effect: Any) -> None:
    """GAP (documented, asserted as the current honest behaviour): the evaluate-only invocation drives the SAME
    pulso-writer@1.0.0 Flow, whose `chk_state` routes a frozen proposal to `registry/reopen`; the protected executor
    denies every mutator in evaluate_only mode, so the Flow escalates before `registry/evaluate`. Nothing may leak
    through: no evaluation run, admission still `admitted`, the invocation is NOT terminal_ok."""
    out = pipeline.eval_only.out
    assert out["state"] != "terminal_ok", out
    assert out.get("outcome") != "completed"
    assert stack.runtime_db.one("select count(*) from reg_eval_runs where proposal_id=%s", pipeline.proposal_id) == 0
    state = stack.runtime_db.one("select state from pulso_bridge.eval_admissions where evaluation_context_ref=%s",
                                 pipeline.ctx_ref)
    assert state == "admitted"
    assert stack.runtime_db.one("select proposal_json::json->>'state' from reg_proposals where proposal_id=%s",
                                pipeline.proposal_id) == "candidate"  # not reopened
    gap("native_evaluate_unreachable_over_http",
        "POST /core-tasks/invoke stage=writer mode=evaluate_only escalates: pulso-writer@1.0.0 chk_state sends a frozen "
        "proposal (candidate_hash != null) to registry/reopen, denied in evaluate_only; registry/evaluate is never "
        "called, so the native evaluate path (A04 conditions b/c/e, report recovery) cannot be exercised end-to-end "
        "through the real runtime image.",
        "L4 (agent-core-assets): add a branch before `reopen` in pulso-writer@1.0.0: when binding.evaluate_enabled "
        "== true and proposal.candidate_hash != null go to `evaluate` (then verify_evaluate/done), keep `reopen` for "
        "iteration; or accept a distinct Flow id for evaluate-only (L3 catalog change).")
    effect("evaluate_only_state", out["state"])


def test_arms_run_against_the_bank_fixture_and_the_report_is_fetchable_by_key(stack: Any, pipeline: Any, effect: Any) -> None:
    for arm in (pipeline.arm_native, pipeline.arm_bank, pipeline.arm_cand):
        assert arm.response.status_code == 200, arm.response.text
        rep = arm.report
        assert rep["status"] == "completed", rep
        assert rep["execution_id"].startswith("arm-") and rep["event_refs"], rep
        got = stack.bridge.arm_by_key(TENANT, arm.body["idempotency_key"])
        assert got.status_code == 200 and got.json() == rep  # the report is the stored one
        by_id = stack.bridge.call("GET", f"/evaluation/arms/{rep['execution_id']}", "arm_read", TENANT)
        assert by_id.status_code == 200 and by_id.json()["execution_id"] == rep["execution_id"]
    assert pipeline.arm_bank.report["initial_state_digest"].startswith("sha256:")
    assert pipeline.arm_bank.report["final_state_ref"].startswith("final:bank-")
    assert pipeline.arm_cand.report["target_commitment"] != pipeline.arm_bank.report["target_commitment"]
    sessions = stack.engine.state()["sessions"]
    assert sessions and all(s["tenant"] == TENANT and s["closed"] == "arm_done" for s in sessions.values())
    effect("arm_bank_sessions_closed", len(sessions))


def test_arm_replay_is_exactly_once_and_a_conflicting_body_is_refused(stack: Any, pipeline: Any, effect: Any) -> None:
    before = stack.engine.state()
    sessions_before = len(before["sessions"])
    runs_before = stack.eval_db.one("select count(*) from runs")
    again = stack.bridge.arm_run(TENANT, pipeline.arm_bank.body)
    assert again.status_code == 200 and again.json() == pipeline.arm_bank.report
    assert len(stack.engine.state()["sessions"]) == sessions_before  # no second bank session / effect
    assert stack.eval_db.one("select count(*) from runs") == runs_before  # no second engine run
    conflict = stack.bridge.arm_run(TENANT, {**pipeline.arm_bank.body, "seed": 8})
    assert conflict.status_code == 409, conflict.text
    effect("arm_replay_extra_sessions", len(stack.engine.state()["sessions"]) - sessions_before)
