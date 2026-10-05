"""Contract 1.2.0 RED: the platform head (support-platform eeb73a8, 2026-10-04) added the assistant, copilot,
agent-builder, escalation and call vocabulary. 1.1.0 rows stay readable (additive); denied columns stay denied."""

import copy
import json
import re
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator

import platform_contract as pc
from platform_contract import conformance, model

ROOT = Path(__file__).resolve().parents[1]


def _first(table):
    return json.loads((ROOT / "examples" / f"{table}.valid.json").read_text("utf-8"))[0]


def _errors(table, row):
    return conformance.validate_rows(table, [row])


def test_version_stamp_is_1_2_0_against_a_reachable_platform_sha():
    assert pc.CONTRACT_VERSION == "1.2.0"
    assert pc.PREVIOUS_CONTRACT_VERSION == "1.1.0"
    assert pc.ARTIFACT_STAMP["commit"].startswith("eeb73a8")
    assert pc.ARTIFACT_STAMP["captured"] == "2026-10-04"
    assert pc.ARTIFACT_STAMP["unreachable_commits"]["a492bfa"]
    readme = (ROOT / "README.md").read_text("utf-8")
    assert "eeb73a8" in readme and "a492bfa" in readme and "unreachable" in readme.lower()
    assert "inference" in readme.lower()


@pytest.mark.parametrize("field,value", [
    ("status", "with_assistant"),
    ("channel", "chat_app"), ("channel", "chat_web"), ("channel", "phone_inbound"),
    ("channel", "phone_outbound"), ("channel", "email"),
    ("priority", "none"), ("priority", "critical"),
    ("last_message_author_role", "assistant"), ("last_turn_author_role", "assistant"),
    ("closed_by_role", "assistant"),
])
def test_new_case_enum_values_are_admitted(field, value):
    assert _errors("cases", dict(_first("cases"), **{field: value})) == []


@pytest.mark.parametrize("field,value", [
    ("kind", "transcript"), ("kind", "note"), ("kind", "email"), ("author_role", "assistant"),
])
def test_new_turn_enum_values_are_admitted(field, value):
    assert _errors("turns", dict(_first("turns"), **{field: value})) == []


@pytest.mark.parametrize("value", ["outbound_call", "assistant_handoff"])
def test_new_assignment_reasons_are_admitted(value):
    assert _errors("assignments", dict(_first("assignments"), reason=value)) == []


def test_event_actor_role_admits_assistant():
    assert _errors("event_log", dict(_first("event_log"), actor_role="assistant")) == []


def test_1_1_0_values_stay_readable():
    for field, value in (("channel", "app_chat"), ("channel", "web_chat"), ("status", "queued"),
                         ("priority", "low")):
        assert _errors("cases", dict(_first("cases"), **{field: value})) == []
    assert _errors("turns", dict(_first("turns"), author_role="analyst", kind="routing")) == []
    assert _errors("assignments", dict(_first("assignments"), reason="queue_drained")) == []


def test_unknown_enum_values_are_still_rejected():
    assert _errors("cases", dict(_first("cases"), status="with_robot")) != []
    assert _errors("cases", dict(_first("cases"), channel="fax")) != []


def test_new_case_columns_are_optional_for_1_1_0_rows_and_typed_when_present():
    base = _first("cases")
    for col in ("rating_score", "rated_at", "open_escalation_id", "active_call_id"):
        assert col in pc.load_schema("cases")["properties"], col
        assert col not in pc.load_schema("cases")["required"], "additive: 1.1.0 rows have no such column"
    assert _errors("cases", base) == []
    assert _errors("cases", dict(base, rating_score=4, rated_at="2026-10-04T15:00:00Z")) == []
    assert _errors("cases", dict(base, rating_score=5)) != [], "scores are 1-4 on the platform"
    assert _errors("cases", dict(base, rating_score=0)) != []
    assert _errors("cases", dict(base, open_escalation_id="ESC-1", active_call_id="CAL-1")) == []


def test_free_text_columns_of_the_new_slices_stay_denied():
    cases = pc.load_schema("cases")
    assert {"rating_comment", "rating_key"} <= set(cases["x-denied-columns"])
    assert not {"rating_comment", "rating_key", "search_text"} & set(cases["properties"])
    assert "subject" in pc.load_schema("turns")["x-denied-columns"]
    assert "subject" not in pc.load_schema("turns")["properties"]
    assert _errors("cases", dict(_first("cases"), rating_comment="great")) != []


