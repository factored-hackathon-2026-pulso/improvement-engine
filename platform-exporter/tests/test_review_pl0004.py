"""Independent review (PL-0004) regression tests: each was written RED before its fix."""

import json
import os

import psycopg
import pytest

from tests.platform_db import add_event


def _raw(rig) -> bytes:
    return b"".join(rig.ingest.raw_bodies)


def _ev(rig):
    return [e for b in rig.ingest.batches for e in b["events"]]


PII_PAYLOAD = {
    "customer_email": "jane@corp.example", "full_name": "FULLNAME-X", "first_name": "FIRSTNAME-X",
    "lastName": "LASTNAME-X", "phoneNumber": "+57-300-PHONE", "message_preview": "PREVIEW-X", "subject": "SUBJECT-X",
    "comment": "COMMENT-X", "username": "USERNAME-X", "snippet": "SNIPPET-X", "address": "ADDRESS-X",
    "ref": "please contact bob@evil.example today", "nested": {"items": [{"Email": "x@y.zz"}, {"note_text": "NOTE-X"}]},
    "turn_sequence": 3, "close_reason": "resolved", "priority": "high", "staff_id": "S1", "queue_label": "es",
}


def test_payload_pii_under_variant_keys_and_values_never_leaves(rig):
    add_event(rig.db, 1, "turn.created", payload=PII_PAYLOAD)
    rig.make().poll_once()
    raw = _raw(rig)
    for marker in (b"jane@corp", b"FULLNAME-X", b"FIRSTNAME-X", b"LASTNAME-X", b"PHONE", b"PREVIEW-X", b"SUBJECT-X",
                   b"COMMENT-X", b"USERNAME-X", b"SNIPPET-X", b"ADDRESS-X", b"bob@evil", b"x@y.zz", b"NOTE-X"):
        assert marker not in raw, marker
    se = next(e for e in _ev(rig) if e["native_event_id"] == "EVT-1")["source_event"]
    for kept in ("turn_sequence", "close_reason", "priority", "staff_id", "queue_label"):
        assert kept in se["payload"], kept  # structural, non-PII fields survive


def test_non_object_payload_is_quarantined_not_forwarded(rig):
    rig.db.execute("INSERT INTO event_log(sequence,event_id,event_type,case_id,event_time,ingested_at,payload) VALUES"
                   "(1,'EVT-1','case.opened','CASE-1','2026-03-01T10:00:00Z','2026-03-01T10:00:00Z',?)",
                   (json.dumps("FREE-TEXT-SECRET"),))
    rig.db.commit()
    ex = rig.make()
    rep = ex.poll_once()
    assert rep.batches_sent == 1 and not rep.stopped
    assert b"FREE-TEXT-SECRET" not in _raw(rig)
    assert [q[3] for q in ex.state.quarantined()] == ["payload_not_object"]


def test_single_oversized_event_is_quarantined_and_does_not_stop_the_partition(rig):
    add_event(rig.db, 1, "case.opened")
    add_event(rig.db, 2, "case.assigned", payload={"staff_id": "S1", "blob": "x" * 700_000})
    add_event(rig.db, 3, "case.viewed")
    ex = rig.make()
    rep = ex.poll_once()
    assert not rep.stopped, rep.stopped
    assert ex.state.cursor("tenant.tenant-1")[0] == "s.3"
    assert [q[3] for q in ex.state.quarantined()] == ["oversized_event"]
    assert b"xxxxxxxxxx" not in _raw(rig)


ADMIN = os.environ.get("PULSO_TEST_PG_ADMIN")


@pytest.mark.pg
@pytest.mark.skipif(not ADMIN, reason="PULSO_TEST_PG_ADMIN not set")
def test_postgres_source_connection_refuses_writes_even_for_a_privileged_role():
    from platform_exporter import PostgresSource

    src = PostgresSource(ADMIN)
    try:
        with pytest.raises(psycopg.errors.ReadOnlySqlTransaction):
            src._conn.execute("CREATE TABLE exporter_must_not_write(a int)")
    finally:
        src.close()


# ---- mutation-driven tests (each kills a surviving mutant found in the review) -------------------------------------
from datetime import UTC, datetime  # noqa: E402

from platform_exporter import SimulatedCrash  # noqa: E402
from platform_exporter.asof import reconstruct_cases  # noqa: E402
from platform_exporter.source import RawEvent  # noqa: E402


def test_asof_excludes_event_whose_event_time_is_after_cutoff_even_if_ingested_earlier(rig):
    add_event(rig.db, 1, "case.opened", event_time="2026-03-01T10:00:00Z")
    add_event(rig.db, 2, "case.closed", event_time="2026-03-01T12:00:00Z", ingested_at="2026-03-01T10:30:00Z",
              payload={"close_reason": "resolved"})  # platform clock skew: stamped in the future, ingested early
    st = rig.make().case_extract(datetime(2026, 3, 1, 11, 0, tzinfo=UTC))["CASE-1"]
    assert st.status == "open" and st.close_reason is None


