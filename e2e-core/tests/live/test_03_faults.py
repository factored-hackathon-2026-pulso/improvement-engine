"""Injected faults: unknown outcomes stay unknown (never promoted to success), no duplicate effects, fail closed."""

from __future__ import annotations

from typing import Any

import pytest

from codex_standin.engine import (
    DESIGN,
    RESEARCH,
    TENANT,
    candidate_changes,
    hypotheses_output,
    verification_output,
)
from helpers import OPS, arm_body, run_arm, tag, writer_commitment

pytestmark = pytest.mark.live
NOT_OK = {"unknown", "manual_reconcile", "terminal_failed", "failed", "denied"}


def _scout(e: Any, n: str, logical: str) -> Any:
    return e.stage("scout", f"job-f-{n}", logical, "pulso-scout", {"briefing_ref": f"wiki/b-{n}.md"},
                   memory_snapshot_ref=f"mem-{n}", extract_manifest_ref=f"ex-{n}")


def _only_scout_script(e: Any, n: str) -> None:
    e.configure(llm_replace=True, llm_rules=[])  # clears every previous rule: nothing from other tests can answer
    e.script_stage_model(f"scout-f-{n}", RESEARCH, hypotheses_output(TENANT))


def test_binding_unavailable_before_effect_is_unknown_with_zero_effects_and_no_model_call(stack: Any, effect: Any) -> None:
    e, n = stack.engine, tag()
    _only_scout_script(e, n)
    calls = len(e.state()["llm_calls"])
    e.configure(faults={"bind": ["503", "503"]})  # the runtime retries a network error once, not a 503
    s = _scout(e, n, "bind503")
    assert s.out["state"] in NOT_OK, s.out  # never terminal_ok on an unproven binding
    st = e.state()
    assert st["binding_effects"].get(f"{TENANT}|job-f-{n}", 0) == 0  # the double applied nothing
    assert len(st["llm_calls"]) == calls  # tools stay denied: the model is never reached
    again = e.bridge.invoke(TENANT, s.key, s.body)  # same key: receipt replay, no re-dispatch to Core
    assert again.status_code in (200, 202) and again.json()["state"] in NOT_OK
    assert e.state()["binding_effects"].get(f"{TENANT}|job-f-{n}", 0) == 0
    e.configure(faults={"bind": []})
    retry = _scout(e, n, "bind503-retry")  # a new logical key is a new attempt: the happy path still works
    assert retry.out["state"] == "terminal_ok", retry.out
    assert e.state()["binding_effects"][f"{TENANT}|job-f-{n}"] == 1
    effect("fault_bind_503_state", s.out["state"])


def test_binding_applied_then_answer_lost_is_never_reported_as_success_and_never_applied_twice(stack: Any, effect: Any) -> None:
    e, n = stack.engine, tag()
    _only_scout_script(e, n)
    e.configure(faults={"bind": ["applied_then_503"]})
    s = _scout(e, n, "lost")
    st = e.state()
    assert st["binding_effects"][f"{TENANT}|job-f-{n}"] == 1  # the effect exists on the platform side ...
    assert s.out["state"] in NOT_OK, s.out  # ... but the bridge cannot prove it: unknown, escalated to reconcile
    assert s.out.get("outcome") != "completed"
    calls = len(st["llm_calls"])
    again = e.bridge.invoke(TENANT, s.key, s.body)
    assert again.json()["state"] in NOT_OK
    st2 = e.state()
    assert st2["binding_effects"][f"{TENANT}|job-f-{n}"] == 1  # replay never binds a second time
    assert len(st2["llm_calls"]) == calls
    effect("fault_bind_lost_answer_state", s.out["state"])


def test_unproven_binding_is_reported_as_unknown_not_as_a_definitive_failure(stack: Any, gap: Any, effect: Any) -> None:
    """Strict expectation (A02/D.1): a binding answer lost after the platform applied it leaves the effect unproven,
    so the receipt must stay `unknown`/`manual_reconcile` for reconciliation; `terminal_failed` invites a redispatch
    that the platform (same job, other command key) will refuse as binding_conflict forever. Today the Flow path
    (`pulso/bind_context` -> denied -> escalation) ends `terminal_failed`: recorded as a gap, xfail."""
    e, n = stack.engine, tag()
    _only_scout_script(e, n)
    e.configure(faults={"bind": ["applied_then_503"]})
    s = _scout(e, n, "lost-strict")
    assert e.state()["binding_effects"][f"{TENANT}|job-f-{n}"] == 1
    if s.out["state"] in ("unknown", "manual_reconcile"):
        return
    gap("binding_5xx_reported_as_terminal_failed",
        f"binding callback answered 503 after the platform applied the binding: invoke state={s.out['state']} "
        f"reason={s.out.get('reason')} (the effect exists, so the outcome is unknown, not failed).",
        "L3a/L3b: when `pulso/bind_context` hits a 5xx/timeout (effect unproven) transition the receipt to "
        "manual_reconcile (BindingService._unproven already does) instead of letting the escalation close it as "
        "terminal_failed.")
    pytest.xfail("unproven binding closed as terminal_failed (gap binding_5xx_reported_as_terminal_failed)")


