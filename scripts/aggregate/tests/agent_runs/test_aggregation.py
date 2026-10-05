import hashlib
import copy
import json
import unittest
from datetime import datetime, timedelta, timezone

from scripts.aggregate.agent_runs.aggregation import (
    ExportContractError,
    aggregate_export,
    outcome_group,
    split_for_run,
)


OUTCOMES = [
    "resolved",
    "escalated",
    "transferred",
    "abstained",
    "clarify_exhausted",
    "failed",
    "cancelled",
    "completed",
    "abandoned",
]


def _run_id(number):
    return f"run-{number:05d}"


def _closed_at(number):
    return (datetime(2026, 1, 1, tzinfo=timezone.utc) + timedelta(minutes=number)).isoformat().replace("+00:00", "Z")


def _page_trace(items, cursor):
    return [
        {"requested_after": None, "items": items, "next_after": cursor},
        {"requested_after": cursor, "items": [], "next_after": cursor},
    ]


def _run_items(export):
    return [run for page in export["runs"]["pages"] for run in page["items"]]


def _event_items(export, run_id):
    return [event for page in export["events"][run_id] for event in page["items"]]


def _make_export(outcomes, *, label="SYNTHETIC TEST"):
    runs = []
    events = {}
    for index, item in enumerate(outcomes):
        run_id, outcome = item if isinstance(item, tuple) else (_run_id(index), item)
        closed_at = _closed_at(index)
        runs.append({
            "run_id": run_id,
            "status": "closed",
            "outcome": outcome,
            "closed_at": closed_at,
            "created_at": closed_at,
            "agent": {"id": "agent-secret-test", "version": "1.0.0"},
            "release": "release-secret-test",
            "principal_type": "builder",
            "locale": "es",
        })
        events[run_id] = _page_trace([{
                "type": "run_closed",
                "run_id": run_id,
                "ts": closed_at,
                "payload": {"outcome": outcome},
            }], f"event-cursor-{index}")
    return {
        "_label": label,
        "runs": {"pages": _page_trace(runs, "runs-cursor")},
        "events": events,
        "registry": {"items": [], "next_after": None},
    }


def _balanced_outcomes(per_group=20):
    """Build at least k observations of every metric bucket in each split."""
    # Deterministic search per outcome so both halves have equal support.
    result = []
    n = 0
    for outcome in OUTCOMES:
        counts = {"discovery": 0, "holdout": 0}
        while min(counts.values()) < per_group:
            run_id = _run_id(n)
            half = split_for_run(run_id)
            if counts[half] < per_group:
                result.append((run_id, outcome))
                counts[half] += 1
            n += 1
    return result


