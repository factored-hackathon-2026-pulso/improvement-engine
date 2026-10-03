"""Translators: REAL results bundle (receipts, native evaluation reports, writer receipts, exporter chain verification, data-derived
analysis) -> debug-console fixture-server world (`/__fixture/state` shape, see debug-console/fixtures/world.mjs). Pure functions.

Honesty rules enforced here: a missing real result is `unknown`, never `pass`; the human step is never marked done; the second
evaluation node only completes when BOTH the (real) native verdict and the improvement judge say pass."""

from __future__ import annotations

from typing import Any

MAIN = "run-demo"
REFUTED = "run-demo-refuted"
SUCCESSOR = "run-demo-successor"
ORDER = ["signals", "scout", "verify", "opportunity", "change", "evaluate_1", "revise", "evaluate_2", "decision", "approve", "publish", "release",
         "observation", "memory"]
LABELS = {
    "signals": "Measure signal families", "scout": "Scout (SQL/wiki)", "verify": "Independent verifier", "opportunity": "Prioritise opportunity",
    "change": "Concrete change (builder + writer)", "evaluate_1": "Evaluate candidate 1", "revise": "Automatic revision",
    "evaluate_2": "Evaluate revised candidate", "decision": "Human decision (hook)", "approve": "Approve", "publish": "Publish to staging",
    "release": "Release / expose", "observation": "Exported observations", "memory": "Memory update",
}
STAGES = {"signals": "scout", "scout": "scout", "verify": "verifier", "opportunity": "hypothesis", "change": "proposal", "evaluate_1": "evaluation",
          "revise": "proposal", "evaluate_2": "evaluation", "decision": "decision", "approve": "decision", "publish": "publish", "release": "release",
          "observation": "observation", "memory": "memory"}


def trace_for(node_id: str) -> str:
    h = 7
    for c in node_id:
        h = (h * 31 + ord(c)) & 0xFFFFFFFF
    return format(h, "x").rjust(8, "0") * 4


def _node(node_id: str, label: str, stage: str, status: str, depends_on: list[str], reason: str | None = None) -> dict[str, Any]:
    return {"node_id": node_id, "label": label, "stage": stage, "status": status, "depends_on": depends_on, "reason_code": reason,
            "node_kind": "material_step", "job_ref": None, "trace_id": trace_for(node_id)}


def _ref(ident: str, digest: str, media: str = "application/json") -> dict[str, str]:
    return {"id": ident, "digest": digest, "media_type": media}


def _ev(ident: str, digest: str, relation: str, summary: str, at: str, source: str = "sandbox_evaluated", validation: str = "reproducible") -> dict[str, Any]:
    return {"evidence_ref": _ref(ident, digest), "relation": relation, "summary": summary, "source_kind": source, "validation": validation,
            "available_at": at, "level": None}


def _core_ok(step: dict[str, Any] | None) -> bool:
    return bool(step) and step.get("state") == "terminal_ok"


def _native_status(att: dict[str, Any] | None) -> tuple[str, str | None]:
    n = (att or {}).get("native")
    if not n:
        return "unknown", "no_native_report"
    return ("pass" if n["verdict"] == "pass" else "fail"), (None if n["verdict"] == "pass" else "native_gate_failed")


def _eval_node(i: int, dep: list[str], core_att: dict[str, Any] | None, an_att: dict[str, Any] | None) -> dict[str, Any]:
    nat, nat_reason = _native_status(core_att)
    imp = (an_att or {}).get("improvement", {"status": "unknown", "reason_code": "no_improvement_result"})
    nid = f"evaluate_{i}"
    if nat == "unknown" or imp["status"] == "unknown":
        status, reason = "unknown", nat_reason or imp.get("reason_code")
    elif nat == "fail":
        status, reason = "dead", nat_reason
    elif imp["status"] != "pass":
        status, reason = "dead", imp.get("reason_code") or "improvement_gate_not_pass"
    else:
        status, reason = "complete", None
    return _node(nid, LABELS[nid], "evaluation", status, dep, reason)


