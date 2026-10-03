"""Exactly-once effects on replay, closed conflicts, auth negatives and tenant isolation (read side)."""

from __future__ import annotations

from typing import Any

import httpx
import pytest

from codex_standin.dto import invocation
from codex_standin.engine import OTHER, TENANT

pytestmark = pytest.mark.live


def _counts(stack: Any) -> dict[str, Any]:
    st = stack.engine.state()
    return {"bindings": dict(st["binding_effects"]), "llm": len(st["llm_calls"]), "bank": dict(st["bank_effects"]),
            "runs": stack.runtime_db.one("select count(*) from runs"),
            "proposals": stack.runtime_db.one("select count(*) from reg_proposals"),
            "writes": stack.runtime_db.one("select count(*) from reg_draft_writes"),
            "receipts": stack.runtime_db.one("select count(*) from pulso_bridge.receipts")}


def test_replaying_every_stage_with_the_same_key_and_body_has_no_second_effect(stack: Any, pipeline: Any, effect: Any) -> None:
    before = _counts(stack)
    for name in ("scout", "verifier", "design", "writer"):
        first = getattr(pipeline, name)
        again = stack.bridge.invoke(TENANT, first.key, first.body)
        assert again.status_code == 200, (name, again.text)
        a, b = first.out, again.json()
        assert (b["state"], b["core_run_id"], b["task_binding_ref"], b["outcome"]) == (
            a["state"], a["core_run_id"], a["task_binding_ref"], a["outcome"]), name
    after = _counts(stack)
    assert after == before  # no second run, binding, model call, registry write, proposal or receipt
    effect("replay_extra_effects", {k: (after[k] != before[k]) for k in before})


def test_same_key_with_another_body_is_a_closed_digest_conflict_with_no_effect(stack: Any, pipeline: Any) -> None:
    before = _counts(stack)
    body = {**pipeline.scout.body, "input": {"briefing_ref": "wiki/something-else.md"}}
    r = stack.bridge.invoke(TENANT, pipeline.scout.key, body)
    assert r.status_code == 409 and r.json()["code"] == "pulso:digest_conflict"
    assert _counts(stack) == before


def test_auth_negatives_never_reach_a_run(stack: Any, pipeline: Any) -> None:
    before = _counts(stack)
    br = stack.bridge
    body, key = pipeline.scout.body, pipeline.scout.key
    assert httpx.post(br.base + "/core-tasks/invoke", json=body, headers={"Idempotency-Key": key}).status_code == 401
    wrong = br.call("POST", "/core-tasks/invoke", "read", TENANT, json=body, headers={"Idempotency-Key": key})
    assert wrong.status_code == 403 and wrong.json()["details"]["reason"] == "purpose_denied"
    notenant = br.call("POST", "/core-tasks/invoke", "invoke", None, json=body, headers={"Idempotency-Key": key})
    assert notenant.status_code == 403 and notenant.json()["details"]["reason"] == "tenant_required"
    tok = br.token("core_task_invoke", TENANT)
    hdr = {"Authorization": "Bearer " + tok, "Idempotency-Key": key}
    assert httpx.post(br.base + "/core-tasks/invoke", json=body, headers=hdr).status_code == 200
    replay = httpx.post(br.base + "/core-tasks/invoke", json=body, headers=hdr)  # same jti: receiver-owned replay
    assert replay.status_code == 401 and replay.json()["details"]["reason"] == "jti_replayed"
    forged = br.token("core_task_invoke", TENANT)[:-4] + "AAAA"
    assert httpx.post(br.base + "/core-tasks/invoke", json=body,
                      headers={"Authorization": "Bearer " + forged, "Idempotency-Key": key}).status_code == 401
    # a body tenant that differs from the signed claim is refused before any state is touched
    mismatch = br.invoke(OTHER, key, body)
    assert mismatch.status_code == 403 and mismatch.json()["code"] == "pulso:tenant_mismatch"
    assert _counts(stack) == before


