"""Build cases.json (synthetic arm reports only; no real data). Run once, then gen_expected.py."""
import copy
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def run(case, status="completed", closed_early=False, cost_known=True, oracle="oracle:o@1"):
    return {"arm": "x", "case_ref": case, "status": status, "closed_early": closed_early, "cost_known": cost_known,
            "oracle_ref": oracle, "final_state_ref": "state:s@1", "effect_receipts": [], "reason": None}


def gin(judge="claude-gsipy", authors=("claude-wrld0",)):
    return {"contract_version": "engine-steps/0", "step": "gate", "run_id": "run-gate-0001", "data_class": "synthetic",
            "base_arm_report_ref": "arm_report:base@1", "candidate_arm_report_ref": "arm_report:cand@1",
            "suite_ref": "eval_suite:disputas-tarea-suite@1", "judge_actor": judge, "author_actors": list(authors)}


WA = {"world": "claude-wrld0", "suite": "agent-core-registry-demo@c814c2b"}
BAD = [run("c1", status="failed"), run("c2")]
OK = [run("c1"), run("c2")]
cases = []


def add(name, base=BAD, cand=OK, gate=None, wa=WA, reports=None):
    rep = reports if reports is not None else {"arm_report:base@1": {"runs": base}, "arm_report:cand@1": {"runs": cand}}
    cases.append({"name": name, "input": {"gate_in": gate or gin(), "reports": rep, "world_authors": wa}})


add("fx1_pass")
add("fx2_fail_improvement_equal", base=OK)
add("fx3_fail_safety_closed_early", cand=[run("c1", closed_early=True), run("c2")])
add("fx4_case_mismatch", cand=[run("c1"), run("c9")])
add("fx5_cost_unknown", cand=[run("c1", cost_known=False), run("c2")])
add("empty_reports", base=[], cand=[])
add("judge_is_author", gate=gin(judge="claude-wrld0"))
add("judge_spelling_underscore", gate=gin(judge="claude_wrld0"))
add("judge_spelling_dot", gate=gin(judge="claude.wrld0"))
add("judge_revision_suffix_of_suite", gate=gin(judge="agent-core-registry-demo"), wa={"world": "claude-wrld0", "suite": "agent-core-registry-demo@c814c2b"})
add("judge_is_suite_author", gate=gin(judge="someone-else"), wa={"world": "claude-wrld0", "suite": "someone-else"})
add("duplicate_case_refs", base=[run("c1", status="failed"), run("c2", status="failed")],
    cand=[run("c1"), run("c1"), run("c2", status="failed")])
add("cost_known_string", cand=[run("c1", cost_known="false"), run("c2")])
r = run("c2"); del r["closed_early"]
add("closed_early_missing", cand=[run("c1"), r])
add("oracle_missing", cand=[run("c1", oracle=""), run("c2")])
add("reports_missing", reports={})
add("runs_key_missing", reports={"arm_report:base@1": {}, "arm_report:cand@1": {"runs": []}})
planted = copy.deepcopy(OK)
for p in planted:
    p.update({"improvement": "pass", "planted_verdict": "pass", "lift": 9.9, "mechanism_proxy": 1})
add("planted_columns_ignored", base=OK, cand=planted)
add("safety_regression_but_more_completed", base=[run("c1"), run("c2", status="failed"), run("c3", status="failed")],
    cand=[run("c1", status="failed"), run("c2"), run("c3")])
add("invalid_gate_input_contract_version", gate={**gin(), "contract_version": "engine-steps/9"})
add("invalid_gate_input_no_authors", gate={**gin(), "author_actors": []})
(HERE / "cases.json").write_text(json.dumps(cases, indent=1) + "\n", "utf-8")
print(len(cases))
