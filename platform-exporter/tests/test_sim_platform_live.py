"""Exporter against platform-sim/platform_live (the simulator double of the real platform, Product artifact a492bfa).

Doubles: platform_live simulator (SQLite), ingest fixture (platform-sim/ingest_fixture, in-process ASGI)."""

import json
import sys
from datetime import UTC, datetime
from pathlib import Path

import pytest
from fastapi.testclient import TestClient
from ingest_fixture.app import IngestState, create_app
from platform_live import PlatformLiveSim

from platform_exporter import Exporter, ExporterConfig, ExporterState, SqliteSource
from platform_exporter.catalog import DENIED_EVENT_TYPES, KNOWN_EVENT_TYPES, PLANNED_PREFIXES
from tests.conftest import FakeClock

pytestmark = pytest.mark.sim
CONTRACT_CATALOG = Path(__file__).resolve().parents[2] / "platform-contract" / "event-catalog.json"
sys.path.insert(0, str(CONTRACT_CATALOG.parent))


@pytest.fixture
def sim_rig(tmp_path):
    path = tmp_path / "sim.db"
    sim = PlatformLiveSim(seed=7, path=str(path))
    sim.generate(n_cases=60, faults=True)
    sim.conn.commit()
    ingest = IngestState()
    client = TestClient(create_app(ingest), base_url="http://ingest.fixture")
    clock = FakeClock()
    cfg = ExporterConfig(tenant_id="tenant-1", instance="plat-sim", binding_ref="binding-1", window_seconds=600,
                         gap_grace_seconds=0.0)
    ex = Exporter(cfg, SqliteSource(path), ExporterState(tmp_path / "state.sqlite"), client, clock=clock,
                  sleep=lambda s: clock.advance(s))
    yield sim, ex, ingest
    ex.close()
    sim.conn.close()


def _evs(ingest):
    return [e for b in ingest.batches for e in b["events"]]


def _records(ingest):
    return [{"tenant_id": b["tenant_id"], "source_id": b["source_id"], "native_event_id": e["native_event_id"],
             "source_sequence": e["source_sequence"], "observed_at": e["observed_at"], "source_event": e["source_event"]}
            for b in ingest.batches for e in b["events"]]


def test_simulator_run_conforms_to_contract_1_1_0_and_findings_never_fill_continuity(sim_rig):
    from platform_contract import conformance
    sim, ex, ingest = sim_rig
    ex.poll_once()
    ex.rescan()
    recs = _records(ingest)
    assert conformance.validate_source_observations(recs) == []
    seq_findings = conformance.check_observation_sequences(recs)
    gap = next(f for f in sim.faults if f["kind"] == "sequence_gap")
    assert {"code": "gap_suspected", "after": gap["after"], "next": gap["after"] + gap["size"] + 1} in seq_findings         or any(f["code"] == "gap_suspected" for f in seq_findings)
    assert not [f for f in seq_findings if f["code"] == "finding_sequence_mismatch"]
    _unique, dups = conformance.dedup_observations(recs)
    assert all(d["code"] == "duplicate_identical" for d in dups)


def test_embedded_catalog_matches_the_platform_contract_catalog():
    cat = json.loads(CONTRACT_CATALOG.read_text(encoding="utf-8"))
    by = {s: {e["event_type"] for e in cat["event_types"] if e["status"] == s} for s in ("admitted", "denied")}
    assert by["admitted"] == set(KNOWN_EVENT_TYPES) and by["denied"] == set(DENIED_EVENT_TYPES)
    assert tuple(cat["planned_prefixes"]) == PLANNED_PREFIXES
    from platform_exporter.catalog import CONTRACT_REVISION
    assert cat["contract_version"] == CONTRACT_REVISION
    assert cat["exporter_finding"]["severities"] and set(cat["exporter_finding"]["finding_codes"]) >= {
        "gap_suspected", "late_event", "capability_profile", "dimension_snapshot"}


