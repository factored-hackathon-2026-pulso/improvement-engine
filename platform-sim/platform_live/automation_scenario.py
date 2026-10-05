"""SIM-ONLY automation scenario for the "Automatizacion" screens.

Contract 1.2.0 has no draft-disposition event and no case type, so both live here in `sim_*` tables of the simulator
database (never in `event_log`, never presented as platform events). The export is aggregates only (counts and the
ordered draft dispositions of a case type, no ids, no text) and is labelled `simulated: true`.

Scenario `cobro_indebido_agent_proposed` (default): "Cobro indebido" reaches stage 3 and an agent is proposed
(84 of the last 100 drafts as is or with minor edits); "Cargo no reconocido" already runs with an agent; the rest stay at
lower stages; "Tarjeta virtual" is a new type nobody has asked the copilot about. Deterministic for a seed.
"""

from __future__ import annotations

import json
import random
from dataclasses import dataclass

from .simulator import PlatformLiveSim

SCENARIO = "cobro_indebido_agent_proposed"
ALLOWED_KEYS = ["type_id", "label", "group", "stage_since", "copilot_questions", "repeat_q_cases", "tool_applicable", "tool_used",
                "drafts", "has_agent", "agent_handled", "agent_resolved", "agent_handed", "cases_today", "copilot_cases", "cases_total"]

# type_id, label, group, cases_today, copilot_cases/cases_total (share of cases with a copilot question), spec
_TYPES = [
    ("cobro_indebido", "Cobro indebido", "Comisiones", 21),
    ("cargo_no_reconocido", "Cargo no reconocido", "Transacciones", 26),
    ("problema_app", "Problema con app", "Técnico", 19),
    ("calidad_servicio", "Calidad de servicio", "Servicio", 12),
    ("atencion_sucursal", "Atención en sucursal", "Sucursal", 9),
    ("tarjeta_virtual", "Tarjeta virtual", "Producto nuevo (ejemplo)", 2),
]


@dataclass
class Result:
    sim: PlatformLiveSim
    stream: dict
    manifest: dict


def _drafts(rng: random.Random, as_is: int, minor: int, discarded: int, extra: int = 20) -> list[str]:
    """`extra` older drafts followed by exactly the requested last-100 composition (so the window is what is measured)."""
    last = ["as_is"] * as_is + ["minor"] * minor + ["discarded"] * discarded
    rng.shuffle(last)
    older = [rng.choice(["as_is", "minor", "discarded"]) for _ in range(extra)]
    return older + last


def build(seed: int = 7, scenario: str = SCENARIO) -> Result:
    if scenario != SCENARIO:
        raise ValueError(f"unknown scenario {scenario!r}")
    sim = PlatformLiveSim(seed=seed)
    rng = random.Random(seed * 7919 + 13)
    sim.conn.execute("CREATE TABLE IF NOT EXISTS sim_draft_disposition (seq INTEGER PRIMARY KEY, type_id TEXT NOT NULL, outcome TEXT NOT NULL)")
    sim.conn.execute("CREATE TABLE IF NOT EXISTS sim_case_type (type_id TEXT PRIMARY KEY, label TEXT NOT NULL, grp TEXT NOT NULL)")

    cobro = _drafts(rng, 61, 23, 16)
    rows = {
        "cobro_indebido": dict(stage_since={"1": "2026-08-04", "2": "2026-09-02", "3": "2026-09-15"}, copilot_questions=118, repeat_q_cases=41,
                               tool_applicable=50, tool_used=40, drafts=cobro, copilot_cases=19, cases_total=21),
        "cargo_no_reconocido": dict(stage_since={"1": "2026-06-10", "2": "2026-07-01", "3": "2026-07-20", "agent": "2026-08-12"}, copilot_questions=96,
                                    repeat_q_cases=33, tool_applicable=40, tool_used=33, has_agent=True, agent_handled=26, agent_resolved=17,
                                    agent_handed=7, copilot_cases=24, cases_total=26),
        "problema_app": dict(stage_since={"1": "2026-08-20", "2": "2026-09-22"}, copilot_questions=52, repeat_q_cases=22, tool_applicable=30, tool_used=18,
                             copilot_cases=14, cases_total=19),
        "calidad_servicio": dict(stage_since={"1": "2026-09-10"}, copilot_questions=17, repeat_q_cases=9, copilot_cases=8, cases_total=12),
        "atencion_sucursal": dict(stage_since={"1": "2026-09-25"}, copilot_questions=5, repeat_q_cases=3, copilot_cases=3, cases_total=9),
        "tarjeta_virtual": dict(stage_since={}, copilot_questions=0, copilot_cases=0, cases_total=2),
    }
    case_types = []
    for tid, label, grp, today in _TYPES:
        r = dict(rows[tid])
        r.update(type_id=tid, label=label, group=grp, cases_today=today)
        sim.conn.execute("INSERT INTO sim_case_type VALUES (?,?,?)", (tid, label, grp))
        for i, o in enumerate(r.get("drafts", [])):
            sim.conn.execute("INSERT INTO sim_draft_disposition (type_id, outcome) VALUES (?,?)", (tid, o))
        case_types.append(r)

    # Platform-shaped side: a few real copilot.query_asked/answered events (ids and sizes only), labelled sim-side.
    analyst = sim.staff_ids()[0]
    for i, cus in enumerate(sim.customer_ids(simulator=False)[:6]):
        sim.advance(rng.randint(10, 90))
        cid = sim.open_case(cus, priority="medium", text="consulta")
        sim.copilot_ask(cid, analyst, case_type=_TYPES[i][0], draft_outcome=rng.choice(["sent_as_is", "minor_edits", "discarded"]))
        sim.close_case(cid, "resolved")
    sim.conn.commit()

    stream = {"source": "sim_draft_stream", "simulated": True, "scenario": scenario, "seed": seed, "as_of": "2026-10-04", "case_types": case_types}
    manifest = {
        "simulated": True, "outside_platform_contract": True, "scenario": scenario, "seed": seed, "tables": ["sim_draft_disposition", "sim_case_type"],
        "note": "Draft dispositions and case types are NOT platform events (contract 1.2.0 has neither); aggregates only.",
        "planted": {"agent_proposed_type": "cobro_indebido", "con_agente_type": "cargo_no_reconocido", "draft_accept_100": "84 of 100",
                    "lower_stage_types": ["problema_app", "calidad_servicio", "atencion_sucursal", "tarjeta_virtual"]},
    }
    return Result(sim, stream, manifest)


def export(r: Result, path: str) -> None:
    with open(path, "w", encoding="utf8") as f:
        json.dump(r.stream, f, ensure_ascii=False, indent=1, sort_keys=True)
    with open(path.replace(".json", ".manifest.json"), "w", encoding="utf8") as f:
        json.dump(r.manifest, f, ensure_ascii=False, indent=1, sort_keys=True)


if __name__ == "__main__":  # python -m platform_live.automation_scenario OUT.json [seed]
    import sys

    export(build(int(sys.argv[2]) if len(sys.argv) > 2 else 7), sys.argv[1])
