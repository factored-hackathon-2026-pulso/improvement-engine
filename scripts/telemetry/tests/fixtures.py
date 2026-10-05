"""Synthetic agent-run export builder (no real data). Same envelope as the T3 aggregator reads."""
from datetime import datetime, timedelta, timezone

from scripts.aggregate.agent_runs.aggregation import split_for_run


def _ts(i):
    return (datetime(2026, 9, 1, tzinfo=timezone.utc) + timedelta(minutes=i)).isoformat().replace("+00:00", "Z")


def _trace(items, cursor):
    return [{"requested_after": None, "items": items, "next_after": cursor},
            {"requested_after": cursor, "items": [], "next_after": cursor}]


def tool_event(run_id, seq, tool, status="ok", attempt=1, latency=120, call=None):
    return {"type": "tool_called", "run_id": run_id, "seq": seq, "ts": _ts(seq),
            "payload": {"node_id": "n", "tool": {"id": tool, "version": "1.0.0"}, "call_id": call or f"c{seq}",
                        "status": status, "attempt": attempt, "args": {}, "latency_ms": latency}}


def make_run(i, agent, locale, tools=(), outcome="resolved", closed_by="flow", status="closed",
             fallback=None, decisions=1, transferred=False):
    """tools: iterable of dicts {tool, status?, attempt?, latency?}. fallback: None (no decision) or depth int."""
    rid = f"run-{i:05d}"
    evs, seq = [], 0
    for _ in range(decisions if fallback is not None else 0):
        evs.append({"type": "decision_made", "run_id": rid, "seq": seq, "ts": _ts(i),
                    "payload": {"fallback_depth": fallback, "latency_ms": 300, "tokens": {}, "cost_usd": 0.0}})
        seq += 1
    for t in tools:
        evs.append(tool_event(rid, seq, t["tool"], t.get("status", "ok"), t.get("attempt", 1), t.get("latency", 120)))
        seq += 1
    if transferred:
        evs.append({"type": "run_transferred", "run_id": rid, "seq": seq, "ts": _ts(i), "payload": {}})
        seq += 1
    run = {"run_id": rid, "cursor": i + 1, "status": status, "created_at": _ts(i),
           "agent": {"id": agent, "version": "1.0.0"}, "locale": locale, "release": "r"}
    if status != "open":
        run.update(outcome=outcome, closed_at=_ts(i + 1))
        evs.append({"type": "run_closed", "run_id": rid, "seq": seq, "ts": _ts(i + 1),
                    "payload": {"outcome": outcome, "closed_by": closed_by}})
    return run, evs


def make_export(specs, label="SYNTHETIC test"):
    runs, events = [], {}
    for run, evs in specs:
        runs.append(run)
        events[run["run_id"]] = _trace(evs, len(evs))
    return {"_label": label, "runs": {"pages": _trace(runs, "rc")}, "events": events}


def balanced(n_per_half, build):
    """Call build(i) for run indexes until each split half has n_per_half runs; returns specs list."""
    out, counts, i = [], {"discovery": 0, "holdout": 0}, 0
    while min(counts.values()) < n_per_half:
        half = split_for_run(f"run-{i:05d}")
        if counts[half] < n_per_half:
            out.append(build(i, half))
            counts[half] += 1
        i += 1
    return out
