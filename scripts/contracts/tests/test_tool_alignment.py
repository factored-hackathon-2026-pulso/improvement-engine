"""SIG1: tool definitions drift between agent-core's registry-e2e fixtures and tool-service `GET /v1/tools`.

The provider (tool-service) is the oracle for the tools it serves. The checker compares the ToolDef fixtures with the provider's
listing shape (`ToolList{tools:[ToolInfo]}`), reports typed findings, and fails on any finding that is not in the recorded
baseline (known upstream drift, each with its ask) or when a baseline entry is no longer true (fixed upstream: remove it).
"""
from __future__ import annotations

import copy
import json
import subprocess
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
TA = HERE.parent / "tool_alignment"
sys.path.insert(0, str(TA))
import check_tool_alignment as ta  # noqa: E402


def info(tid, source="customer_products", risk="read", auth="session", idem=True, props=None, required=None, version="1.0.0"):
    schema = {"type": "object", "additionalProperties": False, "properties": props or {}}
    if required:
        schema["required"] = required
    return {"id": tid, "version": version, "risk_class": risk, "min_auth_level": auth, "source": source, "idempotent": idem,
            "args_schema": schema}


def kinds(report):
    return sorted((f["tool"], f["kind"]) for f in report["findings"])


def test_identical_definitions_have_no_findings():
    t = info("leer_productos", props={"limite": {"type": "integer", "default": 20}})
    r = ta.compare([copy.deepcopy(t)], {"tools": [copy.deepcopy(t)]})
    assert r["findings"] == [] and r["checked"] == 1


def test_source_drift_is_typed_missing_or_mismatch():
    prov = {"tools": [info("a", source="customer_products"), info("b", source="customer_cases"), info("c", source="customer_cases")]}
    fx = [info("a", source="productos"), info("b", source=None), info("c", source="customer_cases")]
    assert kinds(ta.compare(fx, prov)) == [("a", "source_mismatch"), ("b", "source_missing")]
    f = {x["tool"]: x for x in ta.compare(fx, prov)["findings"]}
    assert (f["a"]["fixture"], f["a"]["provider"]) == ("productos", "customer_products")


def test_scalar_fields_and_args_schema_drift():
    prov = {"tools": [info("t", props={"limite": {"type": "integer", "default": 20}, "product_id": {"type": "string"}}, required=["product_id"])]}
    fx = [info("t", risk="write_reversible", auth="step_up", idem=False, version="2.0.0", props={"limite": {"type": "integer", "default": 10}, "extra": {"type": "string"}})]
    assert kinds(ta.compare(fx, prov)) == [
        ("t", "args_property_changed"), ("t", "args_property_extra"), ("t", "args_property_missing"), ("t", "args_required_mismatch"),
        ("t", "idempotent_mismatch"), ("t", "min_auth_level_mismatch"), ("t", "risk_class_mismatch"), ("t", "version_mismatch")]


def test_tools_on_one_side_only():
    prov = {"tools": [info("served_only"), info("both")]}
    fx = [info("both"), info("local_compute", source=None, risk="compute"), info("ghost", source="x")]
    r = ta.compare(fx, prov)
    assert kinds(r) == [("ghost", "not_in_provider"), ("served_only", "not_in_fixture")]
    assert r["agent_owned"] == ["local_compute"], "compute tools are the runtime's own: not a finding"


def test_baseline_gates_new_drift_and_flags_stale_entries():
    prov = {"tools": [info("a", source="customer_products")]}
    fx = [info("a", source="productos")]
    r = ta.compare(fx, prov)
    assert ta.gate(r, {"a:source_mismatch": "ask A1"})["ok"] is True
    g = ta.gate(r, {})
    assert g["ok"] is False and g["new"] == ["a:source_mismatch"]
    fixed = ta.compare([info("a", source="customer_products")], prov)
    g = ta.gate(fixed, {"a:source_mismatch": "ask A1"})
    assert g["ok"] is False and g["stale"] == ["a:source_mismatch"], "fixed upstream: the baseline must shrink"