def test_asof_reopen_clears_close_reason(rig):
    add_event(rig.db, 1, "case.opened", event_time="2026-03-01T10:00:00Z")
    add_event(rig.db, 2, "case.closed", event_time="2026-03-01T10:10:00Z", payload={"close_reason": "resolved"})
    add_event(rig.db, 3, "case.status_changed", event_time="2026-03-01T10:20:00Z", payload={"to": "open"})
    st = rig.make().case_extract(datetime(2026, 3, 1, 11, 0, tzinfo=UTC))["CASE-1"]
    assert st.status == "open" and st.close_reason is None and st.closed_at is None


def _raw_ev(seq, et, ing, etype="case.closed", payload=None):
    return RawEvent(seq, f"EVT-{seq}", etype, "case", "CASE-1", "CASE-1", "analyst", "S1", et, ing, payload or {}, None)


def test_asof_is_independent_of_input_order():
    t = lambda m: datetime(2026, 3, 1, 10, m, tzinfo=UTC)  # noqa: E731
    evs = [_raw_ev(2, t(10), t(10), "case.closed", {"close_reason": "resolved"}), _raw_ev(1, t(0), t(0), "case.opened")]
    st = reconstruct_cases(evs, t(30))["CASE-1"]
    assert st.status == "closed" and st.opened_at == t(0)


def test_start_sequence_defines_the_first_expected_row_and_leading_gap(rig):
    for seq in (3, 4):
        add_event(rig.db, seq, "case.viewed")
    rep = rig.make(start_sequence=1).poll_once()
    assert rep.gaps == [(1, 2)]  # rows 1..2 were never seen: gap, not silence


def test_start_sequence_skips_history_before_it_without_a_gap(rig, tmp_path):
    for seq in (1, 2, 3, 4):
        add_event(rig.db, seq, "case.viewed")
    ex = rig.make(start_sequence=3)
    rep = ex.poll_once()
    assert rep.gaps == [] and ex.state.cursor("tenant.tenant-1")[0] == "s.4"
    sent = {e["source_sequence"] for e in _ev(rig) if e["source_sequence"] is not None}
    assert sent == {3, 4}


def test_late_boundary_ingested_exactly_at_window_end_is_not_late_and_lateness_is_honoured(rig):
    add_event(rig.db, 1, "case.viewed", event_time="2026-03-01T10:30:00Z", ingested_at="2026-03-01T11:00:00Z")
    add_event(rig.db, 2, "case.viewed", event_time="2026-03-01T10:30:00Z", ingested_at="2026-03-01T11:04:00Z")
    add_event(rig.db, 3, "case.viewed", event_time="2026-03-01T10:30:00Z", ingested_at="2026-03-01T11:06:00Z")
    rep = rig.make(window_seconds=3600, allowed_lateness_seconds=300).poll_once()
    assert rep.late_events == ["EVT-3"]  # end (11:00) + 300s lateness = 11:05; exactly-at-end and within lateness are on time
    ex0 = rig.make(state_path=rig.tmp / "s2.sqlite", window_seconds=3600, allowed_lateness_seconds=0)
    assert ex0._is_late(_raw_ev(9, datetime(2026, 3, 1, 10, 30, tzinfo=UTC), datetime(2026, 3, 1, 11, 0, tzinfo=UTC)))[0] is False
    assert ex0._is_late(_raw_ev(9, datetime(2026, 3, 1, 10, 30, tzinfo=UTC), datetime(2026, 3, 1, 11, 0, 1, tzinfo=UTC)))[0] is True


def test_ack_for_another_digest_stops_the_partition_and_keeps_local_state(rig):
    add_event(rig.db, 1, "case.viewed")
    ex = rig.make()
    real_post = ex.client.post

    class _R:
        status_code = 202
        headers: dict = {}

        def __init__(self, ack):
            self._ack = ack

        def json(self):
            return self._ack

    def evil(url, **kw):
        r = real_post(url, **kw)
        return _R({**r.json(), "batch_digest": "0" * 64}) if "observations" in url else r

    ex.client.post = evil  # type: ignore[method-assign]
    rep = ex.poll_once()
    assert rep.stopped == {"tenant.tenant-1": "ack_digest_mismatch"}
    assert ex.state.cursor("tenant.tenant-1") in (None, (None, 0))  # nothing committed on a foreign ack
    assert ex.state.pending() is not None


def test_crash_after_ack_replays_the_stored_bytes_even_if_the_world_changed(rig, tmp_path):
    for i in (1, 2, 3):
        add_event(rig.db, i, "case.viewed")
    ex = rig.make()

    def boom():
        raise SimulatedCrash

    ex.crash_after_ack = boom
    with pytest.raises(SimulatedCrash):
        ex.poll_once()
    stored = ex.state.pending().body
    rig.clock.advance(3600)  # a rebuilt batch would carry other observed_at values -> another digest
    add_event(rig.db, 4, "case.viewed")
    ex2 = rig.make(state_path=tmp_path / "state.sqlite")
    rep = ex2.poll_once()
    assert rep.duplicate_acks == 1  # the persisted batch was re-POSTed verbatim and recognised
    assert stored in rig.ingest.raw_bodies
    assert ex2.state.cursor("tenant.tenant-1")[0] == "s.4"
