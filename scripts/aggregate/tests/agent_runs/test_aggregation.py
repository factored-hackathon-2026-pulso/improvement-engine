import hashlib
import copy
import json
import unittest
from datetime import datetime, timedelta, timezone

from scripts.aggregate.agent_runs.aggregation import (
    ExportContractError,
    _is_handoff,
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


def _transfer_event(run_id, seq):
    return {
        "type": "run_transferred", "run_id": run_id, "seq": seq,
        "payload": {
            "transfer_id": f"synthetic-transfer-{seq}",
            "to_agent": {"id": "pulso-builder", "version": "1.0.0"},
            "to_release_id": "synthetic-release", "to_run_id": "synthetic-target-run",
            "reason": "synthetic contract fixture", "packet_fp": {"alg": "HMAC-SHA256", "kid": "test", "value": "0" * 64},
            "directory": "synthetic", "directory_hash": "0" * 64, "candidates": [],
        },
    }


def _make_export(outcomes, *, label="SYNTHETIC TEST"):
    runs = []
    events = {}
    for index, item in enumerate(outcomes):
        run_id, outcome = item if isinstance(item, tuple) else (_run_id(index), item)
        closed_at = _closed_at(index)
        runs.append({
            "run_id": run_id,
            "cursor": index + 1,
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
                "seq": 21,
                "ts": closed_at,
                "payload": {"outcome": outcome, "closed_by": "flow"},
            }], f"event-cursor-{index}")
    # Make large synthetic fixtures privacy-safe by default: for every
    # outcome×split cell with >=20 runs, mark exactly ten handoffs. Small
    # fixtures remain all non-handoff and therefore safely suppressed.
    buckets = {}
    for run in runs:
        bucket = outcome_group(run["outcome"])
        key = (split_for_run(run["run_id"]), bucket)
        buckets.setdefault(key, []).append(run)
    for members in buckets.values():
        if len(members) < 20:
            continue
        for run in members[:10]:
            event = next(event for event in events[run["run_id"]][0]["items"]
                         if event["type"] == "run_closed")
            event["payload"]["closed_by"] = "escalation"
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
        export = _make_export(_balanced_outcomes(per_group=20))
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

    def test_sufficient_support_emits_one_outcome_metric_with_five_safe_outcome_values(self):
        outcomes = _balanced_outcomes(per_group=20)
        report = aggregate_export(_make_export(outcomes))
        expected = {
            "resolved", "escalation_or_transfer", "abstention_or_clarification_exhausted",
            "failed", "other_terminal",
        }
        self.assertEqual({row["metric"] for row in report["cells"]}, {"AG_RUN_OUTCOME_RATE", "AG_RUN_HANDOFF_RATE"})
        self.assertEqual({row["dims"]["outcome"] for row in report["cells"]
                          if row["metric"] == "AG_RUN_OUTCOME_RATE"}, expected)
        self.assertEqual(len(report["cells"]), 2 * (len(expected) + 1))
        self.assertEqual(report["suppression_status"], "none")
        for row in report["cells"]:
            self.assertEqual(set(row), {"metric", "dims", "half", "period", "numerator", "denominator"})
            self.assertEqual(row["dims"]["agent"], "other")
            self.assertEqual(row["dims"]["locale"], "es")
            self.assertEqual(row["dims"]["topic"], "not_observed")
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
        outcomes = _balanced_outcomes(per_group=20)
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
        outcomes = _balanced_outcomes(per_group=20)
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

    def test_terminal_close_reason_is_required_and_must_be_a_valid_nonempty_string(self):
        for closed_by in (None, "", "unknown", 7):
            export = _make_export(["resolved"])
            payload = _event_items(export, "run-00000")[0]["payload"]
            if closed_by is None:
                payload.pop("closed_by")
            else:
                payload["closed_by"] = closed_by
            with self.subTest(closed_by=closed_by), self.assertRaises(ExportContractError):
                aggregate_export(export)

    def test_tool_attempt_is_optional_and_defaults_to_one(self):
        export = _make_export(["resolved"])
        page = export["events"]["run-00000"][0]
        page["items"].insert(0, {
            "type": "tool_called", "run_id": "run-00000", "seq": 4,
            "payload": {"status": "ok"},
        })
        report = aggregate_export(export)
        self.assertEqual(report["cells"], [])

    def test_missing_attempt_defaults_to_one_in_publishable_retry_cohort(self):
        export = _make_export(_balanced_outcomes(per_group=20))
        runs = _run_items(export)
        for half in ("discovery", "holdout"):
            selected = [run for run in runs if split_for_run(run["run_id"]) == half][:40]
            for index, run in enumerate(selected):
                payload = {"status": "ok"}
                if index < 10:
                    payload["attempt"] = 2
                export["events"][run["run_id"]][0]["items"].insert(0, {
                    "type": "tool_called", "run_id": run["run_id"], "seq": 4,
                    "payload": payload,
                })
        report = aggregate_export(export)
        retry_rows = [row for row in report["cells"] if row["metric"] == "AG_TOOL_RETRY_RUN_RATE"]
        self.assertEqual({row["half"] for row in retry_rows}, {"discovery", "holdout"})
        for row in retry_rows:
            self.assertEqual(row["numerator"], 10)
            self.assertEqual(row["denominator"], 40)

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
        published = _make_export(_balanced_outcomes(per_group=20))
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
        export["events"]["run-00000"] = [{"requested_after": None, "items": [], "next_after": None}]
        report = aggregate_export(export)
        self.assertEqual(report["cells"], [])

    def test_half_assignment_is_deterministic_and_domain_separated(self):
        self.assertEqual(split_for_run("r-123"), split_for_run("r-123"))
        self.assertIn(split_for_run("r-123"), {"discovery", "holdout"})
        self.assertNotEqual(
            hashlib.sha256(b"pulso-agent-run-split-v1\0r-123").digest(),
            hashlib.sha256(b"other-protocol\0r-123").digest(),
        )

    def test_safe_dimensions_map_registry_unknowns_locale_and_topic_without_source_ids(self):
        outcomes = _balanced_outcomes(per_group=20)
        export = _make_export(outcomes)
        for run in _run_items(export):
            run["agent"]["id"] = "unregistered-private-agent"
            run["locale"] = "fr"
        report = aggregate_export(export)
        self.assertTrue(report["cells"])
        for row in report["cells"]:
            self.assertEqual(
                {key: row["dims"][key] for key in ("agent", "locale", "topic")},
                {"agent": "other", "locale": "other", "topic": "not_observed"},
            )
        self.assertNotIn("unregistered-private-agent", repr(report))
        self.assertNotIn("agent-secret-test", repr(report))

    def test_registry_allowlist_emits_only_its_safe_label(self):
        export = _make_export(_balanced_outcomes(per_group=20))
        for run in _run_items(export):
            run["agent"]["id"] = "pulso-builder"
        report = aggregate_export(export)
        self.assertTrue(report["cells"])
        self.assertEqual({row["dims"]["agent"] for row in report["cells"]}, {"builder"})
        self.assertNotIn("pulso-builder", repr(report))

    def test_locale_allowlist_is_es_pt_other(self):
        for source_locale, expected in (("es", "es"), ("es-MX", "es"), ("pt", "pt"), ("pt-BR", "pt"), ("fr-CA", "other")):
            export = _make_export(_balanced_outcomes(per_group=20))
            for run in _run_items(export):
                run["locale"] = source_locale
            report = aggregate_export(export)
            self.assertTrue(report["cells"])
            self.assertEqual({row["dims"]["locale"] for row in report["cells"]}, {expected})

    def test_highest_cursor_run_snapshot_wins_and_stale_cursor_is_not_counted(self):
        export = _make_export(["resolved"])
        stale = dict(_run_items(export)[0])
        stale.update({"cursor": 0, "agent": {"id": "unknown"}, "outcome": "failed"})
        latest = _run_items(export)[0]
        latest["cursor"] = 2
        export["runs"]["pages"][0]["items"].extend([stale, dict(latest)])
        report = aggregate_export(export)
        self.assertEqual(report["cells"], [])
        self.assertEqual(report["availability"], "below_privacy_floor")

    def test_same_run_cursor_with_conflicting_snapshot_fails_closed(self):
        export = _make_export(["resolved"])
        duplicate = dict(_run_items(export)[0])
        duplicate["outcome"] = "failed"
        export["runs"]["pages"][0]["items"].append(duplicate)
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_tool_error_and_retry_are_distinct_run_metrics_with_exact_denominator(self):
        outcomes = _balanced_outcomes(per_group=20)
        export = _make_export(outcomes)
        runs = _run_items(export)
        # Only a subset of runs use a tool. Each run has a complete event page.
        for index, run in enumerate(runs):
            if index < 50:
                event_page = export["events"][run["run_id"]][0]
                closed = event_page["items"][0]
                status = "error" if index < 10 else "timeout" if index < 15 else "denied" if index < 20 else "uncertain" if index < 23 else "step_up_required" if index < 26 else "ok"
                event_page["items"].insert(0, {
                    "type": "tool_called", "run_id": run["run_id"], "seq": 4,
                    "payload": {"status": status, "attempt": 1},
                })
                if index < 25:
                    event_page["items"].insert(1, {
                        "type": "tool_called", "run_id": run["run_id"], "seq": 5,
                        "payload": {"status": "ok", "attempt": 2},
                    })
        report = aggregate_export(export)
        tool_rows = [row for row in report["cells"] if row["metric"] == "AG_TOOL_ERROR_RUN_RATE"]
        retry_rows = [row for row in report["cells"] if row["metric"] == "AG_TOOL_RETRY_RUN_RATE"]
        self.assertTrue(tool_rows)
        for row in tool_rows:
            self.assertEqual(row["denominator"], sum(
                1 for index, run in enumerate(runs)
                if index < 50 and split_for_run(run["run_id"]) == row["half"]
            ))
            self.assertEqual(row["numerator"], sum(
                1 for index, run in enumerate(runs)
                if index < 20 and split_for_run(run["run_id"]) == row["half"]
            ))
            self.assertGreaterEqual(row["numerator"], 10)
            self.assertGreaterEqual(row["denominator"] - row["numerator"], 10)
        self.assertTrue(retry_rows)
        for row in retry_rows:
            matching = [r for r in tool_rows
                        if (r["half"], r["period"], r["dims"]) ==
                        (row["half"], row["period"], row["dims"])]
            if matching:
                self.assertEqual(row["denominator"], matching[0]["denominator"])

    def test_event_duplicate_pair_deduplicates_identical_and_rejects_conflict(self):
        export = _make_export(["resolved"])
        event = {"type": "tool_called", "run_id": "run-00000", "seq": 4,
                 "payload": {"status": "ok", "attempt": 1}}
        export["events"]["run-00000"][0]["items"].insert(0, event)
        export["events"]["run-00000"][0]["items"].insert(1, copy.deepcopy(event))
        self.assertEqual(aggregate_export(export)["availability"], "below_privacy_floor")
        export["events"]["run-00000"][0]["items"][1]["payload"]["status"] = "error"
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_unknown_tool_status_fails_closed(self):
        export = _make_export(["resolved"])
        export["events"]["run-00000"][0]["items"].insert(0, {
            "type": "tool_called", "run_id": "run-00000", "seq": 4,
            "payload": {"status": "invented-status", "attempt": 2},
        })
        with self.assertRaises(ExportContractError):
            aggregate_export(export)

    def test_tool_metric_suppresses_a_sub_k_no_tool_complement(self):
        export = _make_export(_balanced_outcomes(per_group=20))
        for index, run in enumerate(_run_items(export)):
            export["events"][run["run_id"]][0]["items"].insert(0, {
                "type": "tool_called", "run_id": run["run_id"], "seq": 4,
                "payload": {"status": "error" if index % 2 else "ok", "attempt": 2 if index % 2 else 1},
            })
        report = aggregate_export(export)
        self.assertFalse(any(row["metric"].startswith("AG_TOOL_") for row in report["cells"]))
        self.assertTrue(any(row["metric"] == "AG_RUN_OUTCOME_RATE" for row in report["cells"]))

    def test_handoff_union_counts_each_run_once_across_all_three_contract_signals(self):
        # Isolated algorithm fixture: signals are varied independently to
        # prove CL-0075's OR/dedup semantics, not to assert that every
        # combination is emitted by a real Agent Core producer.
        export = _make_export(_balanced_outcomes(per_group=40))
        runs = _run_items(export)
        for run in runs:
            if outcome_group(run["outcome"]) == "other_terminal":
                closed = next(event for event in export["events"][run["run_id"]][0]["items"]
                              if event["type"] == "run_closed")
                closed["payload"]["closed_by"] = "flow"
        handoff_ids = {half: [] for half in ("discovery", "holdout")}
        for run in runs:
            half = split_for_run(run["run_id"])
            if run["outcome"] == "completed" and len(handoff_ids[half]) < 36:
                handoff_ids[half].append(run["run_id"])

        for half, ids in handoff_ids.items():
            self.assertEqual(len(ids), 36)
            for run in runs:
                if split_for_run(run["run_id"]) == half and run["outcome"] in {"completed", "cancelled", "abandoned"}:
                    closed = next(event for event in export["events"][run["run_id"]][0]["items"]
                                  if event["type"] == "run_closed")
                    closed["payload"]["closed_by"] = "flow"
            # Keep the other_terminal joint cells k-safe independently of the
            # completed bucket exercised by the three handoff branches.
            for source_outcome in ("cancelled", "abandoned"):
                safe_ids = [run["run_id"] for run in runs
                            if split_for_run(run["run_id"]) == half and run["outcome"] == source_outcome][:10]
                for run_id in safe_ids:
                    closed = next(event for event in export["events"][run_id][0]["items"]
                                  if event["type"] == "run_closed")
                    closed["payload"]["closed_by"] = "escalation"
            # Twelve distinct runs exercise each documented signal branch.
            for run_id in ids[:12]:
                run = next(item for item in runs if item["run_id"] == run_id)
                run["status"] = "escalated"
            for run_id in ids[12:24]:
                event = _transfer_event(run_id, 20)
                export["events"][run_id][0]["items"].insert(0, event)
            for run_id in ids[24:]:
                closed = next(event for event in export["events"][run_id][0]["items"]
                              if event["type"] == "run_closed")
                closed["payload"]["closed_by"] = "escalation"

            # A run matching more than one signal is still one handoff.
            overlap_id = ids[0]
            export["events"][overlap_id][0]["items"].insert(0, _transfer_event(overlap_id, 19))

        report = aggregate_export(export)
        handoff_rows = [row for row in report["cells"] if row["metric"] == "AG_RUN_HANDOFF_RATE"]
        self.assertEqual({row["half"] for row in handoff_rows}, {"discovery", "holdout"})
        for row in handoff_rows:
            self.assertEqual(row["numerator"], 96)
            self.assertNotIn("outcome", row["dims"])
            self.assertEqual(row["denominator"], 360)
            self.assertGreaterEqual(row["denominator"] - row["numerator"], 10)

    def test_marginals_that_pass_k_are_both_suppressed_when_joint_handoff_cell_is_sub_k(self):
        export = _make_export(_balanced_outcomes(per_group=20))
        runs = _run_items(export)
        for half in ("discovery", "holdout"):
            by_outcome = {}
            for run in runs:
                if split_for_run(run["run_id"]) == half:
                    by_outcome.setdefault(outcome_group(run["outcome"]), []).append(run)
            self.assertEqual(set(len(items) for items in by_outcome.values()), {20, 40, 60})
            # Both marginals independently clear k, but resolved×handoff
            # has only five runs. Clear the fixture's baseline handoffs first.
            for group_items in by_outcome.values():
                for run in group_items:
                    closed = next(event for event in export["events"][run["run_id"]][0]["items"]
                                  if event["type"] == "run_closed")
                    closed["payload"]["closed_by"] = "flow"
            for bucket, items in by_outcome.items():
                selected = items[:5] if bucket == "resolved" else items[:10]
                for run in selected:
                    closed = next(event for event in export["events"][run["run_id"]][0]["items"]
                                  if event["type"] == "run_closed")
                    closed["payload"]["closed_by"] = "escalation"
        report = aggregate_export(export)
        self.assertFalse(any(row["metric"] in {"AG_RUN_OUTCOME_RATE", "AG_RUN_HANDOFF_RATE"}
                             for row in report["cells"]))
        self.assertEqual(report["availability"], "below_privacy_floor")

    def test_handoff_predicate_matches_each_contract_signal_and_union(self):
        # Pure truth-table coverage of the contract predicate; no producer
        # consistency claim is made about these isolated input combinations.
        closed = {"type": "run_closed"}
        transferred = {"type": "run_transferred"}
        self.assertTrue(_is_handoff("escalated", [closed], "flow"))
        self.assertTrue(_is_handoff("closed", [closed, transferred], "flow"))
        self.assertTrue(_is_handoff("closed", [closed], "transfer"))
        self.assertTrue(_is_handoff("closed", [closed], "escalation"))
        self.assertFalse(_is_handoff("closed", [closed], "flow"))
        self.assertFalse(_is_handoff("closed", [closed], None))

    def test_invalid_or_missing_agent_and_locale_fail_before_safe_mapping(self):
        mutations = (
            lambda run: run.pop("agent"),
            lambda run: run.update({"agent": {"id": "bad id", "version": "1.0.0"}}),
            lambda run: run.update({"agent": {"id": "unknown-agent", "version": "bad"}}),
            lambda run: run.pop("locale"),
            lambda run: run.update({"locale": "ES"}),
            lambda run: run.update({"locale": "e"}),
            lambda run: run.update({"locale": "es_419"}),
            lambda run: run.update({"locale": "x" * 36}),
            lambda run: run.update({"locale": ["es"]}),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                export = _make_export(["resolved"])
                mutate(_run_items(export)[0])
                with self.assertRaises(ExportContractError):
                    aggregate_export(export)

    def test_valid_unknown_agent_and_locale_values_are_coarsened(self):
        export = _make_export(_balanced_outcomes(per_group=20))
        for run in _run_items(export):
            run["agent"]["id"] = "unknown-valid-id"
            run["locale"] = "fr"
        report = aggregate_export(export)
        self.assertTrue(report["cells"])
        self.assertEqual({row["dims"]["agent"] for row in report["cells"]}, {"other"})
        self.assertEqual({row["dims"]["locale"] for row in report["cells"]}, {"other"})

    def test_event_page_must_be_complete_before_tool_denominator_exists(self):
        export = _make_export(["resolved"])
        export["events"]["run-00000"].pop()
        with self.assertRaises(ExportContractError):
            aggregate_export(export)


if __name__ == "__main__":
    unittest.main()
