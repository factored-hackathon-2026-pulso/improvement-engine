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


RETRY_CEILING = 5  # policy bound on max_retries (a constant of the demo, not derived)
SEGMENT_DIMS = ("device", "cohort")


class UnsupportedCandidate(RuntimeError):
    """A candidate whose flow/segment has no supported hypothesis behind it is refused (plan: designs come only from supported hypotheses)."""


class RevisionExhausted(RuntimeError):
    """The structured failure of the evaluation cannot be steered any further within the supported scope."""


def supported_scopes(scout_res: dict[str, Any], verify_res: dict[str, Any]) -> list[dict[str, Any]]:
    """Hypotheses a retry-policy candidate may rest on: verifier-SUPPORTED, measured on the otp_verify step (the step the policy governs)."""
    verdict = {a["key"]: a["verdict"] for a in verify_res["assessments"]}
    return [h for h in scout_res["hypotheses"] if verdict.get(h["key"]) == "supported" and h["dim"] == "step" and h["value"] == "otp_verify"]


def check_supported(cand: dict[str, Any], scout_res: dict[str, Any], verify_res: dict[str, Any]) -> None:
    ok = {f"flow:{h['flow']}" for h in supported_scopes(scout_res, verify_res)}
    if cand["scope"] not in ok:
        raise UnsupportedCandidate(f"scope {cand['scope']} has no supported hypothesis (supported: {sorted(ok) or 'none'})")


def design_candidate(conn: sqlite3.Connection, scout_res: dict[str, Any], verify_res: dict[str, Any]) -> dict[str, Any] | None:
    """The builder's first design: BROAD over the flow of the top supported hypothesis, limit sized by what the data shows is needed
    (max extra attempts exhausted users needed in that flow), bounded by the policy ceiling. None when nothing is supported."""
    sup = supported_scopes(scout_res, verify_res)
    if not sup:
        return None
    h = sup[0]
    q = run_query(conn, "q-design-" + h["flow"], "select max(retries_allowed), max(case when retry_exhausted = 1 then extra_needed end) from sessions where flow = ?", (h["flow"],))
    cur, extra = q["rows"][0]
    return {"id": "cand-1", "revision_of": None, "scope": f"flow:{h['flow']}", "exclude_segments": [], "step": "otp_verify", "from_retries": cur,
            "max_retries": min(RETRY_CEILING, cur + max(extra or 0, 1)), "hypothesis_key": h["key"], "derived_from": {"hypothesis": h["key"], "query": q}}


def _scope_sql(cand: dict[str, Any]) -> tuple[str, tuple[Any, ...]]:
    sql, params = "", ()
    if cand["scope"] != "all_flows":
        sql, params = " and flow = ?", (cand["scope"].split(":", 1)[1],)
    for seg in cand.get("exclude_segments", []):
        assert seg["dimension"] in SEGMENT_DIMS
        sql, params = sql + f" and {seg['dimension']} <> ?", (*params, seg["value"])
    return sql, params


def _sim(conn: sqlite3.Connection, cand: dict[str, Any]) -> dict[str, Any]:
    flt, params = _scope_sql(cand)
    k = cand["max_retries"] - cand["from_retries"]
    q = run_query(conn, f"q-sim-{cand['id']}",
                  "select count(*) as n, sum(retry_exhausted = 1 and extra_needed <= ?" + flt + ") as rescued, "
                  "sum(outcome = 'abandoned') as abandoned from sessions", (k, *params))
    n, rescued, abandoned = q["rows"][0]
    gq = run_query(conn, f"q-guard-{cand['id']}",
                   "select count(*) as risky_exposed from sessions where risky = 1 and retry_exhausted = 1 and extra_needed <= ?" + flt, (k, *params))
    risky = gq["rows"][0][0]
    queries = [q, gq]
    segments = []
    for dim in SEGMENT_DIMS:  # the guard metric broken out per segment: which segment carries the exposure
        sq = run_query(conn, f"q-guard-{dim}-{cand['id']}",
                       f"select {dim}, count(*) as risky_exposed from sessions where risky = 1 and retry_exhausted = 1 and extra_needed <= ?" + flt + f" group by {dim}",
                       (k, *params))
        queries.append(sq)
        segments += [{"dimension": dim, "value": v, "exposure": c / n} for v, c in sq["rows"]]
    lift = rescued / n
    se = math.sqrt(max(lift * (1 - lift), 1e-9) / n)
    return {"queries": queries, "n": n, "rescued": rescued, "abandoned": abandoned, "lift": lift, "lift_lo": lift - 1.96 * se,
            "exposure": risky / n, "segments": sorted(segments, key=lambda s: (-s["exposure"], s["dimension"], s["value"]))}


