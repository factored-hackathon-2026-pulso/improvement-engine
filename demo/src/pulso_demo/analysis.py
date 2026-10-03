"""Data-derived scout / verifier / judge / reviser over the synthetic dataset. Every number comes from a recorded SQL query
(the same SQL text is what the scripted model sends through the real `pulso/lab_query` tool). The 'judge' here is a
stand-in for the Codex improvement judge (mechanism_proxy over the sandbox simulation), not the real one."""

from __future__ import annotations

import hashlib
import json
import math
import sqlite3
from typing import Any

GUARD_MAX_EXPOSURE = 0.0035  # risky sessions given extra attempts, as a share of all sessions
MIN_LIFT = 0.002
DO_NOTHING_SQL = "select sum(outcome = 'abandoned') from sessions"


def _digest(rows: Any) -> str:
    return hashlib.sha256(json.dumps(rows, sort_keys=True, default=str).encode()).hexdigest()


def run_query(conn: sqlite3.Connection, qid: str, sql: str, params: tuple[Any, ...] = ()) -> dict[str, Any]:
    cur = conn.execute(sql, params)
    cols = [c[0] for c in cur.description]
    rows = [list(r) for r in cur.fetchall()]
    return {"id": qid, "sql": sql, "params": list(params), "columns": cols, "rows": rows, "rows_digest": _digest(rows)}


def _z(p1: float, n1: int, p0: float, n0: int) -> tuple[float, float, float]:
    se = math.sqrt(p1 * (1 - p1) / max(n1, 1) + p0 * (1 - p0) / max(n0, 1)) or 1e-9
    d = p1 - p0
    return d, d - 1.96 * se, d + 1.96 * se


def scout(conn: sqlite3.Connection) -> dict[str, Any]:
    q1 = run_query(conn, "q-flow-step", "select flow, step, count(*) as n, sum(outcome = 'abandoned') as ab from sessions group by flow, step")
    q2 = run_query(conn, "q-flow-device", "select flow, device, count(*) as n, sum(outcome = 'abandoned') as ab from sessions group by flow, device")
    q0 = run_query(conn, "q-overall", "select count(*) as n, sum(outcome = 'abandoned') as ab from sessions")
    n0, ab0 = q0["rows"][0]
    p0 = ab0 / n0
    cands = []
    for q, dim in ((q1, "step"), (q2, "device")):
        for flow, key, n, ab in q["rows"]:
            if n < 150:
                continue
            p = ab / n
            d, lo, _ = _z(p, n, p0, n0)
            if lo > 0.02 and p >= 1.25 * p0:
                cands.append({"key": f"{flow}/{key}", "flow": flow, "dim": dim, "value": key, "n": n, "abandoned": ab,
                              "rate": round(p, 4), "baseline_rate": round(p0, 4), "excess": round(d, 4), "excess_lo": round(lo, 4),
                              "query_ids": [q["id"], q0["id"]]})
    cands.sort(key=lambda h: -h["excess_lo"])
    return {"queries": [q0, q1, q2], "hypotheses": cands}


def verify(conn: sqlite3.Connection, hyps: list[dict[str, Any]]) -> dict[str, Any]:
    queries, out = [], []
    for h in hyps:
        col = "step" if h["dim"] == "step" else "device"
        q = run_query(conn, f"q-verify-{h['key'].replace('/', '-')}",
                      f"select week, sum(flow = ? and {col} = ?) as n_in, sum(flow = ? and {col} = ? and outcome = 'abandoned') as ab_in, "
                      f"sum(not (flow = ? and {col} = ?)) as n_out, sum(not (flow = ? and {col} = ?) and outcome = 'abandoned') as ab_out "
                      "from sessions group by week order by week", (h["flow"], h["value"]) * 4)
        queries.append(q)
        pos, weeks, ni, ai, no, ao = 0, 0, 0, 0, 0, 0
        for _, n_in, ab_in, n_out, ab_out in q["rows"]:
            ni, ai, no, ao = ni + n_in, ai + ab_in, no + n_out, ao + ab_out
            if n_in >= 20:
                weeks += 1
                pos += int(ab_in / n_in > ab_out / max(n_out, 1) + 0.02)
        d, lo, _ = _z(ai / max(ni, 1), ni, ao / max(no, 1), no)
        consistent = weeks > 0 and pos / weeks >= 0.75
        verdict = "supported" if consistent and lo > 0 else "refuted"
        out.append({"key": h["key"], "verdict": verdict, "weeks_positive": pos, "weeks_observed": weeks, "pooled_diff": round(d, 4),
                    "pooled_lo": round(lo, 4), "query_id": q["id"],
                    "counterevidence": [] if verdict == "supported" else [f"effect present in {pos} of {weeks} weeks only; not a stable lever"],
                    "limitations": ["synthetic dataset; observational, no randomisation"]})
    return {"queries": queries, "assessments": out}


