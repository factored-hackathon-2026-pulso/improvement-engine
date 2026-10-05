"""TEL1: agent-behaviour signals over Agent Core RUN exports (aggregates only, k = 10).

Thin adapter over the X-DOC-owned T3 reader (`scripts/aggregate/agent_runs`): it REUSES that module's strict export
traversal, dedup, terminal-event, handoff and split logic and only adds what T3 deliberately does not publish
(agent allow-list beyond `builder`, tool label, repeated/fallback/latency/steps facts) under a separate metric
namespace A1..A9 (`evidence_class: agent_runs`, never pooled with bank M1..M10 nor with T3's `AG_*`).

Metrics (binary run-level proportions, one run contributes at most once; period = UTC month of `created_at`;
half = T3 SHA-256 split of the private run id). "valid,pos" masking: a row ships only when
denominator >= k, and numerator and complement are each 0 or >= k.

  A1 handoff_rate          terminal runs with run_transferred | status escalated | closed_by escalation/transfer  / terminal runs      [agent, locale]
  A2 fallback_rate         runs with a decision_made whose fallback_depth > 0 / runs with >= 1 decision_made                          [agent, locale]
  A3 closed_early_rate     terminal runs with outcome abstained|clarify_exhausted|abandoned|cancelled / terminal runs                 [agent, locale]
  A4 tool_error_run_rate   runs with a tool_called status error|timeout|denied / runs with >= 1 tool_called                          [agent, locale]
  A5 tool_share            runs that call tool X / runs with >= 1 tool_called of that agent                                           [agent, tool]
  A6 repeated_tool_rate    runs calling one tool (distinct call_id) >= 3 times / runs with >= 1 tool_called                           [agent, locale]
  A7 retry_run_rate        runs with a tool_called attempt > 1 / runs with >= 1 tool_called                                           [agent, locale]
  A8 slow_tool_run_rate    runs whose slowest tool_called has latency_ms >= 2000 / runs with >= 1 tool_called                         [agent, locale]
  A9 long_run_rate         runs with >= 5 steps (distinct tool calls + decisions) / all runs                                          [agent, locale]

Latency percentiles and steps per run are DESCRIPTIVE (bucketed, per agent, withheld below k runs): the six-field
cell schema is binary. Findings are associations, never causes.
"""
from __future__ import annotations

import json
from collections import Counter, defaultdict
from typing import Any, Mapping

from scripts.aggregate.agent_runs.aggregation import (
    DEFAULT_K,
    ExportContractError,
    _AGENT_ID,
    _AGENT_VERSION,
    _LOCALE_TAG,
    _evidence_class,
    _events_for_run,
    _is_handoff,
    _selected_runs,
    _terminal_event,
    _utc_instant,
    split_for_run,
    write_immutable_bytes,
)
from scripts.aggregate.agent_runs.cell_table import render_cell_ndjson

__all__ = ["ExportContractError", "aggregate_agent_signals", "level_findings", "render_ndjson", "write_immutable_bytes"]

PROTOCOL = "agent-signals.v1"
EVIDENCE_CLASS = "agent_runs"
# Versioned allow-lists of registry ids -> safe labels; anything else is coarsened to "other" and never echoed.
AGENT_LABELS = frozenset({"disputas", "consultas", "recepcion", "copiloto-asesor"})
AGENT_ALIASES = {"pulso-builder": "builder"}
TOOL_LABELS = frozenset({
    "buscar_transacciones", "convertir_moneda", "directory/list", "leer_movimientos", "leer_pqr_cliente",
    "leer_productos", "leer_transcript", "obtener_handoff", "obtener_pqr", "radicar_pqr", "seleccionar",
    "registry/create_proposal", "registry/evaluate", "registry/freeze", "registry/get_entity", "registry/get_proposal",
    "registry/get_write", "registry/list_versions", "registry/put_draft", "registry/reopen", "registry/validate"})
LOCALES = frozenset({"es", "pt"})
ERROR_STATUSES = frozenset({"error", "timeout", "denied"})
CLOSED_EARLY = frozenset({"abstained", "clarify_exhausted", "abandoned", "cancelled"})
REPEAT_MIN, SLOW_MS, LONG_STEPS = 3, 2000, 5
LEVEL_SHARE, LEVEL_MIN_DEN, PEER_SHARE_MAX = 0.95, 30, 0.8
LATENCY_EDGES = (50, 100, 250, 500, 1000, 2000, 5000)
METRIC_NAMES = {
    "A1": "handoff_rate", "A2": "fallback_rate", "A3": "closed_early_rate", "A4": "tool_error_run_rate",
    "A5": "tool_share", "A6": "repeated_tool_rate", "A7": "retry_run_rate", "A8": "slow_tool_run_rate",
    "A9": "long_run_rate"}