def judge(conn: sqlite3.Connection, cand: dict[str, Any]) -> dict[str, Any]:
    s = _sim(conn, cand)
    guard_ok = s["exposure"] <= GUARD_MAX_EXPOSURE
    imp_ok = guard_ok and s["lift_lo"] >= MIN_LIFT
    reason = "pass" if imp_ok else ("guard_breach" if not guard_ok else "insufficient_lift")
    imp: dict[str, Any] = {"status": "pass" if imp_ok else "fail", "reason_code": None if imp_ok else reason,
                           "lift": round(s["lift"], 5), "lift_lo": round(s["lift_lo"], 5), "exposure": round(s["exposure"], 5),
                           "guard_max_exposure": GUARD_MAX_EXPOSURE}
    if not guard_ok:
        # structured failure: which segment carries the breach (primary dimension = device), direction and magnitude, all measured
        segs = [{**x, "exposure": round(x["exposure"], 5)} for x in s["segments"] if x["dimension"] == SEGMENT_DIMS[0]]
        top = segs[0] if segs else None
        imp["breach"] = {"metric": "risky_exposure", "direction": "above_limit", "observed": round(s["exposure"], 5), "limit": GUARD_MAX_EXPOSURE,
                         "magnitude": round(s["exposure"] - GUARD_MAX_EXPOSURE, 5), "segments": segs,
                         "affected_segment": {"dimension": top["dimension"], "value": top["value"]} if top else None}
    return {"candidate": cand, "simulation": s,
            "native_proxy": {"status": "pass", "note": "structural validity (the real native evaluation is reported separately)"}, "improvement": imp}


def revise(conn: sqlite3.Connection, cand: dict[str, Any], result: dict[str, Any], scout_res: dict[str, Any], verify_res: dict[str, Any],
           index: int = 1) -> dict[str, Any]:
    """Bounded automatic revision STEERED by the structured failure of the previous evaluation. Only a guard_breach is steerable:
    1. exclude the breached segment from the candidate's scope (if another segment of that dimension remains), else
    2. lower the retry ceiling by one step (never to the current policy). The flow scope never changes: it stays inside the supported hypothesis.
    Anything else (insufficient lift, nothing left to narrow) raises RevisionExhausted: a bounded stop, the failure stays visible."""
    check_supported(cand, scout_res, verify_res)
    imp = result["improvement"]
    breach = imp.get("breach")
    if imp.get("reason_code") != "guard_breach" or not breach or not breach.get("affected_segment"):
        raise RevisionExhausted(f"failure {imp.get('reason_code')} is not steerable by narrowing the scope or limits")
    seg = breach["affected_segment"]
    flt, params = _scope_sql({**cand, "exclude_segments": []})
    values = {r[0] for r in conn.execute(f"select distinct {seg['dimension']} from sessions where 1 = 1{flt}", params)}
    left = values - {s["value"] for s in cand["exclude_segments"] if s["dimension"] == seg["dimension"]}
    where = (f"{breach['metric']} {breach['observed']:.4f} is {breach['magnitude']:.4f} above the {breach['limit']:.4f} limit, "
             f"carried mostly by {seg['dimension']}={seg['value']}")
    if seg["value"] in left and len(left) > 1:
        new = {**cand, "exclude_segments": [*cand["exclude_segments"], seg]}
        action, delta = "exclude_segment", [{"field": "exclude_segments", "from": cand["exclude_segments"], "to": new["exclude_segments"]}]
        rationale = f"{where}: exclude {seg['dimension']}={seg['value']} from the scope of {cand['scope']}"
    elif cand["max_retries"] - 1 > cand["from_retries"]:
        new = {**cand, "max_retries": cand["max_retries"] - 1}
        action, delta = "limit_retries", [{"field": "max_retries", "from": cand["max_retries"], "to": new["max_retries"]}]
        rationale = f"{where}: no segment left to exclude, lower the retry ceiling to bound the exposure"
    else:
        raise RevisionExhausted("breach cannot be narrowed further inside the supported hypothesis scope")
    return {**new, "id": f"cand-{index + 1}", "revision_of": cand["id"],
            "revision": {"of": cand["id"], "trigger": {"reason_code": "guard_breach", "breach": breach}, "action": action, "rationale": rationale, "delta": delta}}