def test_align_takes_the_providers_view_and_keeps_fixture_prose_and_marks_provenance():
    fx = [{**info("leer_productos", source="productos"), "description": "texto del fixture"},
          {**info("convertir_moneda", source=None, risk="compute"), "description": "local"}]
    prov = {"tools": [info("leer_productos", props={"limite": {"type": "integer", "default": 20}}), info("leer_perfil", source="customer_profile")]}
    out = ta.align(fx, prov)
    d = out["tool_defs"]
    assert d["leer_productos"]["source"] == "customer_products" and d["leer_productos"]["description"] == "texto del fixture"
    assert d["leer_productos"]["args_schema"]["properties"] == {"limite": {"type": "integer", "default": 20}}
    assert d["convertir_moneda"]["source"] is None and out["provenance"]["convertir_moneda"] == "agent_owned"
    assert d["leer_perfil"]["source"] == "customer_profile" and out["provenance"]["leer_perfil"] == "provider_only"
    assert out["provenance"]["leer_productos"] == "aligned_to_provider"
    assert ta.compare(list(d.values()), prov)["findings"] == [] or all(f["kind"] == "not_in_fixture" for f in ta.compare(list(d.values()), prov)["findings"])


def test_loader_accepts_the_listing_shape_and_rejects_garbage(tmp_path):
    p = tmp_path / "l.json"
    p.write_text(json.dumps({"tools": [info("a")]}), "utf-8")
    assert ta.load_provider(p)["tools"][0]["id"] == "a"
    p.write_text(json.dumps({"nope": []}), "utf-8")
    with pytest.raises(ta.InputError):
        ta.load_provider(p)
    p.write_text("not json", "utf-8")
    with pytest.raises(ta.InputError):
        ta.load_provider(p)


# --- the committed snapshots: the baseline is the recorded truth of 2026-10-05 -------------------------------------------------

def test_committed_snapshots_match_the_baseline_exactly():
    fx = json.loads((TA / "agent_core_tools.snapshot.json").read_text("utf-8"))["tools"]
    prov = json.loads((TA / "tool_service_catalog.snapshot.json").read_text("utf-8"))
    base = json.loads((TA / "known_drift.json").read_text("utf-8"))["entries"]
    g = ta.gate(ta.compare(fx, prov), base)
    assert g["ok"], g
    assert all(isinstance(v, str) and v for v in base.values()), "every baseline entry names its ask"


def test_the_five_drifting_sources_of_art2_are_in_the_baseline_and_radicar_pqr_too():
    base = json.loads((TA / "known_drift.json").read_text("utf-8"))["entries"]
    for k in ("leer_productos:source_mismatch", "leer_movimientos:source_mismatch", "leer_pqr_cliente:source_mismatch",
              "obtener_pqr:source_missing", "buscar_transacciones:source_missing", "radicar_pqr:source_missing"):
        assert k in base, k


def test_aligned_fixture_has_every_provider_source_and_no_source_drift():
    al = json.loads((TA / "aligned_tool_defs.json").read_text("utf-8"))
    prov = json.loads((TA / "tool_service_catalog.snapshot.json").read_text("utf-8"))
    r = ta.compare(list(al["tool_defs"].values()), prov)
    assert [f for f in r["findings"] if f["kind"] != "not_in_provider"] == [], r["findings"]
    assert {t["id"]: t["source"] for t in prov["tools"]} == {i: al["tool_defs"][i]["source"] for i in (t["id"] for t in prov["tools"])}


def test_the_cli_exits_0_on_the_committed_snapshots_and_1_on_new_drift(tmp_path):
    cmd = [sys.executable, str(TA / "check_tool_alignment.py"), "--fixtures", str(TA / "agent_core_tools.snapshot.json"),
           "--provider", str(TA / "tool_service_catalog.snapshot.json"), "--baseline", str(TA / "known_drift.json")]
    ok = subprocess.run(cmd, capture_output=True, text=True)
    assert ok.returncode == 0, ok.stdout + ok.stderr
    broken = json.loads((TA / "tool_service_catalog.snapshot.json").read_text("utf-8"))
    broken["tools"][0]["risk_class"] = "write_irreversible"
    p = tmp_path / "p.json"
    p.write_text(json.dumps(broken), "utf-8")
    bad = subprocess.run([*cmd[:-4], "--provider", str(p), "--baseline", str(TA / "known_drift.json")], capture_output=True, text=True)
    assert bad.returncode == 1 and "risk_class_mismatch" in bad.stdout
    missing = subprocess.run([*cmd[:-4], "--provider", str(tmp_path / "none.json"), "--baseline", str(TA / "known_drift.json")], capture_output=True, text=True)
    assert missing.returncode == 2
