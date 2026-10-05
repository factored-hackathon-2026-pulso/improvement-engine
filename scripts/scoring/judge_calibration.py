#!/usr/bin/env python3
"""Calibration of a rubric judge against a golden set (human or synthetic scores). Standard library only.

Golden JSON: {"proposals": [{"id", "proposal", "base", "expected": {"R1": 0|1|2, ...}}]}.
Each proposal goes through `run_judge` (family guard, two samples, min, escalation). Reports per criterion and
overall: exact agreement, within-1 agreement, hard-gate agreement (a criterion scored 0 is a rejection gate:
judge-zero == expected-zero), and escalations. Escalated items are excluded from agreement and listed.

`--live` is opt-in and needs the local stack (PULSO_LLM_GATEWAY_ADDR and key in the environment); otherwise
the report says `not_exercised`. `--limit N` judges only the first N proposals (live smoke).
"""
import argparse
import importlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from score_proposal import JUDGED, JudgeError, model_family, run_judge  # noqa: E402

DEFAULT_GOLDEN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "golden", "synthetic_golden_12.json")


def calibrate(golden, judge, builder_model, judge_model, limit=None):
    rows, esc, denied = [], [], []
    fb, fj = model_family(builder_model), model_family(judge_model)
    if fb is None or fj is None or fb == fj:  # configuration errors stay fatal; only per-proposal judge failures are denials
        raise JudgeError("judge/Builder families unknown or equal; refusing to calibrate")
    items = golden["proposals"][:limit] if limit else golden["proposals"]
    for it in items:
        req = {"proposal": it["proposal"], "base": it.get("base", {}), "rubric_criteria": list(JUDGED)}
        try:
            res = run_judge(judge, req, builder_model, judge_model)
        except JudgeError as e:
            if "unreachable" in str(e):  # infrastructure down: abort, do not report it as 'denied' proposals
                raise
            # fail-closed per proposal (e.g. model output never valid): counted, not scored
            denied.append({"id": it["id"], "reason": str(e)[:160]})
            continue
        if res["escalate_human"]:
            esc.append({"id": it["id"], "criteria": res["escalate_human"]})
        for c, got in res["scores"].items():
            if c in it["expected"] and c not in res["escalate_human"]:
                rows.append({"id": it["id"], "criterion": c, "expected": it["expected"][c], "judged": got})
    out = summarize(rows, esc, len(items))
    out["denied"] = denied
    return out


def _rate(rs, f):
    return round(sum(1 for r in rs if f(r)) / len(rs), 4) if rs else None


def summarize(rows, escalated, n_proposals):
    exact = lambda r: r["expected"] == r["judged"]
    within1 = lambda r: abs(r["expected"] - r["judged"]) <= 1
    gate = lambda r: (r["expected"] == 0) == (r["judged"] == 0)
    per = {c: {"n": len([r for r in rows if r["criterion"] == c]),
               "exact": _rate([r for r in rows if r["criterion"] == c], exact),
               "within_1": _rate([r for r in rows if r["criterion"] == c], within1),
               "hard_gate": _rate([r for r in rows if r["criterion"] == c], gate)} for c in JUDGED}
    # proposal-level hard-gate: does the judge agree on "any judged criterion is 0"
    by = {}
    for r in rows:
        d = by.setdefault(r["id"], [False, False])
        d[0] |= r["expected"] == 0
        d[1] |= r["judged"] == 0
    return {"status": "exercised", "proposals": n_proposals, "pairs": len(rows),
            "exact": _rate(rows, exact), "within_1": _rate(rows, within1), "hard_gate": _rate(rows, gate),
            "proposal_gate_agreement": _rate(list(by.values()), lambda d: d[0] == d[1]),
            "per_criterion": per, "escalated_to_human": escalated, "mismatches": [r for r in rows if not within1(r)]}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--golden", default=DEFAULT_GOLDEN)
    ap.add_argument("--judge", default="judges.gateway_judge:judge")
    ap.add_argument("--builder-model", default=os.environ.get("PULSO_JUDGE_BUILDER_MODEL", "xiaomi/mimo-v2.6-flash"))
    ap.add_argument("--judge-model", default=os.environ.get("PULSO_JUDGE_MODEL", "z-ai/glm-5.3-flash"))
    ap.add_argument("--live", action="store_true", help="opt-in: call the local gateway")
    ap.add_argument("--limit", type=int)
    ap.add_argument("--out")
    a = ap.parse_args(argv)
    try:
        with open(a.golden, encoding="utf-8") as f:
            golden = json.load(f)
        if not a.live:
            print("judge_calibration: pass --live to call the gateway; status not_exercised", file=sys.stderr)
            result = {"status": "not_exercised", "reason": "--live not given"}
        elif not os.environ.get("PULSO_LLM_GATEWAY_ADDR") or not (
                os.environ.get("PULSO_LLM_GATEWAY_KEY") or os.environ.get("GATEWAY_TOKEN_AGENT_CORE")):
            result = {"status": "not_exercised", "reason": "gateway env not set"}
        else:
            mod, _, fn = a.judge.partition(":")
            judge = getattr(importlib.import_module(mod), fn)
            result = calibrate(golden, judge, a.builder_model, a.judge_model, a.limit)
            result.update(judge_model=a.judge_model, builder_model=a.builder_model)
    except JudgeError as e:
        result = {"status": "not_exercised", "reason": f"judge refused: {e}"}
    except (OSError, ValueError, ImportError, AttributeError) as e:
        print(f"judge_calibration: {type(e).__name__}: {e}", file=sys.stderr)
        return 2
    text = json.dumps(result, indent=2, sort_keys=True)
    if a.out:
        with open(a.out, "w", encoding="utf-8") as f:
            f.write(text + "\n")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