def test_full_scenario_faults_are_all_surfaced_and_nothing_sensitive_leaves(sim_rig):
    sim, ex, ingest = sim_rig
    rep = ex.poll_once()
    assert not rep.stopped and not rep.errors
    kinds = {f["kind"]: f for f in sim.faults}
    # injected unknown type + the planned teams evolution are quarantined, never failing the batch
    assert rep.unknown_event_types["case.escalated"] == 1
    assert rep.unknown_event_types.get("team.created", 0) >= 1 and rep.unknown_event_types.get("staff.team_changed", 0) >= 1
    # the rolled-back transaction is a suspected gap with a backfill request, matching the injected fault
    gap = kinds["sequence_gap"]
    assert (gap["after"] + 1, gap["after"] + gap["size"]) in rep.gaps
    assert (gap["after"] + 1, gap["after"] + gap["size"]) in rep.backfill_requests
    # the late event reaches Pulso flagged for window revision
    late = kinds["late_event"]
    assert any(e["source_sequence"] == late["sequence"] and e["coverage_marker"] == "late" for e in _evs(ingest))
    assert len(rep.late_events) >= 1
    raw = b"".join(ingest.raw_bodies)
    for forbidden in (b"FAKE-", b"password", b"@", b"display_name"):
        assert forbidden not in raw.replace(b"@@", b""), forbidden


def test_simulator_customers_are_team_generated(sim_rig):
    sim, ex, ingest = sim_rig
    ex.poll_once()
    demo = {r[0] for r in sim.conn.execute("SELECT id FROM customers WHERE simulator=1")}
    cases = {r[0]: r[1] for r in sim.conn.execute("SELECT id, customer_id FROM cases")}
    seen = 0
    for e in _evs(ingest):
        se = e["source_event"]
        if se.get("case_id") in cases:
            seen += 1
            assert (se["evidence_kind"] == "team_generated") == (cases[se["case_id"]] in demo)
            assert se["population_excluded"] == (cases[se["case_id"]] in demo)
    assert seen > 100


def test_rescan_reports_the_injected_turn_sequence_gap(sim_rig):
    sim, ex, ingest = sim_rig
    ex.poll_once()
    case_id = next(f for f in sim.faults if f["kind"] == "turn_gap")["case_id"]
    ex.rescan()
    gaps = [e["source_event"]["details"] for e in _evs(ingest)
            if e["source_event"].get("finding_code") == "turn_sequence_gap"]
    assert any(g["case_id"] == case_id for g in gaps)


def test_extract_matches_simulator_truth_at_cutoff_and_never_leaks_final_state(sim_rig):
    sim, ex, ingest = sim_rig
    row = sim.conn.execute("SELECT case_id, event_time FROM event_log WHERE event_type='case.closed' "
                           "ORDER BY sequence LIMIT 1").fetchone()
    case_id, closed_at = row
    ex_cut = datetime.fromisoformat(closed_at.replace("Z", "+00:00")).astimezone(UTC)
    from datetime import timedelta

    before = ex.case_extract(ex_cut - timedelta(seconds=1))[case_id]
    after = ex.case_extract(ex_cut + timedelta(seconds=5))[case_id]
    final = sim.conn.execute("SELECT status, close_reason FROM cases WHERE id=?", (case_id,)).fetchone()
    assert final[0] == "closed" and final[1]  # the mutable row only shows the final state
    assert before.status != "closed" and before.close_reason is None
    assert after.status == "closed" and after.close_reason == final[1]


def test_exporter_survives_the_teams_evolution_and_profile_reflects_it(sim_rig):
    sim, ex, ingest = sim_rig
    prof = ex.capability_profile()
    assert prof["capabilities"]["teams"]["status"] == "present"
    assert prof["capabilities"]["tool_call"]["status"] == "absent"
    assert prof["capabilities"]["complaint_id"]["status"] == "absent"
    assert "analyst_availability" not in prof["tables_readable"]
    ex.rescan()  # reads the staff dimension after staff.team was replaced by staff.team_id
