"""PL-L3 first RED: platform_live contract pack (denylist, allow-list, drift, golden, catalog)."""

import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator

import platform_contract as pc
from platform_contract import conformance

ROOT = Path(__file__).resolve().parents[1]
DENIED = ("login_accounts", "mfa_challenges", "staff_sessions")
ALLOWED = (
    "cases",
    "turns",
    "assignments",
    "customer_case_slots",
    "customers",
    "event_log",
    "staff_dimensions",
)


@pytest.mark.parametrize("table", DENIED)
def test_denied_tables_have_no_schema_and_are_refused(table):
    assert not (ROOT / "schemas" / f"{table}.schema.json").exists()
    with pytest.raises(pc.TableRefused) as exc:
        pc.load_schema(table)
    assert exc.value.category == "denied"
    with pytest.raises(pc.TableRefused):
        pc.assert_table_readable(table)


def test_unlisted_table_is_refused_by_default():
    with pytest.raises(pc.TableRefused) as exc:
        pc.assert_table_readable("teams")
    assert exc.value.category == "not_allowlisted"


def test_allowlist_is_exact():
    assert set(pc.ALLOWED_TABLES) == set(ALLOWED)
    for t in ALLOWED:
        pc.assert_table_readable(t)
        Draft202012Validator.check_schema(pc.load_schema(t))


def test_schemas_exclude_credentials_names_and_free_text_derivatives():
    banned = {
        "password_hash", "email", "name", "display_name", "search_text",
        "last_message_preview", "last_turn_preview", "suggestions",
    }
    for t in ALLOWED:
        assert not banned & set(pc.load_schema(t)["properties"]), t


def test_committed_artifacts_match_generated_no_drift():
    for rel, content in pc.generate_artifacts().items():
        assert (ROOT / rel).read_text(encoding="utf-8") == content, f"drift in {rel}"


def test_readme_carries_artifact_version_stamp():
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    assert pc.ARTIFACT_STAMP["commit"] in readme
    assert pc.ARTIFACT_STAMP["artifact_id"] in readme
    assert pc.CONTRACT_VERSION in readme


@pytest.mark.parametrize("table", ALLOWED)
def test_golden_valid_examples_conform(table):
    rows = json.loads((ROOT / "examples" / f"{table}.valid.json").read_text(encoding="utf-8"))
    assert rows
    assert conformance.validate_rows(table, rows) == []


def test_golden_invalid_examples_are_rejected():
    for f in sorted((ROOT / "examples" / "invalid").glob("*.json")):
        doc = json.loads(f.read_text(encoding="utf-8"))
        errs = conformance.validate_rows(doc["table"], [doc["row"]])
        assert errs, f.name


def test_event_catalog_classification():
    assert pc.classify_event_type("case.opened") == "admitted"
    assert pc.classify_event_type("turn.created") == "admitted"
    assert pc.classify_event_type("staff.availability_changed") == "admitted"
    assert pc.classify_event_type("auth.account_locked") == "admitted"
    assert pc.classify_event_type("auth.password_accepted") == "denied"
    assert pc.classify_event_type("staff.team_changed") == "planned"
    assert pc.classify_event_type("brand.new") == "unknown"
    assert set(pc.ADMITTED_EVENT_TYPES) >= {
        "case.opened", "case.queued", "case.assigned", "case.status_changed",
        "case.read", "case.first_responded", "case.closed", "case.viewed",
        "turn.created", "staff.availability_changed",
    }


def test_event_row_with_unknown_type_is_schema_valid_but_flagged():
    ev = json.loads((ROOT / "examples" / "event_log.valid.json").read_text("utf-8"))[0]
    ev = dict(ev, event_type="brand.new")
    assert conformance.validate_rows("event_log", [ev]) == []
    findings = conformance.check_event_stream([ev])
    assert [f["code"] for f in findings] == ["unknown_event_type"]


def test_sequence_gap_and_late_event_findings():
    base = json.loads((ROOT / "examples" / "event_log.valid.json").read_text("utf-8"))
    evs = [dict(e) for e in base]
    evs[-1]["sequence"] += 2
    codes = [f["code"] for f in conformance.check_event_stream(evs)]
    assert "gap_suspected" in codes
    evs2 = [dict(e) for e in base]
    evs2[0]["ingested_at"] = "2026-12-31T00:00:00Z"
    codes2 = [f["code"] for f in conformance.check_event_stream(evs2, late_after_seconds=60)]
    assert "late_event" in codes2


def _case_row():
    return dict(json.loads((ROOT / "examples" / "cases.valid.json").read_text("utf-8"))[0])


@pytest.mark.parametrize("bad", ["not-a-date", "2026-03-02", "2026-03-02T09:00:00", "2026-03-02T09:00:00+02:00"])
def test_datetimes_must_be_rfc3339_utc(bad):
    row = dict(_case_row(), opened_at=bad)
    assert conformance.validate_rows("cases", [row]), bad


def test_violation_messages_never_echo_row_values():
    secret = "TOPSECRETVALUE"
    row = dict(_case_row(), close_note=secret * 100, channel=secret, language=secret)
    msgs = "\n".join(conformance.validate_rows("cases", [row]))
    assert msgs and secret not in msgs


@pytest.mark.parametrize("table,field", [("turns", "author_id"), ("cases", "closed_by_id")])
def test_actor_ids_carry_prefix_patterns(table, field):
    rows = json.loads((ROOT / "examples" / f"{table}.valid.json").read_text("utf-8"))
    row = dict(rows[0], **{field: "nobody"})
    assert conformance.validate_rows(table, [row])