def _hyp_text(h: dict[str, Any]) -> str:
    return (f"{h['flow']} / {h['value']}: abandonment {h['rate']:.1%} vs {h['baseline_rate']:.1%} baseline "
            f"(n={h['n']}, excess lower bound {h['excess_lo']:.1%})")


def _diff(final_cand: dict[str, Any], core_att: dict[str, Any] | None) -> list[dict[str, str]]:
    lines = [{"op": "ctx", "text": "# business view of the change spec (stand-in builder)"}, {"op": "ctx", "text": f"retry_policy:   # scope: {final_cand['scope']}"},
             {"op": "del", "text": f"  max_retries: {final_cand['from_retries']}"}, {"op": "add", "text": f"  max_retries: {final_cand['max_retries']}"},
             {"op": "ctx", "text": f"  step: {final_cand['step']}"}, {"op": "ctx", "text": "# Core entities written by the writer (real receipts)"}]
    if core_att:
        for r in core_att.get("write_receipts", []):
            lines.append({"op": "add", "text": f"  {r['op']}: {'verified' if r.get('verified') else 'UNVERIFIED'}"})
        lines.append({"op": "ctx", "text": f"  proposal {core_att.get('proposal_id')} candidate_hash {core_att.get('candidate_hash')}"})
    return lines


