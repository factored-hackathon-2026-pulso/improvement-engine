"""PL-0007 first RED: exporter metadata discriminator (contract 1.1.0, source_event.kind = exporter_finding)."""

import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator

import platform_contract as pc
from platform_contract import conformance

ROOT = Path(__file__).resolve().parents[1]
VALID = sorted((ROOT / "examples" / "source_events" / "valid").glob("*.json"))
INVALID = sorted((ROOT / "examples" / "source_events" / "invalid").glob("*.json"))


def _load(p):
    return json.loads(p.read_text(encoding="utf-8"))


def test_revision_is_1_2_0_and_1_1_0_discriminator_stays():
    assert pc.CONTRACT_VERSION == "1.2.0"
    assert pc.PREVIOUS_CONTRACT_VERSION == "1.1.0"
    cat = json.loads((ROOT / "event-catalog.json").read_text("utf-8"))
    assert cat["contract_version"] == "1.2.0" and cat["catalog_version"] == "1.2.0"
    assert set(cat["source_event_kinds"]) == {"domain_event", "exporter_finding"}
    legacy = cat["legacy_exporter_prefix"]
    assert legacy["prefix"] == "exporter." and legacy["valid_for_contract_versions"] == ["1.0.0"]
    # the 1.0.0 event type list is untouched
    assert "case.opened" in {e["event_type"] for e in cat["event_types"]}


@pytest.mark.parametrize("name", ["exporter_finding", "domain_event", "source_observation"])
def test_new_schemas_exist_and_are_valid(name):
    schema = json.loads((ROOT / "schemas" / f"{name}.schema.json").read_text("utf-8"))
    Draft202012Validator.check_schema(schema)
    assert schema["$id"] == f"platform_live/1.2.0/{name}.schema.json"


def test_exporter_finding_schema_envelope_fields():
    s = pc.load_source_schema("exporter_finding")
    assert set(s["required"]) == {
        "kind", "source_namespace", "catalog_version", "tenant_id", "finding_code", "severity",
        "described_native_event_id", "described_source_sequence", "details"}
    # identity (source_id, native_event_id) and observed_at live on the observation envelope, outside the digest
    obs = pc.load_source_schema("source_observation")
    assert {"tenant_id", "source_id", "native_event_id", "source_sequence", "observed_at"} <= set(obs["required"])
    assert s["properties"]["kind"] == {"const": "exporter_finding"}
    assert s["properties"]["described_source_sequence"]["type"] == ["integer", "null"]
    assert s["additionalProperties"] is False


def test_golden_files_cover_the_agreed_cases():
    names = {p.stem for p in VALID}
    assert {"exporter_finding.valid", "exporter_finding_null_sequence.valid",
            "exporter_finding_late_sequence.valid", "exporter_finding_dedup.valid", "domain_event.valid"} <= names
    assert len(INVALID) >= 6


@pytest.mark.parametrize("path", VALID, ids=lambda p: p.stem)
def test_golden_valid_streams_conform(path):
    doc = _load(path)
    assert conformance.validate_source_observations(doc["records"]) == []
    assert conformance.check_observation_sequences(doc["records"]) == doc.get("sequence_findings", [])
    uniq, dups = conformance.dedup_observations(doc["records"])
    assert len(uniq) == doc["unique_count"] and len(dups) == len(doc["records"]) - doc["unique_count"]
    assert all(d["code"] == "duplicate_identical" for d in dups)


@pytest.mark.parametrize("path", INVALID, ids=lambda p: p.stem)
def test_golden_invalid_rows_are_rejected(path):
    doc = _load(path)
    assert conformance.validate_source_observations(doc["records"]), path.name


def test_null_sequence_finding_is_not_a_continuity_problem():
    doc = _load(ROOT / "examples" / "source_events" / "valid" / "exporter_finding_null_sequence.valid.json")
    fs = [r for r in doc["records"] if r["source_event"]["kind"] == "exporter_finding"]
    assert len(fs) == 2
    assert all(f["source_sequence"] is None and f["source_event"]["described_source_sequence"] is None for f in fs)


