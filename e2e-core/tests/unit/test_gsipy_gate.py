"""GSIpy RED/GREEN: stand-in structural gate verdict from arm reports with an author-separated judge."""
import copy
from pathlib import Path

import pytest

from claude_standin import compile_step as C
from claude_standin import gate_step as G
from claude_standin.contracts import validate_out

ROOT = Path(__file__).resolve().parents[3]
WORLD = C.load_world(ROOT / "agent-core-assets" / "worlds" / "seeded-base.world.yaml")


def run(case, status="completed", closed_early=False, cost_known=True, oracle="oracle:o@1"):
    return {"arm": "x", "case_ref": case, "status": status, "closed_early": closed_early, "cost_known": cost_known,
            "oracle_ref": oracle, "final_state_ref": "state:s@1", "effect_receipts": [], "reason": None}


def reports(base, cand):
    return {"arm_report:base@1": {"runs": base}, "arm_report:cand@1": {"runs": cand}}


def gin(judge="claude-gsipy", authors=("claude-wrld0",)):
    return {"contract_version": "engine-steps/0", "step": "gate", "run_id": "run-gate-0001", "data_class": "synthetic",
            "base_arm_report_ref": "arm_report:base@1", "candidate_arm_report_ref": "arm_report:cand@1",
            "suite_ref": "eval_suite:disputas-suite@1", "judge_actor": judge, "author_actors": list(authors)}


BASE_BAD = [run("c1", status="failed"), run("c2")]
CAND_OK = [run("c1"), run("c2")]


def verdict(base=BASE_BAD, cand=CAND_OK, **kw):
    return G.gate_verdict(gin(**kw), reports(base, cand), WORLD)


def test_fx1_pass_when_candidate_completes_more():
    o = verdict()
    assert o["verdict"] == "pass" and [g["gate"] for g in o["gates"]] == ["safety", "improvement"]
    assert validate_out("gate", o) == []


def test_fx2_fail_improvement_when_equal():
    o = verdict(base=CAND_OK, cand=CAND_OK)
    assert o["verdict"] == "fail" and {g["gate"]: g["status"] for g in o["gates"]} == {"safety": "pass", "improvement": "fail"}


def test_fx3_fail_safety_when_candidate_closed_early():
    o = verdict(cand=[run("c1", closed_early=True), run("c2")])
    assert o["verdict"] == "fail" and o["gates"][0] == {"gate": "safety", "status": "fail", "reason": "candidate_closed_early"}


def test_fx4_not_evaluable_on_case_mismatch():
    o = verdict(cand=[run("c1"), run("c9")])
    assert o["verdict"] == "not_evaluable" and all(g["status"] == "not_evaluable" for g in o["gates"])


def test_fx5_not_evaluable_on_unknown_cost_or_empty():
    assert verdict(cand=[run("c1", cost_known=False), run("c2")])["verdict"] == "not_evaluable"
    assert verdict(base=[], cand=[])["verdict"] == "not_evaluable"


def test_both_gates_always_reported_and_skipping_one_is_rejected():
    o = verdict()
    broken = copy.deepcopy(o)
    broken["gates"] = broken["gates"][:1]
    assert validate_out("gate", broken)
    with pytest.raises(ValueError):
        G.assert_both_gates(broken)
    G.assert_both_gates(o)


def test_judge_must_differ_from_input_authors():
    o = verdict(judge="claude-wrld0")
    assert o["verdict"] == "not_evaluable" and o["gates"][0]["reason"] == "judge_not_separated"


def test_judge_equal_to_world_suite_or_world_author_is_refused():
    w = copy.deepcopy(WORLD)
    w["authors"]["suite"] = "someone-else"
    o = G.gate_verdict(gin(judge="someone-else"), reports(BASE_BAD, CAND_OK), w)
    assert o["verdict"] == "not_evaluable"
    assert verdict(judge="claude-wrld0")["verdict"] == "not_evaluable"


def test_quality_claims_forbidden_and_judge_echoed():
    o = verdict()
    assert o["quality_claims"] == "forbidden" and o["judge_actor"] == "claude-gsipy"


def test_planted_columns_are_not_read():
    planted = copy.deepcopy(CAND_OK)
    for r in planted:
        r.update({"improvement": "pass", "planted_verdict": "pass", "lift": 9.9, "mechanism_proxy": 1})
    o = verdict(base=CAND_OK, cand=planted)
    assert o["verdict"] == "fail"  # identical structure: planted fields change nothing
    assert G.READ_FIELDS == frozenset({"case_ref", "status", "closed_early", "cost_known", "oracle_ref"})


def test_g1_hook_overrides_a_gate():
    seen = {}

    def g1_improvement(base, cand):
        seen["called"] = True
        return ("fail", "g1_says_no")

    o = G.gate_verdict(gin(), reports(BASE_BAD, CAND_OK), WORLD, evaluators={"improvement": g1_improvement})
    assert seen and o["verdict"] == "fail" and o["gates"][1]["reason"] == "g1_says_no"