def test_denied_tables_stay_denied_and_new_tables_are_not_allow_listed():
    for t in ("login_accounts", "mfa_challenges", "staff_sessions"):
        with pytest.raises(pc.TableRefused):
            pc.assert_table_readable(t)
    for t in ("assistant_sessions", "copilot_threads", "builder_threads", "builder_proposals",
              "bank_customer_links", "escalations", "calls", "notifications", "invitations", "password_resets",
              "dev_mailbox"):
        with pytest.raises(pc.TableRefused) as e:
            pc.assert_table_readable(t)
        assert e.value.category == "not_allowlisted", t


NEW_ADMITTED = {
    "case.priority_changed": "operational",
    "case.rated": "operational",
    "case.assistant_started": "assistant",
    "case.assistant_released": "assistant",
    "assistant.session_started": "assistant",
    "assistant.input_queued": "assistant",
    "assistant.turn_answered": "assistant",
    "assistant.step_up_verified": "assistant",
    "assistant.step_up_rejected": "assistant",
    "assistant.ended": "assistant",
    "copilot.query_asked": "copilot",
    "copilot.answered": "copilot",
    "builder.proposal_created": "builder",
    "builder.proposal_tracked": "builder",
    "builder.draft_saved": "builder",
    "builder.proposal_validated": "builder",
    "builder.proposal_frozen": "builder",
    "builder.proposal_reopened": "builder",
    "builder.proposal_evaluated": "builder",
    "builder.proposal_approved": "builder",
    "builder.proposal_rejected": "builder",
    "builder.proposal_published": "builder",
    "builder.alias_promoted": "builder",
    "builder.release_revoked": "builder",
    "builder.question_asked": "builder",
    "builder.answered": "builder",
    "escalation.opened": "operational",
    "escalation.withdrawn": "operational",
    "escalation.answered": "operational",
    "escalation.taken": "operational",
    "escalation.reassigned": "operational",
    "escalation.closed": "operational",
    "escalation.acknowledged": "operational",
    "call.started": "operational",
    "call.answered": "operational",
    "call.held": "operational",
    "call.resumed": "operational",
    "call.mute_changed": "operational",
    "call.ended": "operational",
}


def _catalog():
    return json.loads((ROOT / "event-catalog.json").read_text("utf-8"))


@pytest.mark.parametrize("etype", sorted(NEW_ADMITTED))
def test_new_event_types_are_admitted_with_a_data_class(etype):
    assert pc.classify_event_type(etype) == "admitted"
    entry = next(e for e in _catalog()["event_types"] if e["event_type"] == etype)
    assert entry["data_class"] == NEW_ADMITTED[etype]


def test_catalog_declares_data_classes_and_version():
    cat = _catalog()
    assert cat["contract_version"] == "1.2.0" and cat["catalog_version"] == "1.2.0"
    assert set(cat["data_classes"]) == {"operational", "assistant", "copilot", "builder"}
    assert all("data_class" in e for e in cat["event_types"] if e["status"] == "admitted")


def test_1_1_0_admitted_and_denied_types_are_unchanged():
    old = {"case.opened", "case.queued", "case.assigned", "case.status_changed", "case.read", "case.first_responded",
           "case.closed", "case.viewed", "turn.created", "staff.availability_changed", "auth.login_failed",
           "auth.account_locked", "auth.session_started", "auth.session_ended"}
    assert old <= set(pc.ADMITTED_EVENT_TYPES)
    assert set(pc.ADMITTED_EVENT_TYPES) == old | set(NEW_ADMITTED)
    for t in ("auth.password_accepted", "auth.mfa_challenge_issued", "auth.mfa_failed", "customer.session_started"):
        assert pc.classify_event_type(t) == "denied"


def test_administration_and_unknown_types_stay_quarantined():
    for t in ("staff.created", "staff.invitation_sent", "staff.mfa_enrolled", "team.created", "staff.team_changed"):
        assert pc.classify_event_type(t) == "planned", t
    for t in ("release.published", "brand.new", "exporter.finding"):
        assert pc.classify_event_type(t) == "unknown", t


