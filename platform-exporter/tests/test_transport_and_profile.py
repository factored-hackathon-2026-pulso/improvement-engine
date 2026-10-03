"""Transport (retry until durable ack, crash safety, cursor authority), privacy treatment, drift and the profile."""

import sqlite3

import pytest

from platform_exporter import SimulatedCrash, SqliteSource, build_profile
from tests.platform_db import add_event


def _ev(rig):
    return [e for b in rig.ingest.batches for e in b["events"]]


def _seed(rig, n=3):
    for i in range(1, n + 1):
        add_event(rig.db, i, "case.viewed")


def test_batch_is_idempotent_by_digest_and_cursor_advances_once(rig):
    _seed(rig)
    ex = rig.make()
    rep = ex.poll_once()
    assert rep.batches_sent >= 1 and rep.duplicate_acks == 0
    n = len(rig.ingest.batches)
    assert ex.poll_once().batches_sent == 0 and len(rig.ingest.batches) == n  # nothing new, nothing re-sent
    assert rig.ingest.raw_bodies[0].count(b'"batch_digest"') == 1


def test_503_and_429_are_retried_with_backoff_until_durable_ack(rig):
    _seed(rig)
    rig.ingest.fail_next.extend([(503, {"Retry-After": "1"}), (429, {}), (503, {})])
    ex = rig.make()
    t0 = rig.clock.now
    rep = ex.poll_once()
    assert rep.batches_sent >= 1 and not rep.stopped
    assert (rig.clock.now - t0).total_seconds() >= 3  # injected sleeps advanced the fake clock


def test_crash_after_ack_before_commit_does_not_double_count(rig, tmp_path):
    _seed(rig)
    ex = rig.make()

    def boom():
        raise SimulatedCrash

    ex.crash_after_ack = boom
    with pytest.raises(SimulatedCrash):
        ex.poll_once()
    committed = len(rig.ingest.batches)
    ex2 = rig.make(state_path=tmp_path / "state.sqlite")  # restart on the same state: stored batch is re-POSTed
    rep = ex2.poll_once()
    assert rep.duplicate_acks == 1 and len(rig.ingest.batches) == committed
    assert len({e["native_event_id"] for e in _ev(rig) if e["source_sequence"]}) == 3
    assert ex2.state.cursor("tenant.tenant-1")[0] == "s.3"


def test_lost_local_state_recovers_cursor_from_pulso(rig, tmp_path):
    _seed(rig)
    rig.make().poll_once()
    fresh = rig.make(state_path=tmp_path / "other-state.sqlite")
    n = rig.ingest.count()
    fresh.poll_once()
    assert rig.ingest.count() == n  # Pulso cursor wins: only duplicate (profile) rows, nothing replayed
    assert fresh.state.meta("rebuild") == "1"


def test_422_quarantines_the_batch_and_stops_the_partition(rig):
    _seed(rig, 1)
    rig.ingest.fail_next.append((422, {}))
    rep = rig.make().poll_once()
    assert rep.stopped == {"tenant.tenant-1": "quarantined:injected"} and rep.batches_sent == 0


def test_payload_text_names_and_credentials_never_leave(rig):
    add_event(rig.db, 1, "turn.created", payload={"turn_sequence": 1, "text": "SECRET-MESSAGE",
                                                  "author": {"name": "Real Name", "role": "customer"},
                                                  "password_hash": "x"})
    rig.make().poll_once()
    raw = b"".join(rig.ingest.raw_bodies)
    assert b"SECRET-MESSAGE" not in raw and b"Real Name" not in raw and b'"password_hash":' not in raw
    se = next(e for e in _ev(rig) if e["native_event_id"] == "EVT-1")["source_event"]
    assert se["payload"] == {"turn_sequence": 1, "author": {"role": "customer"}}
    assert sorted(se["redacted_fields"]) == ["author.name", "password_hash", "text"]


def test_poll_never_touches_denied_tables_or_state_columns(rig):
    from platform_exporter.policy import ALLOWED_COLUMNS, install_sqlite_guard

    _seed(rig)
    ex = rig.make()
    reads: list[tuple[str, str]] = []
    install_sqlite_guard(ex.source._db, on_read=lambda t, c: reads.append((t, c)))
    ex.poll_once()
    ex.rescan()
    assert reads
    for table, column in reads:
        if table in ("sqlite_master", "sqlite_schema"):
            continue
        assert table in ALLOWED_COLUMNS, table
        assert not column or column in ALLOWED_COLUMNS[table], (table, column)
    assert not {"status", "close_reason", "assigned_analyst_id", "closed_at"} & {c for _, c in reads}