def build_world(r: dict[str, Any]) -> dict[str, Any]:
    t = r["generated_at"]
    an, core, exp = r["analysis"], r["core"], r["exporter"]
    scout_h = an["scout"]["hypotheses"]
    assess = {a["key"]: a for a in an["verify"]["assessments"]}
    supported = [h for h in scout_h if assess.get(h["key"], {}).get("verdict") == "supported"]
    refuted = [h for h in scout_h if assess.get(h["key"], {}).get("verdict") == "refuted"]
    top = supported[0] if supported else None
    an_att, core_att = an["attempts"], core["attempts"]
    final = len(an_att) - 1
    two = len(an_att) > 1

    scout_core, ver_core, design_core = core.get("scout"), core.get("verifier"), core.get("design")
    receipts = exp.get("verification_receipts", [])
    chain_ok = bool(receipts) and all(x["check_result"].get("ok") for x in receipts)
    obs_status = "dead" if any(not x["check_result"].get("ok") for x in receipts) else ("complete" if chain_ok and exp.get("audit_events", 0) > 0 else "unknown")
    writer_ok = bool(core_att) and all(x.get("verified") for x in core_att[0].get("write_receipts", [])) and bool(core_att[0].get("write_receipts"))

    def st(ok: bool, bad: str = "unknown") -> str:
        return "complete" if ok else bad

    nodes = [
        _node("signals", LABELS["signals"], "scout", st(bool(an["scout"]["queries"])), []),
        _node("scout", LABELS["scout"], "scout", st(_core_ok(scout_core)), ["signals"], None if _core_ok(scout_core) else "core_scout_unknown"),
        _node("verify", LABELS["verify"], "verifier", st(_core_ok(ver_core) and bool(assess)), ["scout"], None if _core_ok(ver_core) else "core_verifier_unknown"),
        _node("opportunity", LABELS["opportunity"], "hypothesis", st(top is not None, "dead"), ["verify"], None if top else "no_supported_hypothesis"),
        _node("change", LABELS["change"], "proposal", st(_core_ok(design_core) and writer_ok), ["opportunity"], None if writer_ok and _core_ok(design_core) else ("core_design_failed" if not _core_ok(design_core) else "writer_receipts_unverified")),
        _eval_node(1, ["change"], core_att[0] if core_att else None, an_att[0] if an_att else None),
    ]
    prev = "evaluate_1"
    if two:
        nodes.append(_node("revise", LABELS["revise"], "proposal", "complete", ["evaluate_1"], "proposal_revised"))
        nodes.append(_eval_node(2, ["revise"], core_att[1] if len(core_att) > 1 else None, an_att[1]))
        prev = "evaluate_2"
    else:
        nodes.append(_node("revise", LABELS["revise"], "proposal", "planned", ["evaluate_1"], "not_needed"))
        nodes.append(_node("evaluate_2", LABELS["evaluate_2"], "evaluation", "planned", ["revise"], "not_needed"))
        prev = "evaluate_1"
    last_eval = next(n for n in nodes if n["node_id"] == prev)
    ready = last_eval["status"] == "complete"
    nodes += [
        _node("decision", LABELS["decision"], "decision", "waiting_dependency" if ready else "planned", [prev],
              "human_decision_pending" if ready else "evaluation_not_passed"),
        _node("approve", LABELS["approve"], "decision", "planned", ["decision"], "awaiting_human_authority"),
        _node("publish", LABELS["publish"], "publish", "planned", ["approve"], "awaiting_human_authority"),
        _node("release", LABELS["release"], "release", "planned", ["publish"], "awaiting_human_authority"),
        _node("observation", LABELS["observation"], "observation", obs_status, [prev], None if obs_status == "complete" else
              ("chain_verification_failed" if obs_status == "dead" else "no_exported_observations")),
        _node("memory", LABELS["memory"], "memory", "planned", ["observation"], "not_run_in_stand_in"),
    ]
    assert [n["node_id"] for n in nodes] == ORDER

    runs: dict[str, Any] = {MAIN: {"run_id": MAIN, "title": "Undirected discovery run (Core stand-in)", "state": "running" if ready else "failed", "origin": "scheduled",
                                   "revision": 0, "nodes": nodes}}
    investigation: dict[str, Any] = {}
    sq = {q["id"]: q for q in an["scout"]["queries"]}
    if top:
        a = assess[top["key"]]
        ev = [_ev("q-flow-step", sq["q-flow-step"]["rows_digest"], "supports", f"Measured through SQL: {_hyp_text(top)}", t),
              _ev(a["query_id"], next(q for q in an["verify"]["queries"] if q["id"] == a["query_id"])["rows_digest"], "supports",
                  f"Verifier (separate agent): effect positive in {a['weeks_positive']} of {a['weeks_observed']} weeks, pooled diff {a['pooled_diff']:+.3f}", t)]
        for h in refuted[:2]:
            ra = assess[h["key"]]
            ev.append(_ev(ra["query_id"], next(q for q in an["verify"]["queries"] if q["id"] == ra["query_id"])["rows_digest"], "contradicts",
                          f"Competing hypothesis {h['key']} refuted: {ra['counterevidence'][0]}", t))
        ev.append(_ev("limits-synthetic", "0" * 63 + "1", "limits", "Synthetic bank dataset; observational; mechanism_proxy only, no causal SLA claim", t,
                      source="synthetic", validation="unverified"))
        investigation[MAIN] = {"hypothesis": _hyp_text(top), "verifier": "supported", "evidence": ev}
    if refuted:
        h = refuted[0]
        ra = assess[h["key"]]
        runs[REFUTED] = {"run_id": REFUTED, "title": f"Refuted hypothesis kept visible: {h['key']}", "state": "completed", "origin": "scheduled", "revision": 0, "nodes": [
            _node("scout", LABELS["scout"], "scout", "complete", []),
            _node("hypothesis", "Form hypothesis", "hypothesis", "complete", ["scout"]),
            _node("verify", LABELS["verify"], "verifier", "complete", ["hypothesis"], "verifier_refuted"),
            _node("decision", "Do nothing", "decision", "complete", ["verify"])]}
        investigation[REFUTED] = {"hypothesis": _hyp_text(h), "verifier": "refuted", "evidence": [
            _ev("q-flow-device", sq["q-flow-device"]["rows_digest"], "supports", f"Pooled rate looked elevated: {_hyp_text(h)}", t),
            _ev(ra["query_id"], next(q for q in an["verify"]["queries"] if q["id"] == ra["query_id"])["rows_digest"], "contradicts", ra["counterevidence"][0], t)]}
    if exp.get("audit_events", 0) > 0:
        runs[SUCCESSOR] = {"run_id": SUCCESSOR, "title": "Successor investigation (planned; not started by the stand-in)", "state": "planned", "origin": "observation",
                           "revision": 0, "nodes": [_node("scout", LABELS["scout"], "scout", "planned", [], "successor_not_started_by_stand_in")]}

    events: dict[str, list[dict[str, Any]]] = {k: [] for k in runs}
    for run_id, run in runs.items():
        for n in run["nodes"]:
            if n["status"] == "planned":
                continue
            run["revision"] += 1
            events[run_id].append({"event_id": f"00000000-0000-4000-8000-{len(events[run_id]) + 1:012d}", "run_id": run_id, "sequence": len(events[run_id]) + 1,
                                   "entity_ref": {"kind": "node", "id": n["node_id"]}, "projection_revision": run["revision"], "kind": "node_status_changed"})
    # ---- gates (second attempt = the candidate that goes to the human) ----
    nat, nat_reason = _native_status(core_att[final] if core_att else None)
    imp = an_att[final]["improvement"] if an_att else {"status": "unknown", "reason_code": "no_improvement_result"}
    nat_report = (core_att[final].get("native") or {}) if core_att else {}
    if nat == "unknown" or imp["status"] == "unknown":
        comb = {"decision": "hold", "reason_code": nat_reason or imp.get("reason_code")}
    elif nat == "pass" and imp["status"] == "pass":
        comb = {"decision": "needs_human", "reason_code": "human_decision_pending"}
    else:
        comb = {"decision": "revise", "reason_code": nat_reason or imp.get("reason_code")}
    gates = {
        "native": {"status": nat, "reason_code": nat_reason, "checked_at": t, "attempt": final + 1,
                   "report_ref": _ref(nat_report["eval_run_ref"], nat_report["report_digest"]) if nat_report.get("report_digest") else None},
        "improvement": {"status": imp["status"], "reason_code": imp.get("reason_code"), "receipt_refs": [], "checked_at": t, "attempt": final + 1,
                        "lift": imp.get("lift"), "lift_lo": imp.get("lift_lo"), "exposure": imp.get("exposure"), "guard_max_exposure": imp.get("guard_max_exposure")},
        "combined": comb}
    final_cand = an_att[final]["candidate"]
    attempts = []
    for i, a in enumerate(an_att):
        nstat, nreason = _native_status(core_att[i] if i < len(core_att) else None)
        attempts.append({"attempt": i + 1, "candidate_id": a["candidate"]["id"], "revision_of": a["candidate"].get("revision_of"),
                         "scope": a["candidate"]["scope"], "max_retries": a["candidate"]["max_retries"], "native": nstat, "native_reason": nreason,
                         "improvement": a["improvement"]})
    memory = [{"memory_id": f"mem-{h['key'].replace('/', '-')}", "title": f"{h['key']}: {assess[h['key']]['verdict']} (proposed, not published)",
               "status": "proposed", "revoked": False} for h in scout_h if h["key"] in assess]
    return {
        "runs": runs, "events": events, "investigation": investigation,
        "diff": {"proposal_id": "prop-1", "lines": _diff(final_cand, core_att[final] if core_att else None)},
        "gates": gates, "memory": memory,
        "decision": {"decision_id": "dec-1", "available_commands": ["approve", "reject"], "needs_step_up": True, "stepped_up": False, "revision": 1},
        "commands": {},
        "demo": {"mode": "stand_in", "namespace": r["namespace"], "runtime_profile": "real_local_core_standin",
                 "doubles": [d["id"] for d in r["doubles"]], "doubles_detail": r["doubles"], "alternatives": an["alternatives"], "attempts": attempts,
                 "decision_hook": "pending", "core": {"scout_run": (scout_core or {}).get("core_run_id"), "attempts": [
                     {"proposal_id": a.get("proposal_id"), "candidate_hash": a.get("candidate_hash")} for a in core_att]},
                 "exporter": {"audit_events": exp.get("audit_events", 0), "chain_receipts_ok": chain_ok}},
    }


def replay_world(world: dict[str, Any]) -> dict[str, Any]:
    """Same world with every main-run node `planned` and no events, so `play.mjs` can emit the transitions live over SSE."""
    import copy
    w = copy.deepcopy(world)
    for run in w["runs"].values():
        for n in run["nodes"]:
            n["status"] = "planned"
        run["revision"] = 0
    w["events"] = {k: [] for k in w["runs"]}
    return w