def test_another_tenant_cannot_read_arms_admissions_or_tasks_of_this_tenant(stack: Any, pipeline: Any) -> None:
    br = stack.bridge
    for stage in (pipeline.scout, pipeline.writer):
        assert br.read_task(TENANT, stage.out["core_run_id"]).status_code == 200
        denied = br.read_task(OTHER, stage.out["core_run_id"])  # the deployment tenant set is enforced at the door
        assert denied.status_code == 403 and denied.json()["code"] == "pulso:tenant_mismatch"
    assert br.arm_by_key(OTHER, pipeline.arm_bank.body["idempotency_key"]).status_code == 403
    rep = pipeline.arm_bank.report
    assert br.call("GET", f"/evaluation/arms/{rep['execution_id']}", "arm_read", OTHER).status_code == 403
    # admitting with this tenant's binding under another tenant's token: the broker double refuses (binding unknown
    # to that tenant), zero admission rows
    rows = stack.runtime_db.one("select count(*) from pulso_bridge.eval_admissions")
    other_before = len([b for b in stack.engine.state()["bindings"] if b["tenant"] == OTHER])
    authz_before = len([r for r in stack.engine.state()["requests"] if r["tenant"] == OTHER and r["route"] == "authz"])
    reqs_before = len([r for r in stack.engine.state()["requests"] if r["tenant"] == OTHER and r["route"] != "authz"])
    other = {**pipeline.admission_body, "evaluation_context_ref": pipeline.ctx_ref + "-x"}
    r = br.admit(OTHER, "job-x", other)
    assert r.status_code == 403
    assert stack.runtime_db.one("select count(*) from pulso_bridge.eval_admissions") == rows
    arm = {**pipeline.arm_bank.body, "idempotency_key": pipeline.arm_bank.body["idempotency_key"] + "-x"}
    assert br.arm_run(OTHER, arm).status_code == 403  # broker_denied: the binding belongs to this tenant
    st = stack.engine.state()
    assert len([b for b in st["bindings"] if b["tenant"] == OTHER]) == other_before  # nothing bound for the intruder
    assert len([r for r in st["requests"] if r["tenant"] == OTHER and r["route"] != "authz"]) == reqs_before
    new_authz = [r for r in st["requests"] if r["tenant"] == OTHER and r["route"] == "authz"][authz_before:]
    assert new_authz == [] or all(r["body"]["allowed"] is False for r in new_authz)


def test_unknown_release_and_wrong_stage_release_are_refused_before_a_run(stack: Any, pipeline: Any) -> None:
    before = _counts(stack)
    e = stack.engine
    wrong = e.stage("scout", "job-wr-" + pipeline.n, "wr", "pulso-scout", {"briefing_ref": "wiki/b.md"},
                    release=e.releases["pulso-verifier"], memory_snapshot_ref="m", extract_manifest_ref="x")
    assert wrong.response.status_code == 409 and wrong.out["code"] == "pulso:release_pin_unavailable"
    unknown = e.stage("scout", "job-ur-" + pipeline.n, "ur", "pulso-scout", {"briefing_ref": "wiki/b.md"},
                      release="rel-does-not-exist", memory_snapshot_ref="m", extract_manifest_ref="x")
    assert unknown.response.status_code == 409
    after = _counts(stack)
    assert {k: v for k, v in after.items() if k != "receipts"} == {k: v for k, v in before.items() if k != "receipts"}
    assert after["receipts"] == before["receipts"] + 2  # the two refusals are recorded as closed terminal_failed receipts


def test_stage_and_agent_must_pair_up_before_a_run(stack: Any, pipeline: Any, effect: Any) -> None:
    """`stage=writer` (constructor authority) naming the scout agent is refused 422 pulso:stage_agent_mismatch before
    the receipt CAS: zero effects (no receipt, no Core run, no binding)."""
    e = stack.engine
    before = _counts(stack)
    key, body = invocation(tenant=TENANT, job=f"job-mm-{pipeline.n}", stage="writer", agent_id="pulso-scout",
                           release_id=e.releases["pulso-scout"], logical="mm")
    r = stack.bridge.invoke(TENANT, key, body)
    assert r.status_code == 422, r.text
    assert r.json()["code"] == "pulso:stage_agent_mismatch", r.json()
    assert _counts(stack) == before  # zero effects, not even a receipt
    assert e.state()["binding_effects"].get(f"{TENANT}|job-mm-{pipeline.n}", 0) == 0
    effect("stage_agent_mismatch_code", r.json()["code"])
