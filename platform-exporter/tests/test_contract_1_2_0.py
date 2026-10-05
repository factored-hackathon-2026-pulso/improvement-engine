"""Exporter follows platform-contract 1.2.0 (platform eeb73a8): AI/escalation/call events are admitted, free-text
payload keys of those types never leave, and the real head schema is not reported as drift."""

import json
import sqlite3
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "platform-contract"))

import platform_contract as pc  # noqa: E402
from platform_contract import conformance  # noqa: E402

from platform_exporter import SqliteSource, catalog, policy  # noqa: E402
from tests.platform_db import add_event  # noqa: E402

CATALOG = json.loads((Path(pc.__file__).resolve().parents[1] / "event-catalog.json").read_text("utf-8"))
NEW = [e["event_type"] for e in CATALOG["event_types"] if e["status"] == "admitted" and "payload_keys" in e]

# Column names of the platform at eeb73a8 (infrastructure/persistence/sqlalchemy/tables.py).
HEAD_DDL = """
CREATE TABLE customers(id TEXT PRIMARY KEY, display_name TEXT, country TEXT, city TEXT, locale TEXT,
  simulator INTEGER, suggestions TEXT);
CREATE TABLE staff(id TEXT PRIMARY KEY, name TEXT, email TEXT, roles TEXT, languages TEXT, team_id TEXT, active INTEGER,
  created_at TEXT, creation_key TEXT, setup TEXT, version INTEGER);
CREATE TABLE cases(id TEXT PRIMARY KEY, customer_id TEXT, channel TEXT, language TEXT, priority TEXT, status TEXT,
  opened_at TEXT, sla_due_at TEXT, first_response_at TEXT, search_text TEXT, previous_case_id TEXT,
  assigned_analyst_id TEXT, assigned_at TEXT, queued_at TEXT, queue_label TEXT, last_sequence INTEGER,
  last_public_sequence INTEGER, last_message_at TEXT, last_message_author_role TEXT, last_message_preview TEXT,
  last_turn_author_role TEXT, last_turn_preview TEXT, assignee_read_sequence INTEGER, unread_sequences TEXT,
  closed_at TEXT, closed_by_id TEXT, closed_by_role TEXT, close_reason TEXT, close_note TEXT, rating_score INTEGER,
  rating_comment TEXT, rated_at TEXT, rating_key TEXT, open_escalation_id TEXT, active_call_id TEXT, version INTEGER);
CREATE TABLE turns(id TEXT PRIMARY KEY, case_id TEXT, sequence INTEGER, kind TEXT, audience TEXT, author_role TEXT,
  author_id TEXT, text TEXT, language TEXT, created_at TEXT, client_message_id TEXT, subject TEXT);
CREATE TABLE assignments(id TEXT PRIMARY KEY, case_id TEXT, staff_id TEXT, reason TEXT, policy_rule_id TEXT,
  open_cases_at_assignment INTEGER, strategy TEXT, assigned_at TEXT, assigned_by_role TEXT, assigned_by_id TEXT,
  waited_seconds INTEGER, previous_staff_id TEXT, paused_override INTEGER);
CREATE TABLE customer_case_slots(customer_id TEXT PRIMARY KEY, open_case_id TEXT, version INTEGER);
CREATE TABLE event_log(sequence INTEGER PRIMARY KEY, event_id TEXT, event_type TEXT, entity TEXT, entity_id TEXT,
  case_id TEXT, actor_role TEXT, actor_id TEXT, event_time TEXT, ingested_at TEXT, payload TEXT);
"""


def test_exporter_revision_follows_the_contract_package():
    assert catalog.CONTRACT_REVISION == pc.CONTRACT_VERSION == "1.3.0"  # 1.3.0: see test_contract_1_3_0