def first_candidate() -> dict[str, Any]:
    """Aggressive first draft of the builder: raise the retry limit everywhere."""
    return {"id": "cand-1", "revision_of": None, "scope": "all_flows", "max_retries": 5, "from_retries": 2, "step": "otp_verify"}


def _sim(conn: sqlite3.Connection, cand: dict[str, Any]) -> dict[str, Any]:
    flt = "" if cand["scope"] == "all_flows" else " and flow = ?"
    params: tuple[Any, ...] = () if cand["scope"] == "all_flows" else (cand["scope"].split(":", 1)[1],)
    k = cand["max_retries"] - cand["from_retries"]
    q = run_query(conn, f"q-sim-{cand['id']}",
                  "select count(*) as n, sum(retry_exhausted = 1 and extra_needed <= ?" + flt + ") as rescued, "
                  "sum(outcome = 'abandoned') as abandoned from sessions", (k, *params))
    n, rescued, abandoned = q["rows"][0]
    gq = run_query(conn, f"q-guard-{cand['id']}",
                   "select count(*) as risky_exposed from sessions where risky = 1 and retry_exhausted = 1 and extra_needed <= ?" + flt, (k, *params))
    risky = gq["rows"][0][0]
    lift = rescued / n
    se = math.sqrt(max(lift * (1 - lift), 1e-9) / n)
    return {"queries": [q, gq], "n": n, "rescued": rescued, "abandoned": abandoned, "lift": lift, "lift_lo": lift - 1.96 * se,
            "exposure": risky / n}


def judge(conn: sqlite3.Connection, cand: dict[str, Any]) -> dict[str, Any]:
    s = _sim(conn, cand)
    guard_ok = s["exposure"] <= GUARD_MAX_EXPOSURE
    imp_ok = guard_ok and s["lift_lo"] >= MIN_LIFT
    reason = "pass" if imp_ok else ("guard_breach" if not guard_ok else "insufficient_lift")
    return {"candidate": cand, "simulation": s,
            "native_proxy": {"status": "pass", "note": "structural validity (the real native evaluation is reported separately)"},
            "improvement": {"status": "pass" if imp_ok else "fail", "reason_code": None if imp_ok else reason,
                            "lift": round(s["lift"], 5), "lift_lo": round(s["lift_lo"], 5), "exposure": round(s["exposure"], 5),
                            "guard_max_exposure": GUARD_MAX_EXPOSURE}}


def revise(conn: sqlite3.Connection, cand: dict[str, Any], result: dict[str, Any]) -> dict[str, Any]:
    """Bounded automatic revision: narrow the scope to one flow and pick the candidate with the best lift that clears the guard."""
    flows = [r[0] for r in conn.execute("select distinct flow from sessions order by flow")]
    best: tuple[dict[str, Any], float] | None = None
    for flow in flows:
        for k in range(cand["from_retries"] + 1, cand["max_retries"] + 1):
            c = {**cand, "id": "cand-2", "revision_of": cand["id"], "scope": f"flow:{flow}", "max_retries": k}
            r = judge(conn, c)
            if r["improvement"]["status"] == "pass" and (best is None or r["improvement"]["lift_lo"] > best[1]):
                best = (c, r["improvement"]["lift_lo"])
    if best is None:
        raise RuntimeError("no revision within bounds clears the gates")
    return {**best[0], "reason": result["improvement"]["reason_code"]}


def alternatives(conn: sqlite3.Connection, cand: dict[str, Any]) -> list[dict[str, Any]]:
    q = run_query(conn, "q-do-nothing", DO_NOTHING_SQL)
    base = q["rows"][0][0]
    s = _sim(conn, cand)
    return [{"id": "alt-0", "kind": "do_nothing", "summary": "Keep the current retry policy", "expected_abandoned": base, "risk": "none new"},
            {"id": "alt-1", "kind": "proposed_change", "summary": f"max_retries {cand['from_retries']} -> {cand['max_retries']} ({cand['scope']})",
             "expected_abandoned": base - s["rescued"], "risk": f"guard exposure {s['exposure']:.4f}"}]