def _dims(run: Mapping[str, Any]) -> tuple[str, str]:
    agent = run.get("agent")
    aid = agent.get("id") if isinstance(agent, Mapping) else None
    ver = agent.get("version") if isinstance(agent, Mapping) else None
    if not isinstance(aid, str) or not _AGENT_ID.fullmatch(aid) or not isinstance(ver, str) or not _AGENT_VERSION.fullmatch(ver):
        raise ExportContractError("invalid run agent reference")
    label = aid if aid in AGENT_LABELS else AGENT_ALIASES.get(aid, "other")
    locale = run.get("locale")
    if not isinstance(locale, str) or len(locale) > 35 or not _LOCALE_TAG.fullmatch(locale):
        raise ExportContractError("missing or invalid run locale")
    lang = locale.split("-", 1)[0]
    return label, lang if lang in LOCALES else "other"


def _tool_label(payload: Mapping[str, Any]) -> str:
    tool = payload.get("tool")
    tid = tool.get("id") if isinstance(tool, Mapping) else None
    return tid if isinstance(tid, str) and tid in TOOL_LABELS else "other"


def _ms(value: object) -> int:
    return value if isinstance(value, int) and not isinstance(value, bool) and value >= 0 else 0


def _facts(export: Mapping[str, Any]) -> list[dict[str, Any]]:
    pages = export.get("events")
    if not isinstance(pages, Mapping):
        raise ExportContractError("invalid event pages")
    out = []
    for run in _selected_runs(export):
        rid = run["run_id"]
        status = run.get("status")
        if status not in {"open", "closed", "escalated"}:
            raise ExportContractError("unknown run status")
        events = _events_for_run(pages, rid)
        agent, locale = _dims(run)
        created = _utc_instant(run.get("created_at"), field="run creation time")
        f: dict[str, Any] = {"agent": agent, "locale": locale, "period": created.strftime("%Y-%m"),
                             "half": split_for_run(rid), "terminal": status != "open"}
        calls: dict[str, set] = defaultdict(set)
        lat, attempts, statuses = [], [], []
        decisions = fallback = 0
        for e in events:
            p = e.get("payload") if isinstance(e.get("payload"), Mapping) else {}
            if e["type"] == "tool_called":
                cid = p.get("call_id")
                calls[_tool_label(p)].add(cid if isinstance(cid, str) else f"seq{e['seq']}")
                lat.append(_ms(p.get("latency_ms")))
                attempts.append(p.get("attempt", 1))
                statuses.append(p["status"])
            elif e["type"] == "decision_made":
                decisions += 1
                fd = p.get("fallback_depth")
                fallback += int(isinstance(fd, int) and not isinstance(fd, bool) and fd > 0)
        f.update(tools=sorted(calls), has_tool=bool(lat), tool_error=any(s in ERROR_STATUSES for s in statuses),
                 retry=any(a > 1 for a in attempts), repeated=any(len(v) >= REPEAT_MIN for v in calls.values()),
                 max_latency=max(lat, default=0), latencies=lat, has_decision=decisions > 0, fallback=fallback > 0,
                 steps=sum(len(v) for v in calls.values()) + decisions)
        if f["terminal"]:
            outcome, _, closed_by = _terminal_event(events)
            f["handoff"] = _is_handoff(status, events, closed_by)
            f["closed_early"] = outcome in CLOSED_EARLY
        out.append(f)
    return out


def k_ok(num: int, den: int, k: int = DEFAULT_K) -> bool:
    return den >= k and (num == 0 or num >= k) and (den - num == 0 or den - num >= k)


def _bucket(value: int) -> str:
    for edge in LATENCY_EDGES:
        if value <= edge:
            return f"<={edge}"
    return f">{LATENCY_EDGES[-1]}"


