"""EVT1: SYNTHETIC platform history for the events->cells aggregator (never real data).

Backbone: the engine's own `platform-sim/platform_live` simulator (11-table platform model, real rules: one open case per customer,
language assignment, SLA, event_log with contiguous sequence). The simulator speaks contract 1.2.0; the 1.3.0 events this lane needs
(`copilot.suggestion_*`, `copilot.tool_used`, `case.type_changed`, `release` on `assistant.turn_answered`, assistant escalations) are
EMITTED HERE with the platform's published payload keys (support-platform docs/platform/api/engine-signals.md). Case types come
from the platform seed vocabulary (`cases.case_type`, the dataset's complaint subcategories). Everything is labelled
`synthetic: true`; payload shapes are assumptions of the simulator, not platform data.

Planted structure (the ground truth the live run is judged against; effects are authored here, independent of the sensor):
  copilot acceptance   : case_type service_quality is rejected far more often (draft_rejection 0.75 vs 0.40)
  heavy edits          : unrecognized_charge x phone_inbound drafts are heavily edited (0.60 vs 0.20)
  suggestion none      : app_issue x web_chat gets no suggestion 30% of the time (vs 6%)
  suggestion failed    : level risk, about 9% of requests fail (threshold 5%); not concentrated in any cell
  tool use             : undue_charge cases use consultar_cargos in 70% (vs 25%)
  case type reassign   : undue_charge cases are corrected to another type 35% of the time (vs 4%)
  assistant escalation : virtual_card cases escalate 45% (vs 10%)
  controls (no effect) : language, release rel-a vs rel-b on draft rejection, branch_service on every family
"""
from __future__ import annotations

import json
import sqlite3
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "platform-sim"))

from platform_live import PlatformLiveSim  # noqa: E402
import importlib.util  # noqa: E402

# The exporter's access policy is stdlib-only; load it by path (the package __init__ pulls non-stdlib deps this script does not need).
_spec = importlib.util.spec_from_file_location("evt1_exporter_policy", ROOT / "platform-exporter" / "src" / "platform_exporter" / "policy.py")
policy = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(policy)

CHANNELS = ["app_chat", "web_chat", "phone_inbound", "email"]
CASE_TYPES = ["unrecognized_charge", "undue_charge", "app_issue", "branch_service", "service_quality", "virtual_card"]
TYPE_WEIGHTS = [22, 20, 18, 12, 14, 14]
TOOLS = ["consultar_cargos", "estado_pqr", "buscar_politica"]
COPILOT = "copiloto-asesor@1.0.0"
ASSISTANT = "recepcion@1.0.0"


def _pick(rng, pairs):
    r, acc = rng.random(), 0.0
    for v, p in pairs:
        acc += p
        if r < acc:
            return v
    return pairs[-1][0]


