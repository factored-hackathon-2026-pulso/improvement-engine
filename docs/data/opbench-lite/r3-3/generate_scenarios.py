"""Emit the synthetic Agent Core suites and their aggregate provenance sidecar."""

import hashlib
import json
import argparse
from pathlib import Path


SCHEMA_BLOB = "ba8c609aeb53278420f13d867431bdda1231e5a0"


def _load_catalog():
    path = Path(__file__).with_name("evidence_catalog.json")
    raw = path.read_bytes()
    return json.loads(raw), hashlib.sha256(raw).hexdigest()


def _aggregate_entry(entry_id, entry):
    """Return the row-free source fields that bind the probe to its cited aggregate."""
    return {
        "entry_id": entry_id,
        "type": entry["type"],
        "status": entry["status"],
        "actionability": entry["actionability"],
        "definition": entry["definition"],
        "cell": entry["cell"],
        "discovery": entry["discovery"],
        "replication": entry["replication"],
        "snapshot": entry["snapshot"],
    }


def _aggregate_hash(entry_id, entry):
    payload = json.dumps(_aggregate_entry(entry_id, entry), ensure_ascii=False,
                         sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def verify_source_catalog(raw_path, excerpt):
    """Check the original aggregate artifact digest and copied entry fields."""
    raw = Path(raw_path).read_bytes()
    if hashlib.sha256(raw).hexdigest() != excerpt["source_sha256"]:
        raise ValueError("original OPBENCH artifact SHA-256 does not match the pinned excerpt")
    source = json.loads(raw)
    entries = {entry["id"]: entry for entry in source["entries"]}
    for entry_id, copied in excerpt["entries"].items():
        original = entries[entry_id]
        for key in ("title", "type", "status", "actionability", "definition", "cell"):
            if copied[key] != original[key]:
                raise ValueError(f"{entry_id}: aggregate excerpt field {key} differs from source")
        for part in ("discovery", "replication"):
            selected = {key: original[part][key] for key in copied[part]}
            if selected != copied[part]:
                raise ValueError(f"{entry_id}: aggregate excerpt field {part} differs from source")
        selected_snapshot = {key: original["snapshot"][key] for key in copied["snapshot"]}
        if selected_snapshot != copied["snapshot"]:
            raise ValueError(f"{entry_id}: aggregate excerpt field snapshot differs from source")
        if copied["snapshot"].get("suppressed") is not False:
            raise ValueError(f"{entry_id}: suppressed source snapshots cannot support this excerpt")


def _scenario(suite, slug, country, lang, text, outcome, *, tools=None, actions=None, escalated=False,
              steps=None):
    principal_no = len(_SCENARIOS[suite]) + 1
    principal_role = "advisor" if suite == "copiloto-asesor" else "customer"
    result = {
        "id": f"probe-{slug}",
        "source": "scripted",
        "principal": {"id": f"syn-{principal_role}-{suite[:3]}-{principal_no:02d}",
                      "attrs": {"country": country, **({"role": "advisor"} if principal_role == "advisor" else {})}},
        "steps": steps or [{"op": "start", "lang": lang}, {"op": "turn", "lang": lang, "text": text}],
        "expect": {"outcome": outcome, "escalated": escalated},
        "assertions": [],
    }
    if tools:
        result["seed"] = {"tools": tools}
    if actions:
        result["expect"]["actions_verified"] = actions
    return result


_SCENARIOS = {"disputas": [], "consultas": [], "copiloto-asesor": []}


def build_pack():
    _SCENARIOS.update({"disputas": [], "consultas": [], "copiloto-asesor": []})
    d = _SCENARIOS["disputas"]
    def dispute_seed(trx, *, readback_error=False):
        txid, pqr = f"syn-tx-{trx}", f"syn-pqr-{trx}"
        return {
            "buscar_transacciones": [{"result": [{"transaction_id": txid, "amount": "120.00", "currency": "USD"}]}],
            "seleccionar": [{"result": {"transaction_id": txid, "amount": "120.00", "currency": "USD"}}],
            "convertir_moneda": [{"result": "120.00"}],
            "radicar_pqr": [{"result": {"status": "Open", "id": pqr}}],
            "obtener_pqr": ([{"status": "error", "error": "synthetic_readback_failure"}] if readback_error
                             else [{"result": {"status": "Open", "id": pqr}}]),
        }
    yes = [{"op": "start", "lang": "es"},
           {"op": "turn", "lang": "es", "text": "No reconozco el cargo ficticio syn-tx-101 y quiero reportarlo."},
           {"op": "confirm", "answer": "yes", "auth": "step_up"}]
    d.append(_scenario("disputas", "cargo-no-reconocido-es", "MX", "es", yes[1]["text"], "resolved",
                       tools=dispute_seed("101"), actions=["radicar_pqr"], steps=yes))
    no = [{"op": "start", "lang": "pt"},
          {"op": "turn", "lang": "pt", "text": "Vejo duas cobranças de teste syn-tx-202; ajude-me a revisar."},
          {"op": "confirm", "answer": "no", "auth": "step_up"}]
    d.append(_scenario("disputas", "cobro-duplicado-cancelado-pt", "CO", "pt", no[1]["text"], "cancelled",
                       tools=dispute_seed("202"), steps=no))
    d.append(_scenario("disputas", "motivo-vago-es", "AR", "es",
                       "Hay algo raro en mis movimientos ficticios.", "escalated", escalated=True,
                       steps=[{"op": "start", "lang": "es"},
                              {"op": "turn", "lang": "es", "text": "Hay algo raro en mis movimientos ficticios."},
                              {"op": "turn", "lang": "es", "text": "No puedo precisar monto, fecha ni comercio."}]))
    d.append(_scenario("disputas", "lookup-failure-pt", "MX", "pt",
                       "Não reconheço o movimento fictício syn-tx-404.", "escalated", escalated=True,
                       tools={"buscar_transacciones": [{"status": "error", "error": "synthetic_lookup_failure"}]}))
    high = [{"op": "start", "lang": "es"},
            {"op": "turn", "lang": "es", "text": "No reconozco la operación sintética syn-tx-505 por USD 600."}]
    d.append(_scenario("disputas", "amount-boundary-600-es", "CO", "es", high[1]["text"], "escalated",
                       escalated=True, tools={
                           "buscar_transacciones": [{"result": [
                               {"transaction_id": "syn-tx-505", "amount": "600.00", "currency": "USD"}]}],
                           "seleccionar": [{"result": {"transaction_id": "syn-tx-505", "amount": "600.00", "currency": "USD"}}],
                           "convertir_moneda": [{"result": "600.00"}],
                       }, steps=high))
    d.append(_scenario("disputas", "readback-failure-es", "AR", "es",
                       "No reconozco el cargo sintético syn-tx-606; quiero reportarlo.", "escalated", escalated=True,
                       tools=dispute_seed("606", readback_error=True),
                       steps=[{"op": "start", "lang": "es"},
                              {"op": "turn", "lang": "es", "text": "No reconozco el cargo sintético syn-tx-606; quiero reportarlo."},
                              {"op": "confirm", "answer": "yes", "auth": "step_up"}]))

    c = _SCENARIOS["consultas"]
    for idx, (country, lang, text, state) in enumerate([
        ("MX", "es", "Quiero consultar el estado de mi PQR sintética syn-pqr-501.", "Open"),
        ("CO", "es", "¿Qué avance tiene la solicitud de soporte ficticia syn-pqr-502?", "InProgress"),
        ("AR", "es", "Necesito revisar mi reclamo de prueba syn-pqr-503.", "Closed"),
        ("CO", "pt", "Quero consultar o andamento da solicitação fictícia syn-pqr-504.", "Open"),
    ], 1):
        pqr = f"syn-pqr-{500 + idx}"
        c.append(_scenario("consultas", f"pqr-status-{idx:02d}", country, lang, text, "resolved",
                           tools={"obtener_pqr": [{"result": {"id": pqr, "status": state}}]}))
        c[-1]["assertions"] = [
            {"event": "engine.tool_called", "where": [{"field": "status", "op": "eq", "value": "ok"}], "expect": "at_least_one"},
            {"event": "engine.response_failed", "expect": "none"},
        ]
    c.append(_scenario("consultas", "missing-radicado", "MX", "es",
                       "Quiero consultar mi PQR de prueba, pero no recuerdo el radicado.", "escalated", escalated=True,
                       steps=[{"op": "start", "lang": "es"},
                              {"op": "turn", "lang": "es", "text": "Quiero consultar mi PQR de prueba, pero no recuerdo el radicado."},
                              {"op": "turn", "lang": "es", "text": "No tengo ningún identificador; por favor escálalo."}]))
    c[-1]["assertions"] = [{"event": "engine.tool_called", "expect": "none"}]
    c.append(_scenario("consultas", "fallo-de-consulta", "AR", "pt",
                       "Minha consulta de suporte está indisponível; também preciso revisar SYN-PQR-506.", "escalated",
                       tools={"obtener_pqr": [{"status": "error", "error": "synthetic_tool_failure"}]}, escalated=True))

    a = _SCENARIOS["copiloto-asesor"]
    for idx, (country, lang, text, amount) in enumerate([
        ("MX", "es", "Resume el movimiento ficticio syn-mov-601 y su estado.", "120.00"),
        ("CO", "es", "Ayúdame a ubicar el último movimiento de prueba syn-mov-602.", "45.50"),
        ("AR", "es", "¿Qué detalle tiene el movimiento sintético syn-mov-603?", "76.25"),
        ("CO", "pt", "Resuma a movimentação fictícia syn-mov-604 e seu estado.", "18.00"),
        ("MX", "pt", "Ajude a localizar a movimentação de teste syn-mov-605.", "39.99"),
        ("AR", "pt", "Qual é o detalhe da movimentação sintética syn-mov-606?", "61.10"),
    ], 1):
        a.append(_scenario("copiloto-asesor", f"lookup-movement-{idx:02d}", country, lang, text, "completed",
                           tools={"leer_movimientos": [{"result": [{"transaction_id": f"syn-mov-{600 + idx}",
                                                                        "amount": amount, "currency": "USD",
                                                                        "status": "posted"}]}]}))

    catalog, catalog_digest = _load_catalog()
    source_findings = catalog["entries"]
    mapping = {}
    for suite_id, suite in _SCENARIOS.items():
        finding = "M1-01" if suite_id == "disputas" else "M1-13" if suite_id == "consultas" else "E1-01"
        for scenario in suite:
            is_control = finding == "E1-01"
            mapping[scenario["id"]] = {
                "opbench_entry_id": finding,
                "catalog_sha256": catalog_digest,
                "source_artifact_sha256": catalog["source_sha256"],
                "aggregate_entry_sha256": _aggregate_hash(finding, source_findings[finding]),
                "cell_descriptor": source_findings[finding]["cell"],
                "evidence_level": "aggregate_snapshot_only",
                "probe_status": "covered_capability_negative_control" if is_control else "authored_synthetic_hypothesis",
                "expected_behavior_status": "contract_target_not_observed",
                "probe_rationale": ("SYNTHETIC authored representative hypothesis; not a cause or subtype established by the aggregate."
                                    if not is_control else
                                    "SYNTHETIC covered-capability regression probe; does not test detector suppression or establish an opportunity."),
                "target_behavior": scenario["expect"],
                "runtime_audit_required": ([
                    "Confirm read/selection/conversion calls may occur; inspect audit events and verify no radicar_pqr or write event."
                ] if scenario["id"] in {"probe-cobro-duplicado-cancelado-pt", "probe-amount-boundary-600-es"} else
                    ["Verify radicar_pqr was attempted and read-back failure was surfaced; do not infer a completed case from the seeded response."]
                    if scenario["id"] == "probe-readback-failure-es" else []),
                "execution_result": "not_run",
                "evaluator_status": "not_evaluable" if is_control else "not_run",
            }

    suites = {}
    for suite_id, scenarios in _SCENARIOS.items():
        suites[suite_id] = {"id": suite_id, "version": "1.1.0", "agent_id": suite_id,
                            "repetitions": 3, "scenarios": scenarios}

    return {
        "suites": suites,
        "provenance": {
            "format": "pulso.opbench-scenario-provenance.v1",
            "catalog_path": "outcome-temp/opbench-v2-1/v2/opbench-lite.json",
            "catalog_excerpt_sha256": catalog_digest,
            "source_artifact_sha256": catalog["source_sha256"],
            "source_catalog_format": catalog["format"],
            "pinned_eval_suite_schema_blob": SCHEMA_BLOB,
            "aggregate_entry_hash_algorithm": "sha256(UTF-8 canonical JSON of row-free entry_id/type/status/actionability/definition/cell/discovery/replication/snapshot; sort_keys=true; separators=(comma,colon); ensure_ascii=false)",
            "source_findings": source_findings,
            "real_row_data_included": False,
            "pii_included": False,
            "scenario_map": mapping,
            "suite_notes": {
                "disputas": "Inspired only by broad M1-01 phone/complaint unresolved-first-contact contrast; charge subtype and its cause are authored probes, not source findings.",
                "consultas": "Inspired only by broad M1-13 phone/technical unresolved-first-contact contrast; PQR lookups and malformed-reference cases are authored capability probes, not demonstrated causes.",
                "copiloto-asesor": "E1-01 covered-capability negative control; probes existing movement lookup only. Not test detector suppression, not a new opportunity. Pinned evaluator principal is customer-shaped, not an authenticated advisor identity.",
            },
        },
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=("disputas", "consultas", "copiloto-asesor"))
    parser.add_argument("--provenance", action="store_true")
    args = parser.parse_args()
    pack = build_pack()
    output = pack["provenance"] if args.provenance else pack["suites"][args.suite] if args.suite else pack
    print(json.dumps(output, ensure_ascii=False, indent=2, sort_keys=True))
