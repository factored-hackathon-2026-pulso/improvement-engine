"""Translator tests: REAL results bundle -> console fixture world. Golden files pin the exact wire shape."""
import copy
import json
import re
from pathlib import Path

import pytest

from pulso_demo import translate

GOLDEN = Path(__file__).resolve().parents[1] / "golden"


@pytest.fixture
def results():
    return json.loads((GOLDEN / "results.json").read_text("utf-8"))


def nodes(world, run="run-demo"):
    return {n["node_id"]: n for n in world["runs"][run]["nodes"]}


def test_golden_world_is_exactly_reproduced(results):
    got = translate.build_world(results)
    assert got == json.loads((GOLDEN / "world.json").read_text("utf-8"))


def test_failed_first_evaluation_then_automatic_revision_is_visible(results):
    n = nodes(translate.build_world(results))
    assert n["evaluate_1"]["status"] == "dead" and n["evaluate_1"]["reason_code"] == "guard_breach"
    assert n["revise"]["depends_on"] == ["evaluate_1"] and n["revise"]["status"] == "complete"
    assert n["evaluate_2"]["depends_on"] == ["revise"] and n["evaluate_2"]["status"] == "complete"


def test_two_gates_and_combined_decision_come_from_the_second_attempt(results):
    g = translate.build_world(results)["gates"]
    assert g["native"]["status"] == "pass" and g["native"]["report_ref"]["digest"]
    assert g["improvement"]["status"] == "pass" and g["combined"]["decision"] == "needs_human"


def test_alternatives_include_do_nothing_and_the_refuted_hypothesis_stays_visible(results):
    w = translate.build_world(results)
    assert "do_nothing" in [a["kind"] for a in w["demo"]["alternatives"]]
    assert w["investigation"]["run-demo-refuted"]["verifier"] == "refuted"
    assert w["runs"]["run-demo-refuted"]["nodes"][-1]["label"] == "Do nothing"


def test_human_step_is_a_hook_never_a_fake_approval(results):
    n = nodes(translate.build_world(results))
    assert n["decision"]["status"] == "waiting_dependency" and n["decision"]["reason_code"] == "human_decision_pending"
    for k in ("approve", "publish", "release"):
        assert n[k]["status"] == "planned"


def test_missing_native_report_is_unknown_not_pass(results):
    r = copy.deepcopy(results)
    r["core"]["attempts"][1]["native"] = None
    w = translate.build_world(r)
    assert nodes(w)["evaluate_2"]["status"] == "unknown" and w["gates"]["native"]["status"] == "unknown"
    assert w["gates"]["combined"]["decision"] == "hold"


def test_broken_chain_receipt_marks_observation_dead(results):
    r = copy.deepcopy(results)
    r["exporter"]["verification_receipts"][0]["check_result"] = {"ok": False, "broken_at": 3, "reason": "hash"}
    assert nodes(translate.build_world(r))["observation"]["status"] == "dead"


def test_events_are_contiguous_and_reference_existing_nodes_and_digests_are_hex(results):
    w = translate.build_world(results)
    for run_id, evs in w["events"].items():
        assert [e["sequence"] for e in evs] == list(range(1, len(evs) + 1))
        ids = {n["node_id"] for n in w["runs"][run_id]["nodes"]}
        assert all(e["entity_ref"]["id"] in ids for e in evs)
    for inv in w["investigation"].values():
        assert all(re.fullmatch(r"[0-9a-f]{64}", e["evidence_ref"]["digest"]) for e in inv["evidence"])


def test_profile_declares_every_double(results):
    d = translate.build_world(results)["demo"]["doubles"]
    assert {"stand_in_engine", "scripted_llm", "fixture_api"} <= set(d)


def test_decision_hook_default_is_pending_and_interface_is_stable():
    from pulso_demo import decision_hook
    req = decision_hook.DecisionRequest(run_id="run-demo", proposal_id="p", candidate_hash="h" * 64, operation="approve")
    assert decision_hook.PendingHook().decide(req).state == "pending"


def test_failed_design_stage_marks_change_unknown_with_a_reason(results):
    r = copy.deepcopy(results)
    r["core"]["design"]["state"] = "terminal_failed"
    n = nodes(translate.build_world(r))
    assert n["change"]["status"] == "unknown" and n["change"]["reason_code"] == "core_design_failed"


# ----------------------------------------------------------------------------------------------- steps 8-10 (decision + successor)
def decided(results, stage="staging_confirmed", **over):
    r = copy.deepcopy(results)
    r["decision"] = {"mode": "scripted", "simulated_human": True, "actor": "local-supervisor", "stage": stage, "proposal_id": "prop-core-2",
                     "candidate_hash": "c" * 64, "release_id": "rel-new" if stage in ("staging_confirmed", "promoted", "published_unconfirmed") else None,
                     "staging_alias": "rel-new" if stage in ("staging_confirmed", "promoted") else "rel-base",
                     "prod_alias": "rel-new" if stage == "promoted" else "rel-base", "promoted": stage == "promoted", "error": None, "reason": None,
                     "trail": [{"event": "decision_requested", "operation": "approve", "candidate_hash": "c" * 64, "intention_id": "int-1"},
                               {"event": "approved", "intention_id": "int-1", "published": False},
                               {"event": "published", "release_id": "rel-new"},
                               {"event": "staging_confirmed", "release_id": "rel-new", "staging_alias": "rel-new", "prod_alias": "rel-base"}], **over}
    return r