def test_known_event_types_mirror_the_contract_catalog():
    assert catalog.KNOWN_EVENT_TYPES == frozenset(pc.ADMITTED_EVENT_TYPES)
    assert len(NEW) == 51 and set(NEW) <= catalog.KNOWN_EVENT_TYPES  # 39 (1.2.0) + 12 (1.3.0)


def test_free_text_keys_mirror_the_contract_catalog():
    declared = {e["event_type"]: set(e["free_text_keys"]) for e in CATALOG["event_types"] if "free_text_keys" in e}
    assert {t: set(k) for t, k in catalog.FREE_TEXT_PAYLOAD_KEYS.items()} == declared


def test_real_head_schema_is_not_reported_as_drift(tmp_path):
    path = tmp_path / "head.db"
    db = sqlite3.connect(path)
    db.executescript(HEAD_DDL)
    db.commit()
    db.close()
    src = SqliteSource(path)
    try:
        assert src.unexpected_columns() == {}
    finally:
        src.close()


def test_rating_columns_are_readable_and_free_text_columns_are_not():
    assert {"rating_score", "rated_at"} <= set(policy.ALLOWED_COLUMNS["cases"])
    for col in ("rating_comment", "rating_key", "open_escalation_id", "active_call_id"):
        with pytest.raises(policy.AccessDenied):
            policy.assert_columns_allowed("cases", [col])
    with pytest.raises(policy.AccessDenied):
        policy.assert_columns_allowed("turns", ["subject"])
    with pytest.raises(policy.AccessDenied):
        policy.assert_table_allowed("assistant_sessions")


def _forward(rig, event_type, payload, seq=1):
    add_event(rig.db, seq, event_type, payload=payload)
    ex = rig.make()
    assert not ex.poll_once().errors
    obs = [e["source_event"] for b in rig.ingest.batches for e in b["events"]
           if e["kind"] == "platform_event" and e["source_event"].get("event_type") == event_type]
    assert len(obs) == 1, f"{event_type} must be admitted, not quarantined"
    return obs[0]


@pytest.mark.parametrize("etype,key", [
    ("escalation.opened", "motive"), ("call.started", "reason"), ("assistant.input_queued", "answer"),
    ("escalation.answered", "note"), ("case.rated", "comment"),
])
def test_free_text_key_of_a_new_type_never_leaves(rig, etype, key):
    se = _forward(rig, etype, {key: "SECRET-CUSTOMER-WORDS", "analyst_id": "STF-1"})
    assert key not in se["payload"] and "SECRET-CUSTOMER-WORDS" not in json.dumps(se)
    assert key in se["redacted_fields"] and se["payload"]["analyst_id"] == "STF-1"


def test_ids_and_enums_of_ai_events_are_forwarded(rig):
    se = _forward(rig, "builder.proposal_published", {"agent_id": "recepcion", "release_id": "REL-1", "step_up": True})
    assert se["payload"] == {"agent_id": "recepcion", "release_id": "REL-1", "step_up": True}
    assert se["redacted_fields"] == []
    assert conformance.classify_source_event(se) == "domain_event"


def test_administration_types_stay_quarantined(rig):
    add_event(rig.db, 1, "staff.created", payload={"name": "Ana", "email": "ana@example.invalid"})
    rig.make().poll_once()
    ses = [e["source_event"] for b in rig.ingest.batches for e in b["events"] if e["kind"] == "platform_event"]
    findings = [s for s in ses if s.get("kind") == "exporter_finding" and s["finding_code"] == "unknown_event_type"]
    assert findings and findings[0]["details"]["catalog_status"] == "planned"
    assert findings[0]["details"]["payload_forwarded"] is False
    assert "ana@example.invalid" not in json.dumps(rig.ingest.batches)


def test_free_text_drop_keys_match_case_and_separator_insensitively():
    from platform_exporter.catalog import treat_payload

    red: list[str] = []
    out = treat_payload({"Comment": "x", "answer_text": "y", "keep": 1}, red, drop_keys=frozenset({"comment", "answerText"}))
    assert out == {"keep": 1}, out