def design_loop(conn: sqlite3.Connection, scout_res: dict[str, Any], verify_res: dict[str, Any], max_revisions: int = 2) -> tuple[list[dict[str, Any]], list[str]]:
    """Candidate 1 from the supported hypothesis, then up to max_revisions steered revisions while the improvement gate fails."""
    c1 = design_candidate(conn, scout_res, verify_res)
    if c1 is None:
        return [], []
    check_supported(c1, scout_res, verify_res)
    attempts, notes = [judge(conn, c1)], []
    for i in range(1, max_revisions + 1):
        if attempts[-1]["improvement"]["status"] == "pass":
            break
        try:
            c = revise(conn, attempts[-1]["candidate"], attempts[-1], scout_res, verify_res, i)
        except RuntimeError as exc:
            notes.append(f"automatic revision stopped: {exc}")
            break
        attempts.append(judge(conn, c))
    else:
        if attempts[-1]["improvement"]["status"] != "pass":
            notes.append(f"automatic revision exhausted its bound (max_revisions={max_revisions})")
    return attempts, notes


def alternatives(conn: sqlite3.Connection, cand: dict[str, Any]) -> list[dict[str, Any]]:
    q = run_query(conn, "q-do-nothing", DO_NOTHING_SQL)
    base = q["rows"][0][0]
    s = _sim(conn, cand)
    return [{"id": "alt-0", "kind": "do_nothing", "summary": "Keep the current retry policy", "expected_abandoned": base, "risk": "none new"},
            {"id": "alt-1", "kind": "proposed_change", "summary": f"max_retries {cand['from_retries']} -> {cand['max_retries']} ({cand['scope']}"
                                        + "".join(f", excl {x['dimension']}={x['value']}" for x in cand.get("exclude_segments", [])) + ")",
             "expected_abandoned": base - s["rescued"], "risk": f"guard exposure {s['exposure']:.4f}"}]


def observe(conn: sqlite3.Connection, prior_hypotheses: list[dict[str, Any]], prior_verify: dict[str, Any]) -> dict[str, Any]:
    """Step 10 (stand-in engine): a SECOND batch of observations arrives. Re-run scout/verifier on it, then compare with the memory claims
    the first round produced: a supported claim that the new data no longer shows is CONTRADICTED (rate re-measured by SQL, no causal
    attribution to the staged change), and a hypothesis the memory does not know starts a successor investigation."""
    sc = scout(conn)
    ver = verify(conn, sc["hypotheses"])
    post_keys = {h["key"] for h in sc["hypotheses"]}
    prior_keys = {h["key"] for h in prior_hypotheses}
    prior_verdict = {a["key"]: a["verdict"] for a in prior_verify["assessments"]}
    queries: list[dict[str, Any]] = []
    updates = []
    for h in prior_hypotheses:
        if prior_verdict.get(h["key"]) != "supported":
            continue
        col = "step" if h["dim"] == "step" else "device"
        q = run_query(conn, f"q-recheck-{h['key'].replace('/', '-')}",
                      f"select count(*) as n, sum(outcome = 'abandoned') as ab from sessions where flow = ? and {col} = ?", (h["flow"], h["value"]))
        queries.append(q)
        n, ab = q["rows"][0]
        after = (ab or 0) / n if n else 0.0
        updates.append({"key": h["key"], "prior_verdict": "supported", "rate_before": h["rate"], "rate_after": round(after, 4), "n_after": n,
                        "status": "still_observed" if h["key"] in post_keys else "contradicted", "query_id": q["id"],
                        "note": "re-measured on batch 2; not attributed to the staged change (prod was not exposed)"})
    assess = {a["key"]: a for a in ver["assessments"]}
    new = [{**h, "verdict": assess[h["key"]]["verdict"], "verify_query_id": assess[h["key"]]["query_id"]} for h in sc["hypotheses"] if h["key"] not in prior_keys]
    return {"scout": sc, "verify": ver, "recheck_queries": queries, "memory_updates": updates, "new_hypotheses": new,
            "successor_target": next((h for h in new if h["verdict"] == "supported"), None)}
