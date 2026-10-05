"""Contract 1.3.0 RED (lane SIG1): support-platform main 5261ecf (2026-10-05) emits the copilot suggestion events,
`copilot.tool_used`, `case.type_changed`, the AI maturity events and `platform.ai_toggled`, and keeps `cases.case_type`.
1.2.0 stays valid (additive). Payloads are ids, enums, counters and flags: no free text is admitted."""

import json
import re
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator

import platform_contract as pc
from platform_contract import conformance, model

ROOT = Path(__file__).resolve().parents[1]
CASE_TYPES = ["none", "unrecognized_charge", "undue_charge", "app_issue", "branch_service", "service_quality",
              "virtual_card"]

NEW_ADMITTED = {
    "copilot.suggestion_requested": "copilot",
    "copilot.suggestion_ready": "copilot",
    "copilot.suggestion_none": "copilot",
    "copilot.suggestion_failed": "copilot",
    "copilot.suggestion_decided": "copilot",
    "copilot.tool_used": "copilot",
    "case.type_changed": "operational",
    "ai.stage_advanced": "maturity",
    "ai.stage_moved_back": "maturity",
    "ai.agent_ready": "maturity",
    "ai.agent_activated": "maturity",
    "platform.ai_toggled": "maturity",
}
NEW_KEYS = {
    "copilot.suggestion_requested": ("analyst_id", "trigger", "based_on_sequence"),
    "copilot.suggestion_ready": ("analyst_id", "agent", "kinds", "count", "truncated", "run_id", "trace_id",
                                 "release"),
    "copilot.suggestion_none": ("analyst_id", "agent", "run_id", "trace_id", "release"),
    "copilot.suggestion_failed": ("analyst_id", "failure_code"),
    "copilot.suggestion_decided": ("subject", "decision", "edit_distance_permille", "turn_id", "agent", "release"),
    "copilot.tool_used": ("tool",),
    "case.type_changed": ("from", "to"),
    "ai.stage_advanced": ("case_type", "from_stage", "to_stage"),
    "ai.stage_moved_back": ("case_type", "from_stage", "to_stage", "agent_cleared"),
    "ai.agent_ready": ("case_type",),
    "ai.agent_activated": ("case_type", "agent_id"),
    "platform.ai_toggled": ("enabled",),
}


def _catalog():
    return json.loads((ROOT / "event-catalog.json").read_text("utf-8"))


def _first(table):
    return json.loads((ROOT / "examples" / f"{table}.valid.json").read_text("utf-8"))[0]


def _errors(table, row):
    return conformance.validate_rows(table, [row])


def test_version_stamp_is_1_3_0_over_1_2_0_against_a_reachable_platform_sha():
    assert pc.CONTRACT_VERSION == "1.3.0"
    assert pc.PREVIOUS_CONTRACT_VERSION == "1.2.0"
    assert pc.ARTIFACT_STAMP["commit"].startswith("5261ecf")
    assert pc.ARTIFACT_STAMP["captured"] == "2026-10-05"
    cat = _catalog()
    assert cat["contract_version"] == "1.3.0" and cat["catalog_version"] == "1.3.0"
    text = (ROOT / "README.md").read_text("utf-8")
    assert "### 1.3.0" in text.split("## Changelog", 1)[1] and "5261ecf" in text


@pytest.mark.parametrize("etype", sorted(NEW_ADMITTED))
def test_new_event_types_are_admitted_with_class_and_exact_payload_keys(etype):
    assert pc.classify_event_type(etype) == "admitted"
    entry = next(e for e in _catalog()["event_types"] if e["event_type"] == etype)
    assert entry["data_class"] == NEW_ADMITTED[etype]
    assert tuple(entry["payload_keys"]) == NEW_KEYS[etype]
    assert "free_text_keys" not in entry, "the platform puts no free text in these payloads"


def test_turn_answered_now_carries_the_release_and_data_classes_gain_maturity():
    by = {e["event_type"]: e for e in _catalog()["event_types"]}
    assert "release" in by["assistant.turn_answered"]["payload_keys"]
    assert set(_catalog()["data_classes"]) == {"operational", "assistant", "copilot", "builder", "maturity"}


def test_the_1_2_0_admitted_set_is_a_subset_and_nothing_else_is_admitted():
    from test_contract_1_2_0 import NEW_ADMITTED as OLD_NEW

    old = {"case.opened", "case.queued", "case.assigned", "case.status_changed", "case.read", "case.first_responded",
           "case.closed", "case.viewed", "turn.created", "staff.availability_changed", "auth.login_failed",
           "auth.account_locked", "auth.session_started", "auth.session_ended"}
    assert set(pc.ADMITTED_EVENT_TYPES) == old | set(OLD_NEW) | set(NEW_ADMITTED)


