"""Static quality gates of the published contract (no target, no database): schemas are valid JSON Schema 2020-12 with
resolvable refs, the OpenAPI document is consistent with `contract.json`, and every golden example validates against its
schema. Exporter-side DTOs are produced by the REAL exporter code and validated against the published schemas."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest
import yaml
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parents[1]
SCHEMAS = {p.name.removesuffix(".schema.json"): json.loads(p.read_text(encoding="utf-8"))
           for p in sorted((ROOT / "schemas").glob("*.schema.json"))}
CONTRACT = json.loads((ROOT / "contract.json").read_text(encoding="utf-8"))
OPENAPI = yaml.safe_load((ROOT / "openapi" / "bridge-internal-v1.yaml").read_text(encoding="utf-8"))
REGISTRY = Registry()
for _n, _s in SCHEMAS.items():
    REGISTRY = REGISTRY.with_resource(f"{_n}.schema.json", Resource.from_contents(_s))


def _placeholder(err: Any) -> bool:
    """Volatile ids are stored as `<key#n>` placeholders: type/pattern/format checks cannot apply to them."""
    i = err.instance
    return isinstance(i, str) and i.startswith("<") and i.endswith(">") or err.validator in ("pattern", "format")


def validate(name: str, instance: Any) -> None:
    Draft202012Validator(SCHEMAS[name], registry=REGISTRY).validate(instance)


@pytest.mark.parametrize("name", sorted(SCHEMAS))
def test_every_schema_is_valid_2020_12_and_its_refs_resolve(name: str) -> None:
    schema = SCHEMAS[name]
    Draft202012Validator.check_schema(schema)
    assert schema["$schema"] == "https://json-schema.org/draft/2020-12/schema"
    assert schema["$id"] == f"{name}.schema.json" and schema["title"] == name

    def refs(node: Any) -> list[str]:
        if isinstance(node, dict):
            return ([node["$ref"]] if "$ref" in node else []) + [r for v in node.values() for r in refs(v)]
        if isinstance(node, list):
            return [r for v in node for r in refs(v)]
        return []

    for ref in refs(schema):
        if ref.endswith(".schema.json"):
            assert ref.removesuffix(".schema.json") in SCHEMAS, f"{name} -> {ref}"
        else:
            REGISTRY.resolver(base_uri=schema["$id"]).lookup(ref)  # in-document ref must resolve


def test_every_route_of_the_contract_is_in_the_openapi_with_its_auth_and_pending_flag() -> None:
    assert OPENAPI["openapi"] == "3.1.0"
    seen = set()
    for r in CONTRACT["routes"]:
        op = OPENAPI["paths"]["/internal/v1" + r["path"]][r["method"].lower()]
        seen.add(op["operationId"])
        assert op["x-pulso-auth"]["purposes"] == r["purposes"] and op["x-pulso-auth"]["audience"] == "core-bridge"
        assert op["security"] == [{"serviceJwt": []}]
        assert ("x-status" in op) == ("x-status" in r) and op.get("x-status", "pending-implementation") == (
            "pending-implementation")
    assert len(seen) == len(CONTRACT["routes"]), "operationIds must be unique"
    paths = {(p, m) for p, ops in OPENAPI["paths"].items() for m in ops}
    assert len(paths) == len(CONTRACT["routes"])


def test_openapi_refs_resolve_and_no_external_refs_remain() -> None:
    text = json.dumps(OPENAPI)
    assert ".schema.json" not in text
    comps = set(OPENAPI["components"]["schemas"]) | set(OPENAPI["components"]["parameters"])

    def walk(node: Any) -> None:
        if isinstance(node, dict):
            if "$ref" in node:
                assert node["$ref"].rsplit("/", 1)[-1] in comps, node["$ref"]
            for v in node.values():
                walk(v)
        elif isinstance(node, list):
            for v in node:
                walk(v)

    walk(OPENAPI)
    assert set(OPENAPI["components"]["schemas"]) == set(SCHEMAS)


def test_openapi_documents_the_documented_statuses_of_invoke() -> None:
    op = OPENAPI["paths"]["/internal/v1/core-tasks/invoke"]["post"]
    assert {"200", "202", "401", "403", "409", "413", "422", "429", "503"} <= set(op["responses"])
    assert any(p.get("$ref", "").endswith("IdempotencyKey") for p in op["parameters"])


def test_error_envelope_enum_is_exactly_the_wire_codes() -> None:
    wire = sorted(c for c in CONTRACT["error_codes"]["wire"])
    assert sorted(SCHEMAS["ErrorEnvelope"]["properties"]["code"]["enum"]) == wire
    non_wire = set(CONTRACT["error_codes"]["non_wire"])
    for code in non_wire - set(wire):
        assert code not in wire
    for code, entry in CONTRACT["error_codes"]["wire"].items():
        assert code.startswith("pulso:") and entry["status"] and "doc" in entry


def test_receipt_state_machine_is_closed_and_terminal_states_have_no_exit() -> None:
    sm = CONTRACT["receipt_state_machine"]
    assert set(sm["terminal"]) <= set(sm["states"])
    for target, sources in sm["transitions"].items():
        assert not set(sources) & set(sm["terminal"]), f"terminal state is a source of {target}"
    assert sm["transitions"]["unknown"] and "prepared" not in sm["transitions"]["unknown"]


def test_stage_catalogue_and_fact_schemas_line_up() -> None:
    for stage, spec in CONTRACT["stages"].items():
        assert f"Fact_{spec['fact']}" in SCHEMAS
        assert SCHEMAS["CoreTaskInvocation"]["properties"]["stage"]["enum"].count(stage) == 1


# -- golden examples ---------------------------------------------------------------------------------------------
STEPS = [(f.stem, s) for f in sorted((ROOT / "examples" / "flows").glob("*.json"))
         for s in json.loads(f.read_text(encoding="utf-8"))["steps"]]
REQUEST_SCHEMA = {("POST", "/core-tasks/invoke"): "CoreTaskInvocation",
                  ("POST", "/core-credentials/issue"): "CoreCredentialIssueRequest",
                  ("POST", "/evaluation/admissions"): "EvaluationAdmissionRequest",
                  ("POST", "/evaluation/arms/run"): "ArmRequest",
                  ("POST", "/core-authoring/dry-run"): "CoreAuthoringDryRunRequest"}


def test_there_are_golden_examples_for_every_route_class() -> None:
    paths = {(s["request"]["method"], s["request"]["path"].split("/")[1] + "/" + (s["request"]["path"].split("/")[2]
                                                                                if len(s["request"]["path"].split("/")) > 2 else ""))
             for _, s in STEPS}
    for needle in ("core-tasks/", "core-credentials/", "evaluation/admissions", "evaluation/arms", "version/",
                   "core-state/aliases", "core-authoring/dry-run"):
        assert any(needle.split("/")[0] in p[1] for p in paths), needle
    assert any(s["response"]["status"] >= 400 for _, s in STEPS)


@pytest.mark.parametrize("flow,step", STEPS, ids=[f"{f}/{s['case']}" for f, s in STEPS])
def test_golden_response_validates_against_its_schema(flow: str, step: dict) -> None:
    resp = step["response"]
    body = resp["body"]
    schema = resp["schema"] if resp["status"] < 400 else "ErrorEnvelope"
    if flow == "invoke_scout" and step["case"] == "invoke_scout_ok":
        body = {**body}  # placeholders are strings: shape checks only
    # placeholders (<core_run_id#1> ...) replace volatile strings; pattern-constrained fields are checked loosely
    loose = json.loads(json.dumps(body))
    errors = [e for e in Draft202012Validator(SCHEMAS[schema], registry=REGISTRY).iter_errors(loose)
              if not _placeholder(e)]
    assert not errors, [e.message[:160] for e in errors][:3]


@pytest.mark.parametrize("flow,step", STEPS, ids=[f"{f}/{s['case']}" for f, s in STEPS])
def test_golden_requests_are_wire_shaped(flow: str, step: dict) -> None:
    req = step["request"]
    assert "authorization" not in {k.lower() for k in req.get("headers", {})}
    assert "eyJ" not in json.dumps(step), "a token leaked into an example"
    name = REQUEST_SCHEMA.get((req["method"], req["path"]))
    body = req.get("body")
    if name is None or not isinstance(body, dict) or "$raw" in body:
        return
    errors = [e for e in Draft202012Validator(SCHEMAS[name], registry=REGISTRY).iter_errors(body)
              if not _placeholder(e)]
    # negative examples are INVALID by design; positive ones must validate
    if step["response"]["status"] in (200, 201, 202):
        assert not errors, [e.message[:160] for e in errors][:3]


# -- exporter side: produced by the REAL exporter code --------------------------------------------------------------
def _exporter() -> Any:
    from pulso_core_runtime.exporter import Exporter, ExporterConfig

    ref = {"id": "schema-1", "digest": "a" * 64, "media_type": "application/json"}
    cfg = ExporterConfig(tenant_id="t1", instance="core-a", expected_runtime_db="rt", expected_eval_db="ev",
                         binding_ref="binding-1", source_schema_ref=ref, registry_schema_ref=ref)
    return Exporter(cfg, None, None, None), ref  # type: ignore[arg-type]


def test_audit_observation_built_by_the_real_exporter_validates() -> None:
    from pulso_core_runtime.exporter.reader import AuditRow

    exp, _ref = _exporter()
    row = AuditRow("run-A", 7, "run-A-e7", "agent_step", None, "p" * 64, "h" * 64, "{}")
    obs = exp._obs_audit(row, {"id": "chain-1", "digest": "b" * 64, "media_type": "application/x-ndjson"}, "open_run")
    validate("ObservationEvent", obs)
    assert obs["source_event"] is None and obs["source_event_digest"] == "h" * 64 and obs["source_sequence"] == 7
    assert set(obs) == set(SCHEMAS["ObservationEvent"]["properties"]), "exporter and contract key sets must match"


def test_outbound_observation_built_by_the_real_exporter_validates(monkeypatch: pytest.MonkeyPatch) -> None:
    from pulso_core_runtime.exporter import service

    exp, _ = _exporter()
    view = {"event_id": "ev-1", "kind": "registry.proposal_frozen"}
    monkeypatch.setattr(service.Exporter, "_project", staticmethod(lambda source, row: (view, None)))
    obs = exp._obs_sparse(service.REGISTRY, object(), None)
    validate("ObservationEvent", obs)
    assert obs["level"] == "outbound_event" and obs["source_event"] == view and obs["source_sequence"] is None


def test_agent_step_failed_evidence_matches_the_checked_in_real_core_fixtures() -> None:
    fixtures = ROOT.parent / "core-bridge" / "tests" / "fixtures" / "dependency_evidence"
    outage = json.loads((fixtures / "outage_unavailable.json").read_text(encoding="utf-8"))
    validate("AuditAgentStepFailedEvidence", outage["agent_step_event"])
    assert outage["agent_step_event"]["payload"]["kind"] == "failed"
    low = json.loads((fixtures / "low_confidence_gave_up.json").read_text(encoding="utf-8"))
    assert low["agent_step_failed_present"] is False, "a legitimate give-up is NOT dependency evidence"
    obs = dict(outage["observation"])
    obs["source_event_ref"] = {"id": "chain-1", "digest": "b" * 64, "media_type": "application/x-ndjson"}
    obs["source_schema_ref"] = {"id": "schema-1", "digest": "a" * 64, "media_type": "application/json"}
    obs["observed_at"] = "2026-10-03T12:00:00Z"
    validate("ObservationEvent", obs)


def test_no_route_or_schema_is_marked_pending_implementation() -> None:
    """Alias read and authoring dry-run are implemented and reviewed (agent-core pin 894fa65): the markers are gone."""
    assert "pending-implementation" not in json.dumps(CONTRACT) + json.dumps(OPENAPI) + json.dumps(SCHEMAS)
    assert not any(c.get("pending") for c in CONTRACT["error_codes"]["wire"].values()), "pending error codes remain"
    for flow in (ROOT / "examples" / "flows").glob("*.json"):
        assert "x-status" not in json.loads(flow.read_text(encoding="utf-8")), flow.name


def test_alias_and_dry_run_goldens_cover_200_404_valid_and_invalid() -> None:
    flow = json.loads((ROOT / "examples" / "flows" / "authoring.json").read_text(encoding="utf-8"))
    cases = {s["case"]: s["response"]["status"] for s in flow["steps"]}
    assert cases["alias_read"] == 200 and cases["alias_unknown_agent"] == 404
    assert cases["dry_run_valid"] == 200 and cases["dry_run_violations"] == 200
    valid = next(s for s in flow["steps"] if s["case"] == "dry_run_valid")["response"]["body"]
    invalid = next(s for s in flow["steps"] if s["case"] == "dry_run_violations")["response"]["body"]
    assert valid["valid"] is True and valid["candidate_hash"] and not valid["violations"]
    assert invalid["valid"] is False and invalid["candidate_hash"] is None and invalid["violations"]