class AgentRunAggregationTests(unittest.TestCase):
    def test_aggregate_report_is_byte_deterministic_under_run_input_permutation(self):
        export = _make_export(_balanced_outcomes(per_group=10))
        permuted = copy.deepcopy(export)
        permuted["runs"]["pages"][0]["items"].reverse()
        first = json.dumps(aggregate_export(export), sort_keys=True, separators=(",", ":"))
        second = json.dumps(aggregate_export(permuted), sort_keys=True, separators=(",", ":"))
        self.assertEqual(first, second)

    def test_outcome_mapping_is_closed_and_does_not_equate_completed_with_resolved(self):
        self.assertEqual(outcome_group("resolved"), "resolved")
        self.assertEqual(outcome_group("escalated"), "escalation_or_transfer")
        self.assertEqual(outcome_group("transferred"), "escalation_or_transfer")
        self.assertEqual(outcome_group("abstained"), "abstention_or_clarification_exhausted")
        self.assertEqual(outcome_group("clarify_exhausted"), "abstention_or_clarification_exhausted")
        self.assertEqual(outcome_group("failed"), "failed")
        self.assertEqual(outcome_group("completed"), "other_terminal")
        self.assertIsNone(outcome_group("new_unknown_outcome"))

    def test_complete_two_run_synthetic_fixture_publishes_no_cells_below_k(self):
        report = aggregate_export(_make_export(["completed", "completed"], label="RECORDED synthetic"))
        self.assertEqual(report["evidence_class"], "recorded")
        self.assertEqual(report["cells"], [])
        self.assertEqual(report["availability"], "below_privacy_floor")
        self.assertEqual(report["suppression_status"], "all")
        self.assertNotIn("run-00000", repr(report))
        self.assertNotIn("agent-secret-test", repr(report))

    def test_sufficient_support_emits_only_the_five_frozen_metrics_and_no_ids(self):
        outcomes = _balanced_outcomes(per_group=10)
        report = aggregate_export(_make_export(outcomes))
        expected = {
            "resolved",
            "escalation_or_transfer",
            "abstention_or_clarification_exhausted",
            "failed",
            "other_terminal",
        }
        self.assertEqual({row["metric"] for row in report["cells"]}, expected)
        self.assertEqual(len(report["cells"]), 2 * len(expected))
        self.assertEqual(report["suppression_status"], "none")
        for row in report["cells"]:
            self.assertEqual(set(row), {"metric", "dims", "half", "period", "numerator", "denominator"})
            self.assertEqual(row["dims"], {})
            self.assertGreaterEqual(row["numerator"], 10)
            self.assertGreaterEqual(row["denominator"] - row["numerator"], 10)
            self.assertEqual(row["period"], "2026-01")
        serialized = repr(report)
        # The fixed protocol string contains the words "agent-run"; assert
        # concrete source identifiers are absent rather than banning that
        # harmless protocol name.
        for secret in ("run-00000", "run-00001", "agent-secret", "release-secret", "builder"):
            self.assertNotIn(secret, serialized)

    def test_agent_core_escalated_status_is_a_terminal_population_member(self):
        outcomes = _balanced_outcomes(per_group=10)
        export = _make_export(outcomes)
        for run in _run_items(export):
            if run["outcome"] == "escalated":
                run["status"] = "escalated"
        report = aggregate_export(export)
        self.assertEqual(report["availability"], "published")
        for half in ("discovery", "holdout"):
            expected = sum(1 for run in _run_items(export) if split_for_run(run["run_id"]) == half)
            rows = [row for row in report["cells"] if row["half"] == half]
            self.assertTrue(rows)
            self.assertEqual({row["denominator"] for row in rows}, {expected})

    def test_low_support_outcome_suppresses_entire_partition_vector(self):
        outcomes = _balanced_outcomes(per_group=10)
        # Replace one held-out rare group row with another terminal category.
        rare_outcome = "failed"
        failed_index = next(i for i, value in enumerate(outcomes) if value[1] == rare_outcome and split_for_run(value[0]) == "holdout")
        outcomes[failed_index] = (outcomes[failed_index][0], "completed")
        report = aggregate_export(_make_export(outcomes))
        holdout_rows = [row for row in report["cells"] if row["half"] == "holdout"]
        self.assertEqual(holdout_rows, [])

    def test_duplicate_identical_run_is_counted_once_and_conflict_fails(self):
        one = _make_export(["resolved"])
        one["runs"]["pages"][0]["items"].append(dict(_run_items(one)[0]))
        report = aggregate_export(one)
        self.assertEqual(report["cells"], [])

        conflicting = _make_export(["resolved"])
        conflicting["runs"]["pages"][0]["items"].append({**_run_items(conflicting)[0], "outcome": "failed"})
        with self.assertRaises(ExportContractError):
            aggregate_export(conflicting)

    def test_closed_run_requires_matching_run_closed_event(self):
        export = _make_export(["resolved"])
        _event_items(export, "run-00000")[0]["payload"]["outcome"] = "failed"
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_terminal_event_run_id_must_match_its_event_page(self):
        export = _make_export(["resolved"])
        _event_items(export, "run-00000")[0]["run_id"] = "another-run"
        with self.assertRaisesRegex(ExportContractError, "event run key"):
            aggregate_export(export)

    def test_closed_run_accepts_terminal_event_timestamp_drift_within_month(self):
        export = _make_export(["resolved"])
        _event_items(export, "run-00000")[0]["ts"] = "2026-01-01T00:01:00Z"
        self.assertEqual(aggregate_export(export)["availability"], "below_privacy_floor")

    def test_terminal_event_in_different_month_rejects_export(self):
        export = _make_export(["resolved"])
        _event_items(export, "run-00000")[0]["ts"] = "2026-02-01T00:00:00Z"
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_nonterminal_run_cursor_rejects_incomplete_export(self):
        export = _make_export(["resolved"])
        export["runs"]["pages"].pop()
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_nonterminal_event_cursor_rejects_incomplete_export(self):
        export = _make_export(["resolved"])
        export["events"]["run-00000"].pop()
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_complete_export_accepts_opaque_cursor_when_empty_terminal_page_was_observed(self):
        export = _make_export(["resolved"])
        export["runs"]["pages"][0]["next_after"] = "opaque-run-cursor"
        export["runs"]["pages"][1]["requested_after"] = "opaque-run-cursor"
        export["events"]["run-00000"][0]["next_after"] = "opaque-event-cursor"
        export["events"]["run-00000"][1]["requested_after"] = "opaque-event-cursor"
        report = aggregate_export(export)
        self.assertEqual(report["availability"], "below_privacy_floor")

    def test_run_page_cursor_chain_must_match_the_requested_cursor(self):
        export = _make_export(["resolved"])
        export["runs"]["pages"][1]["requested_after"] = "different-cursor"
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_event_page_cursor_chain_must_match_the_requested_cursor(self):
        export = _make_export(["resolved"])
        export["events"]["run-00000"][1]["requested_after"] = "different-cursor"
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_nonempty_page_cannot_repeat_the_same_cursor(self):
        export = _make_export(["resolved"])
        export["runs"]["pages"][0]["next_after"] = None
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_traversal_cannot_continue_after_empty_terminal_page(self):
        export = _make_export(["resolved"])
        export["runs"]["pages"].append({
            "requested_after": "runs-cursor",
            "items": [],
            "next_after": "runs-cursor",
        })
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_boolean_requested_cursor_cannot_match_integer_cursor(self):
        export = _make_export(["resolved"])
        export["runs"]["pages"][0]["next_after"] = 1
        export["runs"]["pages"][1]["requested_after"] = True
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_terminal_page_cursor_must_be_null_or_valid_opaque_cursor(self):
        for invalid in (True, 1.5, ""):
            export = _make_export(["resolved"])
            export["runs"]["pages"][1]["next_after"] = invalid
            with self.subTest(invalid=invalid), self.assertRaises(ExportContractError):
                aggregate_export(export)

    def test_duplicate_run_with_different_close_instant_is_a_conflict(self):
        export = _make_export(["resolved"])
        changed = dict(_run_items(export)[0])
        changed["closed_at"] = "2026-01-01T00:05:00Z"
        changed["created_at"] = changed["closed_at"]
        export["runs"]["pages"][0]["items"].append(changed)
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_naive_or_non_utc_closed_at_fails_closed(self):
        export = _make_export(["resolved"])
        _run_items(export)[0]["closed_at"] = "2026-01-01T00:00:00"
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_malformed_unhashable_status_fails_with_contract_error(self):
        export = _make_export(["resolved"])
        _run_items(export)[0]["status"] = []
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_privacy_floor_cannot_be_lowered_by_callers(self):
        with self.assertRaises(TypeError):
            aggregate_export(_make_export(["resolved"]), k=2)

    def test_mixed_published_and_suppressed_months_signal_partial_suppression(self):
        published = _make_export(_balanced_outcomes(per_group=10))
        suppressed = _make_export([("run-99999", "completed")])
        _run_items(suppressed)[0]["closed_at"] = "2026-02-01T00:00:00Z"
        _run_items(suppressed)[0]["created_at"] = "2026-02-01T00:00:00Z"
        _event_items(suppressed, "run-99999")[0]["ts"] = "2026-02-01T00:00:00Z"
        published["runs"]["pages"][0]["items"].extend(_run_items(suppressed))
        published["events"].update(suppressed["events"])
        report = aggregate_export(published)
        self.assertEqual(report["availability"], "published")
        self.assertEqual(report["suppression_status"], "partial")
        self.assertNotIn("2026-02", repr(report))

    def test_open_runs_do_not_enter_the_closed_terminal_population(self):
        export = _make_export(["resolved"])
        _run_items(export)[0]["status"] = "open"
        _run_items(export)[0]["outcome"] = None
        export["events"]["run-00000"][0]["items"] = []
        report = aggregate_export(export)
        self.assertEqual(report["cells"], [])

    def test_half_assignment_is_deterministic_and_domain_separated(self):
        self.assertEqual(split_for_run("r-123"), split_for_run("r-123"))
        self.assertIn(split_for_run("r-123"), {"discovery", "holdout"})
        self.assertNotEqual(
            hashlib.sha256(b"pulso-agent-run-split-v1\0r-123").digest(),
            hashlib.sha256(b"other-protocol\0r-123").digest(),
        )


if __name__ == "__main__":
    unittest.main()