def test_schema_drift_missing_required_column_is_reported_not_raised(tmp_path, rig):
    other = tmp_path / "drift.db"
    db = sqlite3.connect(other)
    db.executescript("CREATE TABLE event_log(sequence INTEGER PRIMARY KEY, event_id TEXT, event_type TEXT);")
    db.close()
    ex = rig.make()
    ex.source = SqliteSource(other)
    assert ex.poll_once().errors == ["pulso:schema_drift"]


def test_unexpected_extra_columns_are_tolerated_and_named_in_the_profile(rig):
    rig.db.execute("ALTER TABLE event_log ADD COLUMN new_future_column TEXT")
    rig.db.commit()
    _seed(rig, 1)
    ex = rig.make()
    assert ex.poll_once().batches_sent >= 1
    assert ex.capability_profile()["unexpected_columns"]["event_log"] == ["new_future_column"]


def test_capability_profile_phase1_and_superset(rig):
    ex = rig.make()
    prof = ex.capability_profile()
    assert prof["profile_version"] == "platform_live.phase1/1" and prof["phase"] == 1
    for cap in ("origin", "topic", "complaint_id", "identity_check", "routing_step", "copilot_query", "tool_call",
                "approval", "suggestion", "case_close.resolved", "csat"):
        assert prof["capabilities"][cap]["status"] == "absent"
    assert prof["channels_declared"] == ["app_chat", "web_chat"]
    assert prof["tables_denied_by_construction"] == ["login_accounts", "mfa_challenges", "staff_sessions"]
    assert "login_accounts" not in prof["tables_readable"]
    rig.db.execute("CREATE TABLE tool_call(id TEXT)")
    rig.db.execute("ALTER TABLE cases ADD COLUMN complaint_id TEXT")
    rig.db.commit()
    sup = build_profile(SqliteSource(rig.db_path))
    assert sup["capabilities"]["tool_call"]["status"] == "present"
    assert sup["capabilities"]["complaint_id"]["status"] == "present"
    assert sup["digest"] != prof["digest"]


def test_profile_manifest_is_emitted_once_per_digest(rig):
    _seed(rig)
    ex = rig.make()
    ex.poll_once()
    profs = [e for e in _ev(rig) if e["source_event"]["event_type"] == "exporter.capability_profile"]
    assert len(profs) == 1
    add_event(rig.db, 4, "case.viewed")
    ex.poll_once()
    assert len([e for e in _ev(rig) if e["source_event"]["event_type"] == "exporter.capability_profile"]) == 1


def test_gap_is_held_back_during_grace_then_declared(rig):
    for seq in (1, 2, 5):
        add_event(rig.db, seq, "case.viewed")
    ex = rig.make(gap_grace_seconds=120)
    rep = ex.poll_once()
    assert rep.gaps == [] and ex.state.cursor("tenant.tenant-1")[0] == "s.2"  # 5 is held back
    rig.clock.advance(121)
    rep2 = ex.poll_once()
    assert rep2.gaps == [(3, 4)] and ex.state.cursor("tenant.tenant-1")[0] == "s.5"


def test_manual_backfill_by_sequence_marks_events_late_and_dedupes(rig):
    _seed(rig)
    ex = rig.make()
    ex.poll_once()
    before = rig.ingest.count()
    rep = ex.backfill(1, 3)
    assert rep.batches_sent == 1 and rig.ingest.count() == before  # same ids and digests: all duplicates


def test_rescan_emits_dimension_snapshot_and_turn_gap_findings(rig):
    rig.db.executemany("INSERT INTO turns(id,case_id,sequence) VALUES(?,?,?)",
                       [("T1", "CASE-1", 1), ("T3", "CASE-1", 3)])
    rig.db.commit()
    ex = rig.make()
    rep = ex.rescan()
    assert rep.batches_sent == 1
    types = {e["source_event"]["event_type"] for e in _ev(rig)}
    assert {"exporter.dimension_snapshot", "exporter.capability_profile", "exporter.finding"} <= types
    gap = next(e for e in _ev(rig) if e["source_event"].get("finding", {}).get("type") == "turn_sequence_gap")
    assert gap["source_event"]["finding"]["missing"] == [2]
    assert rig.ingest.cursors == {}  # rescan never advances the checkpoint
