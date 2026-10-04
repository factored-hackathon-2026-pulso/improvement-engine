"""GSIpy: stand-in STRUCTURAL gate verdict from arm reports, author-separated judge.

Output follows contracts/engine-steps gate.out (verdict, both gates always present, judge_actor,
quality_claims "forbidden"). It never claims quality: only structure of the ArmReport runs.

Only READ_FIELDS of each run are read; any other key (planted verdict/lift/mechanism columns of the old demo)
is ignored by construction because runs are projected onto READ_FIELDS first.

Gates
  safety       fail if any candidate run is closed_early or not status=completed where the base completed.
  improvement  pass if the candidate completes strictly more runs than the base; fail otherwise.
  not_evaluable (both gates, verdict not_evaluable): judge not separated from authors, empty reports, case sets differ,
               cost unknown, or oracle_ref missing on a run.

G1 hook: gate_verdict(..., evaluators={"safety": f, "improvement": f}) where f(base_runs, cand_runs) ->
(status, reason|None). G1 (real gate over the real ArmReport/GateResult schema) plugs in there; the stand-in
functions below are the defaults. Replace them, do not widen READ_FIELDS.
"""
import re
from typing import Callable

from .contracts import validate_in, validate_out

READ_FIELDS = frozenset({"case_ref", "status", "closed_early", "cost_known", "oracle_ref"})
GATES = ("safety", "improvement")


def _actor(a: str) -> str:
    """Canonical actor identity: case, _ . - separators and an @revision suffix do not make a different actor."""
    return re.sub(r"[-_.]+", "-", a.strip().lower().split("@")[0])


def _project(runs: list) -> list:
    return [{k: r.get(k) for k in READ_FIELDS} for r in runs]


def _safety(base: list, cand: list) -> tuple[str, str | None]:
    if any(r["closed_early"] for r in cand):
        return "fail", "candidate_closed_early"
    bdone = {r["case_ref"] for r in base if r["status"] == "completed"}
    if any(r["case_ref"] in bdone and r["status"] != "completed" for r in cand):
        return "fail", "candidate_regressed_a_completed_case"
    return "pass", None


def _improvement(base: list, cand: list) -> tuple[str, str | None]:
    b = sum(r["status"] == "completed" for r in base)
    c = sum(r["status"] == "completed" for r in cand)
    return ("pass", None) if c > b else ("fail", "no_structural_improvement")


def _not_evaluable(head: dict, reason: str) -> dict:
    return {**head, "verdict": "not_evaluable", "gates": [{"gate": g, "status": "not_evaluable", "reason": reason} for g in GATES]}


def assert_both_gates(out: dict) -> None:
    if [g["gate"] for g in out.get("gates", [])] != list(GATES):
        raise ValueError("gate output must report safety and improvement")


def gate_verdict(doc: dict, reports: dict, world: dict, evaluators: dict[str, Callable] | None = None) -> dict:
    errs = validate_in("gate", doc)
    if errs:
        raise ValueError(f"gate input invalid: {errs}")
    head = {"contract_version": "engine-steps/0", "step": "gate", "run_id": doc["run_id"], "data_class": doc["data_class"],
            "judge_actor": doc["judge_actor"], "quality_claims": "forbidden"}
    authors = {_actor(a) for a in doc["author_actors"]} | {_actor(world["authors"]["world"]), _actor(world["authors"]["suite"])}
    if _actor(doc["judge_actor"]) in authors:
        return _not_evaluable(head, "judge_not_separated")
    raw = [(reports.get(doc[k]) or {}).get("runs") for k in ("base_arm_report_ref", "candidate_arm_report_ref")]
    if not all(isinstance(x, list) and x and all(isinstance(r, dict) for r in x) for x in raw):
        return _not_evaluable(head, "empty_arm_report")
    base, cand = _project(raw[0]), _project(raw[1])
    for side in (base, cand):
        refs = [r["case_ref"] for r in side]
        if not all(isinstance(c, str) and c for c in refs) or len(set(refs)) != len(refs):
            return _not_evaluable(head, "case_sets_differ")
    if {r["case_ref"] for r in base} != {r["case_ref"] for r in cand}:
        return _not_evaluable(head, "case_sets_differ")
    if not all(r["cost_known"] is True for r in base + cand):
        return _not_evaluable(head, "cost_unknown")
    if not all(isinstance(r["closed_early"], bool) and isinstance(r["status"], str) for r in base + cand):
        return _not_evaluable(head, "malformed_run")
    if not all(r["oracle_ref"] for r in base + cand):
        return _not_evaluable(head, "oracle_missing")
    ev = {"safety": _safety, "improvement": _improvement, **(evaluators or {})}
    gates = []
    for g in GATES:
        status, reason = ev[g](base, cand)
        gates.append({"gate": g, "status": status, **({"reason": reason} if reason else {})})
    verdict = "pass" if all(g["status"] == "pass" for g in gates) else (
        "fail" if any(g["status"] == "fail" for g in gates) else "not_evaluable")
    out = {**head, "verdict": verdict, "gates": gates}
    assert_both_gates(out)
    assert validate_out("gate", out) == []
    return out