def test_unscripted_model_call_fails_closed_and_is_counted(stack: Any, effect: Any) -> None:
    e, n = stack.engine, tag()
    e.configure(llm_replace=True, llm_rules=[])
    before = e.state()["llm_unscripted"]
    s = _scout(e, n, "noscript")
    assert s.out["state"] != "terminal_ok" and s.out.get("outcome") != "completed", s.out
    unscripted = e.state()["llm_unscripted"] - before
    assert unscripted >= 1  # surfaced, never silently answered by a default
    effect("unscripted_llm_calls_injected", unscripted)


def test_broker_revocation_before_freeze_leaves_an_unfrozen_draft_and_no_freeze_effect(stack: Any, effect: Any) -> None:
    e, n = stack.engine, tag()
    changes, title = candidate_changes("1.2.0"), f"pulso-key:e2e-deny-{n}"
    e.seal(f"plan-{n}", {"agent_id": "pulso-scout", "title": title, "changes": changes})
    base = e.releases["pulso-scout"]
    e.configure(deny_operations=["registry/freeze"])
    try:
        w = e.stage("writer", f"job-wd-{n}", "w", "pulso-writer", {
            "draft_plan_ref": f"plan-{n}", "proposal_id": None, "base_release_id": base, "evaluate_enabled": False},
            registry_mutation_commitment=writer_commitment(base, title, changes))
    finally:
        e.configure(deny_operations=[])
    assert w.out["state"] != "terminal_ok", w.out
    db = stack.runtime_db
    pid = db.one("select proposal_id from reg_proposals where proposal_json::json->>'title' = %s", title)
    assert pid is not None  # create + put happened (authorised) ...
    ops = [r[0] for r in db.rows("select op from reg_draft_writes where proposal_id=%s order by created_at", pid)]
    assert "freeze" not in ops and ops[:2] == OPS[:2]  # ... freeze was denied: zero effect
    assert db.one("select proposal_json::json->>'state' from reg_proposals where proposal_id=%s", pid) != "candidate"
    effect("fault_freeze_revoked_ops", ops)


def test_commitment_mismatch_is_denied_with_zero_registry_effect(stack: Any, effect: Any) -> None:
    e, n = stack.engine, tag()
    changes, title = candidate_changes("1.3.0"), f"pulso-key:e2e-mm-{n}"
    e.seal(f"plan-{n}", {"agent_id": "pulso-scout", "title": title, "changes": changes})
    base = e.releases["pulso-scout"]
    sealed = writer_commitment(base, title, changes)
    sealed["put_draft_digest"] = "0" * 64  # the Rust engine sealed another draft than the plan the Flow will put
    w = e.stage("writer", f"job-wm-{n}", "w", "pulso-writer", {
        "draft_plan_ref": f"plan-{n}", "proposal_id": None, "base_release_id": base, "evaluate_enabled": False},
        registry_mutation_commitment=sealed)
    assert w.out["state"] != "terminal_ok", w.out
    ops = [r[0] for r in stack.runtime_db.rows(
        "select w.op from reg_draft_writes w join reg_proposals p on p.proposal_id=w.proposal_id "
        "where p.proposal_json::json->>'title' = %s", title)]
    assert "put_draft" not in ops and "freeze" not in ops  # the mismatching write never happened
    effect("fault_commitment_mismatch_ops", ops)


def test_sandbox_down_is_failed_infra_without_effects_and_a_new_key_recovers(stack: Any, pipeline: Any, effect: Any) -> None:
    e = stack.engine
    sessions = len(e.state()["sessions"])
    key = f"arm-down-{tag()}"
    bref = pipeline.writer.out["task_binding_ref"]
    target = {"kind": "published_release", "release_id": pipeline.base_rel}
    e.configure(faults={"sandbox_open": ["503"]})
    down = run_arm(e, arm_body(key, bref, pipeline.manifest, "task_builder", target))
    assert down.report["status"] == "failed_infra", down.report
    assert len(e.state()["sessions"]) == sessions  # nothing was opened on the bank
    assert not any(v for v in e.state()["bank_effects"].values() if v != 1)
    ok = run_arm(e, arm_body(key + "-retry", bref, pipeline.manifest, "task_builder", target))
    assert ok.report["status"] == "completed", ok.report
    effect("fault_sandbox_down_status", down.report["status"])


def test_bank_lost_response_readback_needs_a_stateful_scenario(stack: Any) -> None:
    pytest.skip("not_run: the seeded pulso-smoke scenarios issue no bank actions, so `lose_response` (effect applied, "
                "answer lost -> readback by key) cannot be exercised through the real runtime; covered in-process by "
                "core-bridge tests/integration/test_arm_broker.py (loopback bank double)")


def test_models_other_than_the_scripted_stage_prompts_are_not_needed(stack: Any) -> None:
    e = stack.engine  # sanity: the markers that route model answers exist in the seeded prompts
    from codex_standin.engine import ASSETS, VERIFY
    texts = {p.name: p.read_text(encoding="utf-8") for p in (ASSETS / "worlds/pulso-evolution/prompts/p").glob("*.yaml")}
    assert any(RESEARCH in t for t in texts.values()) and any(VERIFY in t for t in texts.values())
    assert any(DESIGN in t for t in texts.values())
    _ = (e, verification_output)
