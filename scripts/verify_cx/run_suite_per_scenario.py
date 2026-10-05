"""Evaluate each scenario of an eval suite ALONE on the LOCAL agent-core stack (one manual proposal per scenario).

Why: `evaluate` aborts the whole suite with HTTP 410 run_closed when ANY scenario sends a turn after its run closed, so
a suite-level verdict hides which scenario fails. This driver isolates scenarios and records per-scenario
pass / fail (with the scorer's failure reasons) / error (the HTTP problem code). It never calls approve, publish,
promote or reject. Token: <state-dir>/tokens.json key `admin`, sent only to the local base URL, never printed.

    uv run --with pyyaml python scripts/verify_cx/run_suite_per_scenario.py SUITE.yaml --base http://127.0.0.1:8004 \
        --state-dir .dev-stack/vx --out result.json
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import importlib.util
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve()
spec = importlib.util.spec_from_file_location("attach", HERE.parents[1] / "dev-stack" / "attach_eval_suite.py")
att = importlib.util.module_from_spec(spec)
spec.loader.exec_module(att)


def one(api, agent_id: str, suite: dict, scen: dict) -> dict:
    s1 = dict(suite)
    s1["scenarios"] = [scen]
    rec: dict = {"id": scen["id"], "expect": scen.get("expect"), "turns": sum(1 for s in scen["steps"] if s["op"] == "turn")}
    st, p = api.call("POST", "/proposals", {"agent_id": agent_id, "origin": "manual", "title": f"VX {scen['id']}"[:80]})
    if st != 201:
        return {**rec, "status": "error", "problem": f"create http {st}"}
    pid = p["proposal_id"]
    st, p = api.call("GET", f"/proposals/{pid}")
    rev = (p.get("proposal") or p).get("rev", 0)
    draft = [{"kind": "eval_suite", "content": s1, "docs": {"description": "vx single scenario", "rationale": "verification",
                                                            "changelog": "vx"}}]
    st, b = api.call("PUT", f"/proposals/{pid}/draft", {"expected_rev": rev, "changes": draft})
    if st != 200:
        return {**rec, "status": "error", "problem": f"put_draft {st} {b.get('code')}"}
    st, b = api.call("POST", f"/proposals/{pid}/validate")
    if st != 200 or not b.get("valid", True):
        return {**rec, "status": "error", "problem": f"validate {st} {b.get('violations')}"}
    st, b = api.call("POST", f"/proposals/{pid}/freeze")
    if st != 200:
        return {**rec, "status": "error", "problem": f"freeze {st}"}
    st, b = api.call("POST", f"/proposals/{pid}/evaluate", {"suite_id": suite["id"], "suite_version": suite["version"]})
    rep = b.get("payload") if st == 409 and "payload" in b else b
    if st not in (200, 409):
        return {**rec, "status": "error", "problem": f"evaluate {st} {b.get('code')}: {b.get('detail')}"}
    items = (rep or {}).get("items") or []
    res = (rep or {}).get("results") or []
    fails = []
    for r in res:
        sc = r.get("score") or {}
        if not sc.get("passed", True):
            fails.append(sc.get("failures") or [])
    gate_fail = [i["metric_id"] for i in items if not i["passed"]]
    passed = (rep or {}).get("verdict") == "pass"
    return {**rec, "status": "pass" if passed else "fail", "verdict": (rep or {}).get("verdict"),
            "failures": fails[:1], "failed_items": gate_fail}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("suite", type=Path)
    ap.add_argument("--base", default="http://127.0.0.1:8004")
    ap.add_argument("--state-dir", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--workers", type=int, default=3)
    a = ap.parse_args()
    suite = att.load_suite(a.suite)
    tok = json.loads((a.state_dir / "tokens.json").read_text(encoding="utf-8"))["admin"]
    api = att.Api(a.base, tok)
    st, ent = api.call("GET", f"/entities/agent/{suite['agent_id']}")
    agent = ent.get("content") or ent.get("spec") or ent
    suite, _ = att.with_default_thresholds(suite, agent)
    with cf.ThreadPoolExecutor(a.workers) as ex:
        rows = list(ex.map(lambda sc: one(api, suite["agent_id"], suite, sc), suite["scenarios"]))
    summ = {k: sum(1 for r in rows if r["status"] == k) for k in ("pass", "fail", "error")}
    a.out.write_text(json.dumps({"suite": suite["id"], "summary": summ, "scenarios": rows}, indent=1, ensure_ascii=False), encoding="utf-8")
    print(suite["id"], summ)
    return 0


if __name__ == "__main__":
    sys.exit(main())