def generate(seed=7, n_cases=3000, days=28):
    """Returns (events, cases, labels). `events`/`cases` are exporter-shaped dicts; `labels` documents the plant."""
    sim = PlatformLiveSim(seed=seed, n_customers=300)
    rng = sim.rng
    case_type = {}
    analysts = sim.staff_ids()
    cids = sim.customer_ids()
    step = days * 86400 / max(1, n_cases)
    for i in range(n_cases):
        sim.advance(max(1.0, rng.gauss(step, step * 0.3)))
        release = "rel-a" if i < n_cases // 2 else "rel-b"
        cust = rng.choice(cids)
        ctype = rng.choices(CASE_TYPES, TYPE_WEIGHTS)[0]
        channel = rng.choice(CHANNELS)
        assistant_first = channel in ("app_chat", "web_chat") and rng.random() < 0.55
        try:
            if assistant_first:
                cid = sim.open_case_with_assistant(cust)
                channel = sim._case(cid)["channel"]  # the simulator names chat channels chat_app/chat_web (1.1.0): aliased downstream
            else:
                cid = sim.open_case(cust, priority=rng.choice(["low", "medium", "high"]), channel=channel)
        except Exception:
            for (c,) in sim._q("SELECT id FROM cases WHERE status!='closed'").fetchall():
                sim.close_case(c, "other")
            continue
        language = sim._case(cid)["language"]
        ch_alias = {"chat_app": "app_chat", "chat_web": "web_chat"}.get(channel, channel)
        pt = language == "pt"
        # --- case type: labelled by staff (none -> type), a share corrected later
        final_type = ctype
        sim.advance(rng.randint(5, 60))
        if rng.random() < 0.9:
            sim._emit("case.type_changed", "case", cid, cid, "analyst", rng.choice(analysts), {"from": "none", "to": ctype})
        p_fix = 0.35 if ctype == "undue_charge" else 0.04
        if rng.random() < p_fix:
            final_type = rng.choice([t for t in CASE_TYPES if t != ctype])
            sim._emit("case.type_changed", "case", cid, cid, "analyst", rng.choice(analysts), {"from": ctype, "to": final_type})
        case_type[cid] = final_type
        # --- assistant path
        escalated = True
        if assistant_first:
            sim._emit("assistant.turn_answered", "assistant", sim._ast[cid], cid, "assistant", ASSISTANT,
                      {"agent": ASSISTANT, "run_id": f"run-{sim._next_seq}", "awaiting": "customer", "status": "ok", "outcome": None,
                       "trace_id": f"tr-{sim._next_seq}", "messages": 1, "release": release})
            p_esc = 0.45 if ctype == "virtual_card" else 0.10
            escalated = rng.random() < p_esc
            if escalated:
                sim.assistant_escalate(cid)
            else:
                sim.assistant_resolve(cid)
                continue
        # --- copilot on analyst-handled cases
        if rng.random() < 0.85:
            ana = rng.choice(analysts)
            sim.advance(rng.randint(5, 120))
            r = rng.random()
            sim._emit("copilot.suggestion_requested", "copilot", f"SUG-{sim._next_seq}", cid, "system", None,
                      {"analyst_id": ana, "trigger": "customer_message", "based_on_sequence": sim._next_seq})
            p_fail = 0.09
            p_none = 0.30 if (ctype == "app_issue" and ch_alias == "web_chat") else 0.06
            if r < p_fail:
                sim._emit("copilot.suggestion_failed", "copilot", f"SUG-{sim._next_seq}", cid, "system", None,
                          {"analyst_id": ana, "failure_code": rng.choice(["gateway_timeout", "model_error"])})
            elif r < p_fail + p_none * (1 - p_fail):
                sim._emit("copilot.suggestion_none", "copilot", f"SUG-{sim._next_seq}", cid, "system", None,
                          {"analyst_id": ana, "agent": COPILOT, "run_id": f"run-{sim._next_seq}", "trace_id": f"tr-{sim._next_seq}", "release": release})
            else:
                sim._emit("copilot.suggestion_ready", "copilot", f"SUG-{sim._next_seq}", cid, "system", None,
                          {"analyst_id": ana, "agent": COPILOT, "kinds": ["reply"], "count": 1, "truncated": False,
                           "run_id": f"run-{sim._next_seq}", "trace_id": f"tr-{sim._next_seq}", "release": release})
                sim.advance(rng.randint(5, 90))
                p_rej = 0.75 if ctype == "service_quality" else 0.40
                if rng.random() < p_rej:
                    dec = "discarded" if rng.random() < 0.6 else "ignored"
                    dist = None
                else:
                    heavy = 0.60 if (ctype == "unrecognized_charge" and ch_alias == "phone_inbound") else 0.20
                    dec = rng.choice(["used", "edited"])
                    dist = rng.randint(500, 950) if rng.random() < heavy else rng.randint(10, 480)
                sim._emit("copilot.suggestion_decided", "copilot", f"SUG-{sim._next_seq}", cid, "analyst", ana,
                          {"subject": "reply", "decision": dec, "edit_distance_permille": dist, "turn_id": None, "agent": COPILOT, "release": release})
            p_tool = {"consultar_cargos": 0.70 if ctype == "undue_charge" else 0.25, "estado_pqr": 0.20, "buscar_politica": 0.15}
            for tool in TOOLS:
                if rng.random() < p_tool[tool]:
                    sim._emit("copilot.tool_used", "copilot", f"TLU-{sim._next_seq}", cid, "analyst", ana, {"tool": tool})
        sim.advance(rng.randint(30, 600))
        sim.close_case(cid, "resolved")
    sim.conn.commit()
    # --- export through the exporter's allow-list (engine-level guard), exactly the columns the real exporter may read
    policy.install_sqlite_guard(sim.conn)
    ev_cols = ["sequence", "event_type", "case_id", "event_time", "payload"]
    events = []
    for seq, et, cid, t, payload in sim.conn.execute(policy.select_sql("event_log", ev_cols, order_by="sequence")):
        events.append({"sequence": seq, "event_type": et, "case_id": cid, "event_time": t, "payload": json.loads(payload or "{}")})
    cases = []
    for cid, cust, ch, lang, opened in sim.conn.execute(policy.select_sql("cases", ["id", "customer_id", "channel", "language", "opened_at"], order_by="id")):
        cases.append({"case_id": cid, "customer_id": cust, "channel": ch, "language": lang, "opened_at": opened,
                      "case_type": case_type.get(cid, "none")})
    sim.conn.close()
    labels = {"synthetic": True, "source": "platform-sim/platform_live + EVT1 scenario", "seed": seed, "n_cases": n_cases, "days": days,
              "planted_up": {"P_DRAFT_REJECT": "case_type=service_quality", "P_DRAFT_HEAVY_EDIT": "case_type=unrecognized_charge,channel=phone_inbound",
                             "P_SUGG_NONE": "case_type=app_issue,channel=web_chat", "P_TOOL_USE": "case_type=undue_charge,tool=consultar_cargos",
                             "P_TYPE_REASSIGN": "case_type=undue_charge", "P_ASSIST_ESCALATION": "case_type=virtual_card"},
              "planted_level": {"P_SUGG_FAILED": "pooled about 0.09 vs threshold 0.05"},
              "controls": ["language", "release rel-a vs rel-b on P_DRAFT_REJECT", "case_type=branch_service"]}
    return events, cases, labels


def write(out_dir, seed=7, n_cases=3000):
    events, cases, labels = generate(seed=seed, n_cases=n_cases)
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    for name, rows in (("events.ndjson", events), ("cases.ndjson", cases)):
        (out / name).write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in rows), encoding="utf-8", newline="\n")
    (out / "SYNTHETIC.json").write_text(json.dumps(labels, indent=2, sort_keys=True), encoding="utf-8", newline="\n")
    return events, cases, labels


if __name__ == "__main__":
    import argparse
    ap = argparse.ArgumentParser(description="SYNTHETIC platform history (platform-sim) for the EVT1 aggregator")
    ap.add_argument("--out", required=True)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--cases", type=int, default=3000)
    a = ap.parse_args()
    ev, cs, lb = write(a.out, a.seed, a.cases)
    print(json.dumps({"events": len(ev), "cases": len(cs), "synthetic": True}))