def test_finding_reusing_a_late_event_sequence_does_not_create_a_duplicate_domain_sequence():
    doc = _load(ROOT / "examples" / "source_events" / "valid" / "exporter_finding_late_sequence.valid.json")
    seqs = [r["source_sequence"] for r in doc["records"]]
    assert len(seqs) != len(set(seqs))  # the finding shares the late row's sequence
    assert conformance.check_observation_sequences(doc["records"]) == []


def test_findings_never_hide_a_domain_sequence_gap():
    doc = _load(ROOT / "examples" / "source_events" / "valid" / "exporter_finding_late_sequence.valid.json")
    dom = [r for r in doc["records"] if r["source_event"]["kind"] == "domain_event"][0]
    gapped = [dom, dict(dom, native_event_id="EVT-0099", source_sequence=dom["source_sequence"] + 2,
                        source_event=dict(dom["source_event"], event_id="EVT-0099"))]
    # include a finding sitting exactly in the hole: it must not fill it
    fnd = [r for r in doc["records"] if r["source_event"]["kind"] == "exporter_finding"][0]
    fnd = dict(fnd, source_sequence=dom["source_sequence"] + 1)  # described sequence matches: allowed, but no fill
    assert [f["code"] for f in conformance.check_observation_sequences(gapped + [fnd])] == ["gap_suspected"]
    lying = dict(fnd, source_event=dict(fnd["source_event"], described_source_sequence=None))
    codes = [f["code"] for f in conformance.check_observation_sequences(gapped + [lying])]
    assert codes == ["gap_suspected", "finding_sequence_mismatch"]


def test_dedup_keeps_first_and_flags_identity_conflicts():
    doc = _load(ROOT / "examples" / "source_events" / "valid" / "exporter_finding_dedup.valid.json")
    a = doc["records"][0]
    conflicting = dict(a, source_event=dict(a["source_event"], severity="error"))
    uniq, dups = conformance.dedup_observations([a, conflicting])
    assert len(uniq) == 1 and [d["code"] for d in dups] == ["identity_conflict"]


def test_envelope_tenant_must_match_source_event_and_finding_identity_is_prefixed():
    doc = _load(ROOT / "examples" / "source_events" / "valid" / "exporter_finding.valid.json")
    f = [r for r in doc["records"] if r["source_event"]["kind"] == "exporter_finding"][0]
    assert conformance.validate_source_observations([dict(f, tenant_id="other")])
    assert conformance.validate_source_observations([dict(f, native_event_id="EVT-0003")])


def test_details_are_bounded():
    doc = _load(ROOT / "examples" / "source_events" / "valid" / "exporter_finding.valid.json")
    r = json.loads(json.dumps(doc["records"][0]))
    r["source_event"]["details"] = {"blob": "x" * (pc.MAX_FINDING_DETAILS_BYTES + 1)}
    assert conformance.validate_source_observations([r])


def test_violation_messages_do_not_echo_values():
    doc = _load(ROOT / "examples" / "source_events" / "valid" / "exporter_finding.valid.json")
    r = json.loads(json.dumps(doc["records"][0]))
    r["source_event"]["severity"] = "TOPSECRETVALUE"
    assert "TOPSECRETVALUE" not in "\n".join(conformance.validate_source_observations([r]))


def test_classification_and_legacy_prefix_rule():
    c = conformance.classify_source_event
    assert c({"kind": "exporter_finding"}) == "exporter_finding"
    assert c({"kind": "domain_event", "event_type": "case.opened"}) == "domain_event"
    assert c({"event_type": "exporter.finding"}) == "legacy_exporter_prefix"
    assert c({"event_type": "case.opened"}) == "legacy_domain_event"
    assert c({"kind": "mystery"}) == "unsupported"
    assert c({}) == "unsupported"


def test_domain_event_may_not_use_the_exporter_prefix():
    doc = _load(ROOT / "examples" / "source_events" / "valid" / "domain_event.valid.json")
    r = json.loads(json.dumps(doc["records"][0]))
    r["source_event"]["event_type"] = "exporter.finding"
    assert conformance.validate_source_observations([r])