def test_administration_types_and_credentials_stay_quarantined_or_denied():
    for t in ("staff.created", "staff.invitation_sent", "team.created"):
        assert pc.classify_event_type(t) == "planned", t
    for t in ("copilot.suggestion_shown", "copilot.suggestion_ignored", "release.published"):
        assert pc.classify_event_type(t) == "unknown", t  # `ignored` is a decision value, not an event
    for t in ("auth.password_accepted", "auth.mfa_challenge_issued", "auth.mfa_failed", "customer.session_started"):
        assert pc.classify_event_type(t) == "denied"


def test_decision_and_subject_are_closed_enums_in_the_model():
    assert model.EVENT_PAYLOAD_ENUMS["copilot.suggestion_decided"]["subject"] == ("reply", "escalation")
    assert model.EVENT_PAYLOAD_ENUMS["copilot.suggestion_decided"]["decision"] == (
        "used", "edited", "discarded", "ignored", "accepted")
    assert model.EVENT_PAYLOAD_ENUMS["copilot.suggestion_requested"]["trigger"] == (
        "customer_message", "manual", "handover")
    entry = next(e for e in _catalog()["event_types"] if e["event_type"] == "copilot.suggestion_decided")
    assert entry["payload_enums"]["decision"] == ["used", "edited", "discarded", "ignored", "accepted"]


def _tokens(key):
    return {t for t in re.split(r"[^a-z0-9]+", key.lower()) if t}


def test_new_types_carry_no_text_shaped_key_except_the_declared_enums():
    texty = {"text", "body", "message", "motive", "note", "comment", "reason", "answer", "name", "email", "question",
             "summary", "title", "description", "draft", "evidence"}
    for t, keys in NEW_KEYS.items():
        enums = model.EVENT_PAYLOAD_ENUMS.get(t, {})
        for k in keys:
            if _tokens(k) & texty:
                assert k in enums, f"{t}.{k}"
    # `subject` is the one text-looking key: a closed two-value enum, never a subject line
    assert "subject" in model.EVENT_PAYLOAD_ENUMS["copilot.suggestion_decided"]


def test_cases_case_type_is_an_optional_closed_enum_so_1_2_0_rows_stay_valid():
    schema = pc.load_schema("cases")
    assert "case_type" in schema["properties"] and "case_type" not in schema["required"]
    assert schema["properties"]["case_type"]["enum"] == CASE_TYPES
    base = _first("cases")
    assert _errors("cases", base) == []
    for v in CASE_TYPES:
        assert _errors("cases", dict(base, case_type=v)) == []
    assert _errors("cases", dict(base, case_type="free text about a customer")) != []
    assert _errors("cases", dict(base, case_type=None)) != [], "the platform column is NOT NULL"


def test_denied_columns_and_tables_are_unchanged():
    cases = pc.load_schema("cases")
    assert {"rating_comment", "rating_key", "search_text"} <= set(cases["x-denied-columns"])
    assert "staff_line" not in pc.load_schema("turns")["properties"]
    for t in ("login_accounts", "mfa_challenges", "staff_sessions", "copilot_suggestions", "case_type_maturity",
              "platform_settings"):
        with pytest.raises(pc.TableRefused):
            pc.assert_table_readable(t)


def test_new_event_rows_validate_and_flow_through_the_stream_check():
    base = _first("event_log")
    rows = [dict(base, sequence=i, event_id=f"EVT-{i}", event_type=t, payload={}, actor_role="system")
            for i, t in enumerate(sorted(NEW_ADMITTED), start=300)]
    assert conformance.validate_rows("event_log", rows) == []
    assert conformance.check_event_stream(rows) == []


def test_schemas_are_stamped_1_3_0_and_valid():
    for t in pc.ALLOWED_TABLES:
        s = pc.load_schema(t)
        Draft202012Validator.check_schema(s)
        assert s["x-contract-version"] == "1.3.0" and "/1.3.0/" in s["$id"]


def test_1_2_0_example_rows_still_validate():
    for t in ("cases", "turns", "assignments", "customers", "event_log", "customer_case_slots", "staff_dimensions"):
        rows = json.loads((ROOT / "examples" / f"{t}.valid.json").read_text("utf-8"))
        assert conformance.validate_rows(t, rows) == [], t


def test_turn_created_staff_line_is_declared_free_text():
    """Platform slice 23c puts staff names in `turn.created.staff_line.params`: dropped like `text` and `subject`."""
    entry = next(e for e in _catalog()["event_types"] if e["event_type"] == "turn.created")
    assert set(entry["free_text_keys"]) == {"text", "subject", "staff_line"}
