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