def _pct(values: list[int], q: float) -> int:
    ordered = sorted(values)
    return ordered[max(0, -(-int(q * 1000) * len(ordered) // 1000) - 1)]  # nearest rank


def aggregate_agent_signals(export: Mapping[str, Any], k: int = DEFAULT_K) -> dict[str, Any]:
    if not isinstance(export, Mapping):
        raise ExportContractError("invalid export")
    label_class = _evidence_class(export.get("_label"))
    facts = _facts(export)
    acc: dict[tuple, list[int]] = defaultdict(lambda: [0, 0])

    def add(metric, dims, f, num, den=1):
        c = acc[(metric, tuple(sorted(dims.items())), f["half"], f["period"])]
        c[0] += int(num)
        c[1] += den

    for f in facts:
        d = {"agent": f["agent"], "locale": f["locale"]}
        if f["terminal"]:
            add("A1", d, f, f["handoff"])
            add("A3", d, f, f["closed_early"])
        if f["has_decision"]:
            add("A2", d, f, f["fallback"])
        if f["has_tool"]:
            add("A4", d, f, f["tool_error"])
            add("A6", d, f, f["repeated"])
            add("A7", d, f, f["retry"])
            add("A8", d, f, f["max_latency"] >= SLOW_MS)
            for t in f["tools"]:
                add("A5", {"agent": f["agent"], "tool": t}, f, 1, 0)  # denominator filled below
        add("A9", d, f, f["steps"] >= LONG_STEPS)
    # A5 denominators: tool-calling runs of the agent (a run counts once per tool in the numerator).
    tool_runs: Counter = Counter((f["agent"], f["half"], f["period"]) for f in facts if f["has_tool"])
    for key in [x for x in acc if x[0] == "A5"]:
        acc[key][1] = tool_runs[(dict(key[1])["agent"], key[2], key[3])]

    cells, suppressed = [], 0
    for (metric, dims, half, period), (num, den) in sorted(acc.items()):
        if not k_ok(num, den, k):
            suppressed += 1
            continue
        cells.append({"metric": metric, "dims": dict(dims), "half": half, "period": period,
                      "numerator": num, "denominator": den})

    descriptive = []
    by_agent: dict[str, list] = defaultdict(list)
    for f in facts:
        by_agent[f["agent"]].append(f)
    for agent, fs in sorted(by_agent.items()):
        lats = [x for f in fs for x in f["latencies"]]
        if len(fs) < k or len(lats) < k:
            continue
        descriptive.append({
            "agent": agent, "runs": len(fs), "tool_events": len(lats),
            "tool_latency_ms_p50_bucket": _bucket(_pct(lats, 0.5)),
            "tool_latency_ms_p90_bucket": _bucket(_pct(lats, 0.9)),
            "steps_per_run_p50": _pct([f["steps"] for f in fs], 0.5),
            "steps_per_run_p90": _pct([f["steps"] for f in fs], 0.9)})
    cells.sort(key=lambda r: (r["period"], r["half"], r["metric"], json.dumps(r["dims"], sort_keys=True)))
    return {
        "protocol": PROTOCOL, "evidence_class": EVIDENCE_CLASS, "source_label_class": label_class,
        "k": k, "metrics": METRIC_NAMES, "runs_read": len(facts), "suppressed_cells": suppressed,
        "availability": "published" if cells else "below_privacy_floor" if suppressed else "no_eligible_runs",
        "cells": cells, "descriptive": descriptive}


def render_ndjson(cells: list[dict[str, Any]]) -> str:
    return render_cell_ndjson(cells)


def level_findings(cells: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """LEVEL findings from the already-masked A5 cells (no new disclosure): an agent whose tool-calling runs call the
    same tool in >= 95% of cases, in BOTH halves, with >= 30 tool-calling runs per half. `peers_vary` is true when
    some other agent (>= k runs in both halves) calls that tool in <= 80% of its tool-calling runs."""
    share: dict[tuple, dict[str, list[int]]] = defaultdict(lambda: {"discovery": [0, 0], "holdout": [0, 0]})
    for c in cells:
        if c["metric"] == "A5":
            s = share[(c["dims"]["agent"], c["dims"]["tool"])][c["half"]]
            s[0] += c["numerator"]
            s[1] += c["denominator"]
    out = []
    for (agent, tool), halves in sorted(share.items()):
        if any(h[1] < LEVEL_MIN_DEN or h[0] / h[1] < LEVEL_SHARE for h in halves.values()):
            continue
        peers = [h for (a, t), h in share.items() if t == tool and a != agent
                 and all(x[1] >= DEFAULT_K for x in h.values())]
        out.append({
            "kind": "always_same_tool", "evidence_class": EVIDENCE_CLASS, "claim": "association",
            "agent": agent, "tool": tool,
            "discovery": {"numerator": halves["discovery"][0], "denominator": halves["discovery"][1]},
            "holdout": {"numerator": halves["holdout"][0], "denominator": halves["holdout"][1]},
            "threshold_share": LEVEL_SHARE, "replicated_in_both_halves": True,
            "peers_compared": len(peers),
            "peers_vary": any(sum(x[0] for x in h.values()) / sum(x[1] for x in h.values()) <= PEER_SHARE_MAX
                              for h in peers)})
    return out
