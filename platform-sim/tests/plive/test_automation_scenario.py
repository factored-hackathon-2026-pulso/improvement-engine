"""SIM-ONLY automation scenario: draft dispositions and case types live in `sim_*` tables, outside the platform contract
(contract 1.2.0 has no draft event and no case type). The export is aggregates only and always labelled simulated."""

import json

from platform_live import PlatformLiveSim
from platform_live.automation_scenario import ALLOWED_KEYS, build, export

TYPES = ["cobro_indebido", "cargo_no_reconocido", "problema_app", "calidad_servicio", "atencion_sucursal", "tarjeta_virtual"]


def test_deterministic_for_a_seed():
    a, b = build(seed=7), build(seed=7)
    assert json.dumps(a.stream, sort_keys=True) == json.dumps(b.stream, sort_keys=True)
    assert json.dumps(a.manifest, sort_keys=True) == json.dumps(b.manifest, sort_keys=True)


def test_cobro_indebido_is_stage3_material_and_others_stay_lower():
    s = build(seed=7).stream
    by = {r["type_id"]: r for r in s["case_types"]}
    assert list(by) == TYPES
    drafts = by["cobro_indebido"]["drafts"]
    last = drafts[-100:]
    assert len(drafts) >= 100
    assert (last.count("as_is"), last.count("minor"), last.count("discarded")) == (61, 23, 16)
    assert by["cobro_indebido"]["repeat_q_cases"] >= 20
    assert by["cobro_indebido"]["tool_used"] / by["cobro_indebido"]["tool_applicable"] >= 0.7
    assert by["cargo_no_reconocido"]["has_agent"] is True
    assert (by["cargo_no_reconocido"]["agent_handled"], by["cargo_no_reconocido"]["agent_resolved"]) == (26, 17)
    # the others never reach the draft step
    assert by["problema_app"]["tool_used"] / by["problema_app"]["tool_applicable"] < 0.7
    assert by["calidad_servicio"]["repeat_q_cases"] < 20
    assert by["tarjeta_virtual"]["copilot_questions"] == 0
    for t in ("problema_app", "calidad_servicio", "atencion_sucursal", "tarjeta_virtual"):
        assert "drafts" not in by[t] or len(by[t]["drafts"]) < 100


def test_export_is_labelled_simulated_aggregates_only(tmp_path):
    out = tmp_path / "sim_draft_stream.json"
    export(build(seed=7), str(out))
    d = json.loads(out.read_text(encoding="utf8"))
    assert d["simulated"] is True and d["source"] == "sim_draft_stream"
    for r in d["case_types"]:
        assert set(r) <= set(ALLOWED_KEYS)
    m = json.loads((tmp_path / "sim_draft_stream.manifest.json").read_text(encoding="utf8"))
    assert m["simulated"] is True and m["outside_platform_contract"] is True and m["seed"] == 7
    assert m["planted"]["agent_proposed_type"] == "cobro_indebido"
    assert m["planted"]["con_agente_type"] == "cargo_no_reconocido"


def test_draft_dispositions_are_not_platform_events():
    r = build(seed=7)
    names = [x[0] for x in r.sim.conn.execute("select name from sqlite_master where type='table' and name like 'sim_%'")]
    assert "sim_draft_disposition" in names
    types = {x[0] for x in r.sim.conn.execute("select distinct event_type from event_log")}
    assert not any("draft" in t or "tool" in t for t in types)
    assert "copilot.query_asked" in types
    assert isinstance(r.sim, PlatformLiveSim)
