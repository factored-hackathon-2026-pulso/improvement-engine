"""Exporter follows platform-contract 1.3.0 (platform 5261ecf, lane SIG1): the suggestion/maturity/case-type events are
admitted without free text, `cases.case_type` is readable, and the real head schema is not reported as drift."""

import json
import sqlite3
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "platform-contract"))

import platform_contract as pc  # noqa: E402

from platform_exporter import SqliteSource, catalog, policy  # noqa: E402
from tests.platform_db import add_event  # noqa: E402
from tests.test_contract_1_2_0 import HEAD_DDL as HEAD_1_2_0  # noqa: E402

# platform 5261ecf: cases.case_type (slice 18), turns.staff_line (slice 23c), assistant_sessions.agent_release (not read).
HEAD_DDL = (HEAD_1_2_0.replace("active_call_id TEXT, version INTEGER);",
                               "active_call_id TEXT, case_type TEXT NOT NULL DEFAULT 'none', version INTEGER);")
            .replace("client_message_id TEXT, subject TEXT);", "client_message_id TEXT, subject TEXT, staff_line TEXT);"))

NEW_TYPES = ["copilot.suggestion_requested", "copilot.suggestion_ready", "copilot.suggestion_none",
             "copilot.suggestion_failed", "copilot.suggestion_decided", "copilot.tool_used", "case.type_changed",
             "ai.stage_advanced", "ai.stage_moved_back", "ai.agent_ready", "ai.agent_activated",
             "platform.ai_toggled"]


def test_exporter_declares_revision_1_3_0():
    assert catalog.CONTRACT_REVISION == pc.CONTRACT_VERSION == "1.3.0"


def test_new_types_are_known_and_mirror_the_contract_catalog():
    assert catalog.KNOWN_EVENT_TYPES == frozenset(pc.ADMITTED_EVENT_TYPES)
    assert set(NEW_TYPES) <= catalog.KNOWN_EVENT_TYPES


def test_enum_whitelist_mirrors_the_contract_catalog():
    cat = json.loads((Path(pc.__file__).resolve().parents[1] / "event-catalog.json").read_text("utf-8"))
    declared = {e["event_type"]: {k: set(v) for k, v in e["payload_enums"].items()}
                for e in cat["event_types"] if "payload_enums" in e}
    assert {t: {k: set(v) for k, v in m.items()} for t, m in catalog.ENUM_PAYLOAD_KEYS.items()} == declared


def test_case_type_is_readable_and_the_real_head_is_not_drift(tmp_path):
    assert "case_type" in policy.ALLOWED_COLUMNS["cases"]
    assert "case_type" not in policy.KNOWN_UNREADABLE["cases"]
    path = tmp_path / "head.db"
    db = sqlite3.connect(path)
    db.executescript(HEAD_DDL)
    db.execute("INSERT INTO cases(id,customer_id,channel,language,priority,case_type) "
               "VALUES('CASE-1','CUS-1','chat_web','es','high','undue_charge')")
    db.execute("INSERT INTO cases(id,customer_id,channel,language,priority,case_type) "
               "VALUES('CASE-2','CUS-2','email','pt','low','none')")
    db.commit()
    db.close()
    src = SqliteSource(path)
    try:
        assert src.unexpected_columns() == {}
        assert src.case_types() == {"CASE-1": "undue_charge", "CASE-2": "none"}
    finally:
        src.close()


def test_case_types_is_empty_on_a_database_that_predates_the_column(tmp_path):
    path = tmp_path / "old.db"
    db = sqlite3.connect(path)
    db.executescript(HEAD_1_2_0)
    db.commit()
    db.close()
    src = SqliteSource(path)
    try:
        assert src.case_types() == {}
    finally:
        src.close()


def test_staff_line_stays_unreadable():
    with pytest.raises(policy.AccessDenied):
        policy.assert_columns_allowed("turns", ["staff_line"])


def _forward(rig, event_type, payload, seq=1):
    add_event(rig.db, seq, event_type, payload=payload)
    assert not rig.make().poll_once().errors
    obs = [e["source_event"] for b in rig.ingest.batches for e in b["events"]
           if e["kind"] == "platform_event" and e["source_event"].get("event_type") == event_type]
    assert len(obs) == 1, f"{event_type} must be admitted, not quarantined"
    return obs[0]


