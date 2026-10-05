"""PL-0007 first RED: the exporter emits the contract 1.1.0 `exporter_finding` kind (legacy `exporter.` prefix only
behind legacy_prefix=True) and every observation it sends conforms to platform-contract."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "platform-contract"))

import platform_contract as pc  # noqa: E402
from platform_contract import conformance  # noqa: E402

from platform_exporter import catalog  # noqa: E402
from tests.platform_db import add_event  # noqa: E402


def _obs(rig):
    return [e for b in rig.ingest.batches for e in b["events"] if e["kind"] == "platform_event"]


def _records(rig):
    out = []
    for b in rig.ingest.batches:
        for e in b["events"]:
            out.append({"tenant_id": b["tenant_id"], "source_id": b["source_id"], "native_event_id": e["native_event_id"],
                        "source_sequence": e["source_sequence"], "observed_at": e["observed_at"],
                        "source_event": e["source_event"]})
    return out


def _findings(rig, code=None):
    return [e for e in _obs(rig) if e["source_event"].get("kind") == "exporter_finding"
            and (code is None or e["source_event"]["finding_code"] == code)]


def test_exporter_declares_the_contract_revision_it_implements():
    assert catalog.CONTRACT_REVISION == pc.CONTRACT_VERSION == "1.3.0"


def test_unknown_type_finding_uses_exporter_finding_kind_not_the_prefix(rig):
    add_event(rig.db, 1, "case.opened")
    add_event(rig.db, 2, "team.created", payload={"secret_marker": "DO-NOT-FORWARD"})
    rig.make().poll_once()
    (f,) = _findings(rig, "unknown_event_type")
    se = f["source_event"]
    assert "event_type" not in se and se["severity"] == "warning"
    assert se["described_native_event_id"] == "EVT-2" and se["described_source_sequence"] == 2
    assert f["native_event_id"] == "finding:unknown_event_type:EVT-2" and f["source_sequence"] == 2
    assert se["details"]["event_type"] == "team.created"
    assert not any(e["source_event"].get("event_type", "").startswith("exporter.") for e in _obs(rig))
    assert b"DO-NOT-FORWARD" not in b"".join(rig.ingest.raw_bodies)


def test_gap_finding_has_null_sequences(rig):
    for seq in (1, 2, 5, 6):
        add_event(rig.db, seq, "case.viewed")
    rig.make().poll_once()
    (f,) = _findings(rig, "gap_suspected")
    assert f["source_sequence"] is None and f["source_event"]["described_source_sequence"] is None
    assert f["source_event"]["described_native_event_id"] is None
    assert f["source_event"]["details"] == {"from_sequence": 3, "to_sequence": 4, "backfill_requested": True,
                                            "verify_with_owner": True}


def test_late_event_finding_reuses_the_late_row_sequence_and_never_fills_continuity(rig):
    add_event(rig.db, 1, "case.viewed", event_time="2026-03-01T10:30:00Z", ingested_at="2026-03-01T10:31:00Z")
    add_event(rig.db, 2, "case.viewed", event_time="2026-03-01T10:40:00Z", ingested_at="2026-03-01T11:20:00Z")
    rig.make(window_seconds=3600).poll_once()
    (f,) = _findings(rig, "late_event")
    assert f["source_sequence"] == 2 and f["source_event"]["described_source_sequence"] == 2
    assert f["source_event"]["described_native_event_id"] == "EVT-2"
    assert f["source_event"]["details"]["window_revision_required"] is True
    recs = _records(rig)
    assert conformance.check_observation_sequences(recs) == []
    assert [r["source_sequence"] for r in recs].count(2) >= 2


def test_domain_rows_carry_kind_domain_event(rig):
    add_event(rig.db, 1, "case.opened")
    rig.make().poll_once()
    doms = [e for e in _obs(rig) if e["native_event_id"] == "EVT-1"]
    assert doms and all(e["source_event"]["kind"] == "domain_event" for e in doms)


def test_every_observation_conforms_to_the_contract_including_rescan_meta(rig):
    for seq in (1, 2, 5):
        add_event(rig.db, seq, "case.viewed")
    add_event(rig.db, 6, "brand.new")
    add_event(rig.db, 7, "auth.password_accepted")
    rig.db.executemany("INSERT INTO turns(id,case_id,sequence) VALUES(?,?,?)", [("T1", "CASE-1", 1), ("T3", "CASE-1", 3)])
    rig.db.commit()
    ex = rig.make()
    assert not ex.poll_once().errors
    assert not ex.rescan().errors
    recs = _records(rig)
    assert conformance.validate_source_observations(recs) == []
    codes = {r["source_event"]["finding_code"] for r in recs if r["source_event"]["kind"] == "exporter_finding"}
    assert {"capability_profile", "dimension_snapshot", "turn_sequence_gap", "gap_suspected",
            "unknown_event_type", "denied_event_type"} <= codes
    assert not any(str(r["source_event"].get("event_type", "")).startswith("exporter.") for r in recs)


def test_repeated_rescan_does_not_create_identity_conflicts(rig):
    ex = rig.make()
    add_event(rig.db, 1, "case.viewed")
    ex.poll_once()
    assert not ex.rescan().errors and not ex.rescan().errors
    _unique, dups = conformance.dedup_observations(_records(rig))
    assert all(d["code"] == "duplicate_identical" for d in dups)


def test_legacy_prefix_flag_keeps_the_1_0_0_shape(rig):
    add_event(rig.db, 1, "case.opened")
    add_event(rig.db, 2, "team.created")
    ex = rig.make(legacy_prefix=True)
    ex.poll_once()
    ex.rescan()
    ses = [e["source_event"] for e in _obs(rig)]
    fnd = [s for s in ses if s.get("event_type") == "exporter.finding"]
    assert fnd and all("kind" not in s and s["finding"]["type"] for s in fnd)
    assert {"exporter.capability_profile", "exporter.dimension_snapshot"} <= {s.get("event_type") for s in ses}
    assert not any("kind" in s for s in ses)
    assert all(conformance.classify_source_event(s) in ("legacy_exporter_prefix", "legacy_domain_event") for s in ses)


def test_legacy_prefix_defaults_to_false(rig):
    from platform_exporter import ExporterConfig
    assert ExporterConfig(tenant_id="t", instance="i", binding_ref="b").legacy_prefix is False
