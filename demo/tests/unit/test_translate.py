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