def test_decided_is_forwarded_whole_including_the_enum_named_subject(rig):
    payload = {"subject": "reply", "decision": "edited", "edit_distance_permille": 310, "turn_id": "TRN-9",
               "agent": "copiloto", "release": "REL-4"}
    se = _forward(rig, "copilot.suggestion_decided", payload)
    assert se["payload"] == payload and se["redacted_fields"] == []


def test_subject_with_a_value_outside_the_enum_is_redacted(rig):
    se = _forward(rig, "copilot.suggestion_decided",
                  {"subject": "Re: my card was blocked at the mall", "decision": "used"})
    assert "subject" not in se["payload"] and "mall" not in json.dumps(se)
    assert "subject" in se["redacted_fields"] and se["payload"]["decision"] == "used"


def test_an_enum_value_outside_the_set_is_redacted_for_non_text_named_keys_too(rig):
    se = _forward(rig, "copilot.suggestion_decided", {"subject": "escalation", "decision": "free words here"})
    assert "decision" not in se["payload"] and "decision" in se["redacted_fields"]
    assert se["payload"]["subject"] == "escalation"


def test_subject_is_still_redacted_on_every_other_type(rig):
    se = _forward(rig, "copilot.suggestion_ready", {"analyst_id": "STF-1", "subject": "reply", "count": 1})
    assert "subject" not in se["payload"] and se["payload"]["count"] == 1


@pytest.mark.parametrize("etype,payload", [
    ("copilot.suggestion_requested", {"analyst_id": "STF-1", "trigger": "manual", "based_on_sequence": 4}),
    ("copilot.suggestion_ready", {"analyst_id": "STF-1", "agent": "copiloto", "kinds": ["reply", "tool"], "count": 2,
                                  "truncated": False, "run_id": "run-1", "trace_id": "t-1", "release": "REL-4"}),
    ("copilot.suggestion_none", {"analyst_id": "STF-1", "agent": "copiloto", "run_id": None, "trace_id": "t-1",
                                 "release": None}),
    ("copilot.suggestion_failed", {"analyst_id": "STF-1", "failure_code": "agent_unavailable"}),
    ("copilot.tool_used", {"tool": "leer_pqr_cliente"}),
    ("case.type_changed", {"from": "none", "to": "undue_charge"}),
    ("ai.stage_advanced", {"case_type": "undue_charge", "from_stage": 1, "to_stage": 2}),
    ("ai.stage_moved_back", {"case_type": "undue_charge", "from_stage": 3, "to_stage": 2, "agent_cleared": True}),
    ("ai.agent_ready", {"case_type": "undue_charge"}),
    ("ai.agent_activated", {"case_type": "undue_charge", "agent_id": "disputas"}),
    ("platform.ai_toggled", {"enabled": False}),
    ("assistant.turn_answered", {"agent": "recepcion", "run_id": "r", "awaiting": "none", "status": "ok",
                                 "outcome": None, "trace_id": "t", "messages": 1, "release": "REL-4"}),
])
def test_new_types_are_admitted_and_their_payload_survives_untouched(rig, etype, payload):
    se = _forward(rig, etype, payload)
    assert se["payload"] == payload and se["redacted_fields"] == []


def test_a_row_written_before_release_and_turn_id_existed_is_not_drift(rig):
    se = _forward(rig, "copilot.suggestion_decided", {"subject": "reply", "decision": "discarded"})
    assert se["payload"] == {"subject": "reply", "decision": "discarded"}


def test_turn_created_staff_line_carries_staff_names_and_never_leaves(rig):
    """Platform slice 23c: `turn.created.staff_line.params` holds 'names as they were then' (staff names)."""
    se = _forward(rig, "turn.created", {"kind": "routing", "staff_line": {"kind": "reassigned",
                                                                           "params": {"to": "Ana Real Name"}}})
    assert "staff_line" not in se["payload"] and "Ana Real Name" not in json.dumps(se)
    assert "staff_line" in se["redacted_fields"]
