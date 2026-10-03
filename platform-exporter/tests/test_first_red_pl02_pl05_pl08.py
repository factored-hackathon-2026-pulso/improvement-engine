"""First RED set (spec 32.5): PL-02 unknown type, PL-03 gap, PL-04 late event, PL-05 as-of, PL-08 simulator rows."""

from datetime import UTC, datetime

from tests.platform_db import add_event


def _events(rig, kind="platform_event"):
    return [e for b in rig.ingest.batches for e in b["events"] if e["kind"] == kind]


def _findings(rig, name):
    return [e for e in _events(rig) if e["source_event"].get("kind") == "exporter_finding"
            and e["source_event"]["finding_code"] == name]


def test_pl02_unknown_event_type_is_counted_quarantined_and_batch_acked(rig):
    add_event(rig.db, 1, "case.opened")
    add_event(rig.db, 2, "team.created", payload={"secret_marker": "DO-NOT-FORWARD"})
    add_event(rig.db, 3, "case.assigned")
    ex = rig.make()
    rep = ex.poll_once()
    assert rep.batches_sent == 1 and not rep.stopped
    assert rep.unknown_event_types == {"team.created": 1}
    (finding,) = _findings(rig, "unknown_event_type")
    assert finding["source_event"]["details"]["event_type"] == "team.created"
    assert finding["source_sequence"] == 2
    assert rig.ingest.cursors[("plat-a.events", "tenant.tenant-1")]["cursor"] == "s.3"  # acked past the unknown row
    assert b"DO-NOT-FORWARD" not in b"".join(rig.ingest.raw_bodies)  # quarantined payload never leaves
    assert [q[2] for q in ex.state.quarantined()] == ["team.created"]
    known = {e["native_event_id"] for e in _events(rig) if e["source_event"].get("event_type", "").startswith("case.")}
    assert known == {"EVT-1", "EVT-3"}


def test_pl03_skipped_sequence_is_gap_suspected_with_backfill_request(rig):
    for seq in (1, 2, 5, 6):
        add_event(rig.db, seq, "case.viewed")
    ex = rig.make()
    rep = ex.poll_once()
    assert rep.gaps == [(3, 4)] and rep.backfill_requests == [(3, 4)]
    (f,) = _findings(rig, "gap_suspected")
    assert f["source_event"]["details"] == {"from_sequence": 3, "to_sequence": 4,
                                            "backfill_requested": True, "verify_with_owner": True}
    assert ex.state.open_backfills() == [(3, 4)]
    # the rows commit late: backfill delivers them as late events and closes the request
    add_event(rig.db, 3, "case.viewed")
    add_event(rig.db, 4, "case.viewed")
    rep2 = ex.poll_once()
    assert ex.state.open_backfills() == []
    late = [e for e in _events(rig) if e["coverage_marker"] == "late" and e["source_sequence"] in (3, 4)]
    assert len(late) == 2 and rep2.batches_sent >= 1


def test_pl04_event_ingested_after_window_close_triggers_window_revision(rig):
    add_event(rig.db, 1, "case.viewed", event_time="2026-03-01T10:30:00Z", ingested_at="2026-03-01T10:31:00Z")
    add_event(rig.db, 2, "case.viewed", event_time="2026-03-01T10:40:00Z", ingested_at="2026-03-01T11:20:00Z")
    rep = rig.make(window_seconds=3600).poll_once()
    assert rep.late_events == ["EVT-2"]
    (f,) = _findings(rig, "late_event")
    assert f["source_event"]["details"]["window_start"] == "2026-03-01T10:00:00Z"
    assert f["source_event"]["details"]["window_end"] == "2026-03-01T11:00:00Z"
    assert f["source_event"]["details"]["window_revision_required"] is True
    by_id = {e["native_event_id"]: e for e in _events(rig)}
    assert by_id["EVT-2"]["coverage_marker"] == "late" and by_id["EVT-1"]["coverage_marker"] is None
    assert by_id["EVT-2"]["source_event"]["available_at"] == "2026-03-01T11:20:00Z"  # available_at = ingested_at


def test_pl05_case_closed_after_cutoff_is_open_without_close_reason_in_extract(rig):
    add_event(rig.db, 1, "case.opened", event_time="2026-03-01T10:00:00Z", payload={"status": "open"})
    add_event(rig.db, 2, "case.assigned", event_time="2026-03-01T10:05:00Z", payload={"analyst_id": "S1"})
    add_event(rig.db, 3, "case.closed", event_time="2026-03-01T12:00:00Z", payload={"close_reason": "resolved"})
    ex = rig.make()
    cut = datetime(2026, 3, 1, 11, 0, tzinfo=UTC)
    before = ex.case_extract(cut)["CASE-1"]  # the mutable cases row already says closed/resolved
    assert before.status == "open" and before.close_reason is None and before.assigned_to == "S1"
    after = ex.case_extract(datetime(2026, 3, 1, 13, 0, tzinfo=UTC))["CASE-1"]
    assert after.status == "closed" and after.close_reason == "resolved"


def test_pl05_extract_excludes_events_not_yet_ingested_at_the_cutoff(rig):
    add_event(rig.db, 1, "case.opened", event_time="2026-03-01T10:00:00Z")
    add_event(rig.db, 2, "case.closed", event_time="2026-03-01T10:30:00Z", ingested_at="2026-03-01T12:30:00Z",
              payload={"close_reason": "resolved"})
    st = rig.make().case_extract(datetime(2026, 3, 1, 11, 0, tzinfo=UTC))["CASE-1"]
    assert st.status == "open" and st.close_reason is None  # known only after ingestion: no future leakage


def test_pl08_simulator_customers_are_team_generated_and_excluded_from_populations(rig):
    rig.db.execute("INSERT INTO cases(id,customer_id,status) VALUES('CASE-SIM','CUS-SIM','open')")
    rig.db.commit()
    add_event(rig.db, 1, "case.opened", case_id="CASE-SIM")
    add_event(rig.db, 2, "case.opened", case_id="CASE-1")
    rig.make().poll_once()
    by_case = {e["source_event"]["case_id"]: e["source_event"] for e in _events(rig)
               if e["source_event"].get("event_type") == "case.opened"}
    assert by_case["CASE-SIM"]["evidence_kind"] == "team_generated" and by_case["CASE-SIM"]["population_excluded"]
    assert by_case["CASE-1"]["evidence_kind"] == "observed" and not by_case["CASE-1"]["population_excluded"]