def test_golden_decided_world_is_exactly_reproduced():
    res = json.loads((GOLDEN / "results_decided.json").read_text("utf-8"))
    assert translate.build_world(res) == json.loads((GOLDEN / "world_decided.json").read_text("utf-8"))


def test_requested_state_is_waiting_for_the_human_and_nothing_downstream_moved(results):
    n = nodes(translate.build_world(decided(results, "requested")))
    assert (n["decision"]["status"], n["decision"]["reason_code"]) == ("waiting_dependency", "human_decision_pending")
    assert all(n[k]["status"] == "planned" for k in ("approve", "publish", "release"))


def test_approved_is_not_published(results):
    n = nodes(translate.build_world(decided(results, "approved_not_published")))
    assert n["approve"]["status"] == "complete" and n["decision"]["status"] == "complete"
    assert (n["publish"]["status"], n["publish"]["reason_code"]) == ("waiting_dependency", "approved_not_published")
    assert n["release"]["status"] == "planned"


def test_staging_confirmed_never_marks_prod_exposure_and_carries_receipt_refs(results):
    w = translate.build_world(decided(results))
    n = nodes(w)
    assert n["publish"]["status"] == "complete" and n["release"]["status"] == "waiting_dependency"
    assert n["release"]["reason_code"] == "staging_confirmed_prod_unchanged"
    h = w["demo"]["human"]
    assert h["release_id"] == "rel-new" and h["staging_alias"] == "rel-new" and h["prod_alias"] == "rel-base" and h["simulated_human"] is True
    assert [e["event"] for e in w["demo"]["decision_timeline"]] == ["decision_requested", "approved", "published", "staging_confirmed"]
    assert w["demo"]["decision_hook"] == "staging_confirmed" and w["decision"]["available_commands"] == []


def test_only_an_explicit_promote_completes_the_release_node(results):
    n = nodes(translate.build_world(decided(results, "promoted")))
    assert n["release"]["status"] == "complete" and n["release"]["reason_code"] == "promoted_to_prod_explicit"


def test_unconfirmed_staging_is_unknown_and_rejection_is_dead(results):
    assert nodes(translate.build_world(decided(results, "published_unconfirmed")))["publish"]["status"] == "unknown"
    n = nodes(translate.build_world(decided(results, "rejected")))
    assert n["decision"]["status"] == "dead" and n["approve"]["status"] == "planned"


def test_decision_never_applies_when_the_gates_did_not_pass(results):
    r = decided(results)
    r["core"]["attempts"][1]["native"] = None
    n = nodes(translate.build_world(r))
    assert n["decision"]["status"] == "planned" and n["approve"]["status"] == "planned"


def test_second_batch_publishes_memory_contradicts_and_starts_a_successor_investigation():
    res = json.loads((GOLDEN / "results_decided.json").read_text("utf-8"))
    w = translate.build_world(res)
    mem = {m["memory_id"]: m for m in w["memory"]}
    assert any(m["status"] == "contradicted" and "transfer_limit" in m["title"] for m in mem.values())
    assert any(m["status"] == "published" for m in mem.values())
    succ = w["runs"][translate.SUCCESSOR]
    assert succ["state"] == "running" and succ["origin"] == "observation" and "stand-in" in succ["title"].lower()
    assert nodes(w)["memory"]["status"] == "complete" and nodes(w)["memory"]["reason_code"] == "memory_published_by_stand_in"
    assert w["investigation"][translate.SUCCESSOR]["verifier"] == "pending"
    assert "address_change" in w["investigation"][translate.SUCCESSOR]["hypothesis"]


def test_memory_stays_proposed_when_staging_was_not_confirmed(results):
    w = translate.build_world(decided(results, "approved_not_published"))
    assert {m["status"] for m in w["memory"]} == {"proposed"}


def test_world_exposes_structured_hypotheses_and_gates_by_run(results):
    w = translate.build_world(results)
    hs = w["investigation"]["run-demo"]["hypotheses"]
    assert {h["verdict"] for h in hs} >= {"supported", "refuted"} and all(h["hypothesis_id"] and h["statement"] for h in hs)
    assert [h["verdict"] for h in w["investigation"]["run-demo-refuted"]["hypotheses"]] == ["refuted"]
    att = w["gates_by_run"]["run-demo"]["attempts"]
    assert w["gates_by_run"]["run-demo"]["proposal_id"] == "prop-core-2"
    assert [(a["native_pass"], a["improvement_pass"], a["reason"], a["revision_of"]) for a in att] == [(True, False, "guard_breach", None), (True, True, None, "cand-1")]


def test_successor_hypothesis_is_inconclusive_not_verified():
    w = translate.build_world(json.loads((GOLDEN / "results_decided.json").read_text("utf-8")))
    assert w["investigation"][translate.SUCCESSOR]["hypotheses"][0]["verdict"] == "inconclusive"