# `case.assistant_released.reason` is the closed enum escalated|ended|failed|supervision (domain/cases/events.py).
ENUM_KEYS = {("case.assistant_released", "reason"), ("call.ended", "end_reason")}
FREE_TEXT_TOKENS = {"text", "body", "message", "motive", "note", "comment", "reason", "answer", "subject", "name",
                    "email", "question", "summary", "title", "description"}


def _tokens(key):
    return {t for t in re.split(r"[^a-z0-9]+", key.lower()) if t}


def test_new_types_carry_ids_enums_and_counters_only():
    """Every declared payload key that looks like free text is listed in `free_text_keys` (dropped by the exporter);
    nothing else may be a text token. `*_length`/`*_id`/`*_ref` markers are sizes/ids and are fine."""
    for e in _catalog()["event_types"]:
        if e["event_type"] not in NEW_ADMITTED:
            continue
        keys = e.get("payload_keys")
        assert isinstance(keys, list) and keys, e["event_type"]
        free = set(e.get("free_text_keys", []))
        assert free <= set(keys)
        for k in keys:
            toks = _tokens(k)
            sizeish = (k.endswith(("_length", "_id", "_ref", "_count")) or k in ("messages", "items", "attempts")
                       or (e["event_type"], k) in ENUM_KEYS)
            if toks & FREE_TEXT_TOKENS and not sizeish:
                assert k in free, f"{e['event_type']}.{k} looks like free text but is not in free_text_keys"


@pytest.mark.parametrize("etype,key", [
    ("escalation.opened", "motive"), ("escalation.answered", "note"), ("call.started", "reason"),
    ("assistant.input_queued", "answer"), ("case.rated", "comment"),
])
def test_known_free_text_payload_keys_are_declared(etype, key):
    entry = next(e for e in _catalog()["event_types"] if e["event_type"] == etype)
    assert key in entry["free_text_keys"]


def test_builder_events_carry_release_identity_where_the_platform_emits_it():
    by = {e["event_type"]: e for e in _catalog()["event_types"]}
    assert {"agent_id", "release_id"} <= set(by["builder.proposal_published"]["payload_keys"])
    assert {"alias", "release_id"} <= set(by["builder.alias_promoted"]["payload_keys"])


def test_new_event_rows_validate_and_flow_through_the_stream_check():
    base = _first("event_log")
    rows = []
    for i, t in enumerate(sorted(NEW_ADMITTED), start=100):
        rows.append(dict(base, sequence=i, event_id=f"EVT-{i}", event_type=t, payload={}, actor_role="assistant"))
    assert conformance.validate_rows("event_log", rows) == []
    assert conformance.check_event_stream(rows) == []


def test_1_1_0_example_rows_still_validate_under_1_2_0():
    for t in ("cases", "turns", "assignments", "customers", "event_log", "customer_case_slots", "staff_dimensions"):
        rows = json.loads((ROOT / "examples" / f"{t}.valid.json").read_text("utf-8"))
        assert conformance.validate_rows(t, rows) == [], t


def test_schemas_are_stamped_1_2_0_and_valid():
    for t in pc.ALLOWED_TABLES:
        s = pc.load_schema(t)
        Draft202012Validator.check_schema(s)
        assert s["x-contract-version"] == "1.2.0" and "/1.2.0/" in s["$id"]


def test_changelog_documents_the_revision():
    text = (ROOT / "README.md").read_text("utf-8")
    assert "## Changelog" in text and "1.2.0" in text.split("## Changelog", 1)[1]
    assert "with_assistant" in text and "a492bfa" in text


def test_model_exposes_payload_specs_consistently():
    assert set(model.EVENT_PAYLOAD_KEYS) == set(NEW_ADMITTED)
    for t, keys in model.EVENT_FREE_TEXT_KEYS.items():
        assert set(keys) <= set(model.EVENT_PAYLOAD_KEYS.get(t, keys))
    assert copy.deepcopy(model.EVENT_DATA_CLASSES)


def test_assistant_turn_author_id_is_agent_at_version():
    row = dict(_first("turns"), author_role="assistant", author_id="recepcion@1.0.0")
    assert _errors("turns", row) == []
    assert _errors("turns", dict(row, author_id="not an id")) != []
