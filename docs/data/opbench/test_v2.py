"""Synthetic regressions for the preregistered OPBENCH-lite v2 contract."""

import json
import csv
import contextlib
import io
import tempfile
import unittest
import sys
from types import SimpleNamespace
from unittest.mock import patch
from pathlib import Path

from opbench_v2 import (
    CHANNELS_V2,
    REASONS_V2,
    aggregate_bank_v2_rows,
    generate_v2_from_rows,
    aggregate_digital_events,
    aggregate_marketing_consent,
    normalize_channel_v2,
    normalize_pqr_category_v2,
    normalize_reason_v2,
    planned_cells_v2,
    summarize_e0_complaint_linkage,
    build_v2_payload,
    m1_discovery_qualified,
    m1_replication_qualified,
    m1_monthly_persistent,
    AGENT_OUTLIER_CONTROL,
)
from generate_v2 import _run_e0_v2, generate, iter_unique_source_rows, main, validate_v2_artifacts, write_v2_outputs


def valid_coverage():
    return {
        "bank": {
            name: {"value": 10, "suppressed": False, "suppression_reason": None}
            for name in ("contact_rows", "contacts_with_reason", "contacts_with_channel", "pqr_valid_status", "pqr_valid_sla_flag", "eligible_csat", "linked_eligible_csat")
        },
        "digital_actions": {
            name: {"numerator": 10, "denominator": 20, "rate": 0.5, "suppressed": False, "suppression_reason": None}
            for name in ("transactional_actions", "other_views")
        },
        "marketing_consent": {"total_valid": {"value": 10, "suppressed": False, "suppression_reason": None}},
        "e0_linkage": {"eligible_cases": 20, "matched_cases": 20},
    }


class V2VocabularyAndPlanTests(unittest.TestCase):
    def test_strict_v2_schema_freezes_family_and_audit_only_control(self):
        schema_path = Path(__file__).with_name("opbench-lite-v2.schema.json")
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        self.assertEqual(schema["properties"]["version"]["const"], "2")
        self.assertEqual(schema["properties"]["entries"]["minItems"], 98)
        self.assertEqual(schema["properties"]["entries"]["maxItems"], 98)
        negative = schema["properties"]["negative_controls"]["properties"]
        self.assertEqual(negative["agent_outliers"]["properties"]["origin"]["const"], "audited_source")
        self.assertEqual(negative["agent_outliers"]["properties"]["individuals_emitted"]["const"], False)
        self.assertIn("sourceCoverage", schema["$defs"])
        conditions = schema["$defs"]["entry"]["allOf"]
        self.assertGreaterEqual(len(conditions), 5)
        self.assertEqual(schema["properties"]["source_coverage"]["$ref"], "#/$defs/sourceCoverage")

    def test_contact_domains_include_audited_reasons_and_distinct_channels(self):
        self.assertEqual(
            REASONS_V2,
            ("complaint", "transactional", "technical", "commercial", "retention", "product"),
        )
        self.assertEqual(
            CHANNELS_V2,
            ("phone", "email", "mobile_app", "whatsapp", "web_chat", "web"),
        )
        self.assertEqual(normalize_reason_v2("Comercial"), "commercial")
        self.assertEqual(normalize_reason_v2("RETENCIÓN"), "retention")
        self.assertEqual(normalize_channel_v2(" PHONE "), "phone")
        self.assertEqual(normalize_channel_v2("WhatsApp"), "whatsapp")
        self.assertEqual(normalize_channel_v2("Web_Chat"), "web_chat")
        self.assertEqual(normalize_channel_v2("Web Chat"), "web_chat")
        with self.assertRaises(ValueError):
            normalize_channel_v2("unknown private channel")
        with self.assertRaises(ValueError):
            normalize_channel_v2("Inbound Call")
        with self.assertRaises(ValueError):
            normalize_channel_v2("cáll")

    def test_pqr_vocabulary_is_independent_of_contact_reasons(self):
        self.assertEqual(
            [normalize_pqr_category_v2(value) for value in
             ("Transactions", "Fees", "Technical", "Branch", "Service")],
            ["transactions", "fees", "technical", "branch", "service"],
        )
        with self.assertRaises(ValueError):
            normalize_pqr_category_v2("customer-provided free text")

    def test_full_preregistered_inferential_family_is_95_cells(self):
        expected = {"M1": 37, "M2": 7, "M3": 7, "M4": 6, "M5": 6, "M6": 31, "E1": 1}
        cells = {metric: planned_cells_v2(metric) for metric in expected}
        self.assertEqual({metric: len(value) for metric, value in cells.items()}, expected)
        self.assertEqual(sum(map(len, cells.values())), 95)
        self.assertEqual(
            cells["M1"][0],
            {"reason_category": "complaint", "channel": "phone"},
        )
        self.assertIn({"reason_category": "retention", "channel": "phone"}, cells["M1"])
        self.assertIn({"pqr_category": "fees"}, cells["M4"])
        self.assertIn({"reason_category": "complaint", "channel": "other"}, cells["M6"])

    def test_m1_discovery_and_holdout_require_preregistered_risk_thresholds(self):
        self.assertTrue(m1_discovery_qualified(600, 1000, 400, 1000, 0.01, 1200, 800))
        self.assertFalse(m1_discovery_qualified(600, 1000, 400, 1000, 0.01, 499, 1000))
        self.assertFalse(m1_discovery_qualified(450, 1000, 400, 1000, 0.01, 1000, 1000))
        self.assertFalse(m1_discovery_qualified(600, 1000, 400, 1000, 0.051, 1200, 800))
        self.assertTrue(m1_replication_qualified(600, 1000, 400, 1000, 0.04))
        self.assertFalse(m1_replication_qualified(490, 1000, 400, 1000, 0.01))

    def test_m1_monthly_robustness_requires_audited_same_direction_in_80_percent(self):
        stable = [(200, 120, 200, 40)] * 35
        self.assertTrue(m1_monthly_persistent(stable))
        unstable = [(200, 120, 200, 40)] * 27 + [(200, 30, 200, 40)] * 8
        self.assertFalse(m1_monthly_persistent(unstable))
        self.assertFalse(m1_monthly_persistent(stable[:34]))

    def test_catalog_builder_requires_every_cell_and_exact_three_contexts(self):
        inferential = []
        for metric in ("M1", "M2", "M3", "M4", "M5", "M6", "E1"):
            for index, cell in enumerate(planned_cells_v2(metric)):
                entry = {
                    "id": f"{metric}-{index:02d}", "family": "synthetic", "title": "Synthetic test entry",
                    "metric_id": metric, "cell": cell, "definition": "synthetic test definition",
                    "numerator": None, "denominator": None,
                    "snapshot": {"numerator": None, "denominator": None, "suppressed": True},
                    "cells_explored": {"family_size": 95},
                    "multiple_testing": {
                        "method": "two_proportion_z_benjamini_hochberg_fdr",
                        "family_size": 95, "adjusted_q": None,
                    },
                    "status": "uncertain", "type": "descriptive_only",
                    "actionability": "covered_existing_capability" if metric == "E1" else "context_only",
                    "caveats": ["synthetic"],
                }
                if (metric, tuple(sorted(cell.items()))) in {
                    ("M2", (("channel", "phone"),)), ("M3", (("channel", "phone"),)),
                    ("M4", (("pqr_category", "technical"),)), ("M5", (("pqr_category", "technical"),)),
                }:
                    entry["negative_control_status"] = "audit_consistent"
                inferential.append(entry)
        descriptive = [
            {
                "id": metric, "family": "context", "title": "Synthetic context",
                "metric_id": metric, "cell": {"scope": "overall"},
                "definition": "synthetic context definition", "numerator": None,
                "denominator": None, "snapshot": {"suppressed": True},
                "cells_explored": {"family_size": 0},
                "multiple_testing": {
                    "method": "not_applicable_preregistered_descriptive", "adjusted_q": None,
                },
                "status": "context_only", "type": "risk" if metric == "R1" else "descriptive_only",
                "actionability": "context_only", "caveats": ["synthetic"],
            }
            for metric in ("D1", "R1", "L1")
        ]
        coverage = valid_coverage()
        result = build_v2_payload(inferential, descriptive, coverage)
        self.assertEqual(len(result["entries"]), 98)
        self.assertEqual(result["negative_controls"]["agent_outliers"]["origin"], "audited_source")
        self.assertEqual(result["negative_controls"]["agent_outliers"]["aggregate_statement"], AGENT_OUTLIER_CONTROL)
        bad_context = [dict(entry) for entry in descriptive]
        bad_context[0]["type"] = "problem"
        with self.assertRaisesRegex(ValueError, "registered semantics"):
            build_v2_payload(inferential, bad_context, {})
        bad_m6 = [dict(entry) for entry in inferential]
        m6_entry = next(entry for entry in bad_m6 if entry["metric_id"] == "M6")
        m6_entry["type"] = "problem"
        m6_entry["actionability"] = "candidate"
        with self.assertRaisesRegex(ValueError, "M6.*cannot be actionable"):
            build_v2_payload(bad_m6, descriptive, {})
        bad_control = [dict(entry) for entry in inferential]
        phone_control = next(entry for entry in bad_control if entry["metric_id"] == "M2" and entry["cell"] == {"channel": "phone"})
        phone_control["negative_control_status"] = "audit_consistent"
        phone_control["status"] = "corroborated"
        phone_control["actionability"] = "candidate"
        with self.assertRaisesRegex(ValueError, "audit-consistent disposition"):
            build_v2_payload(bad_control, descriptive, {})
        phone_control["negative_control_status"] = "audit_discrepancy"
        phone_control["caveats"] = ["This is a discrepancy with the audit."]
        phone_control["cell"] = {"channel": "web"}
        with self.assertRaisesRegex(ValueError, "invalid or duplicate cell"):
            build_v2_payload(bad_control, descriptive, {})
        phone_control["cell"] = {"channel": "phone"}
        phone_control["negative_control_status"] = "audit_discrepancy"
        phone_control["caveats"] = ["no caveat"]
        with self.assertRaisesRegex(ValueError, "discrepancy must be disclosed"):
            build_v2_payload(bad_control, descriptive, {})
        with self.assertRaisesRegex(ValueError, "all 95 cells"):
            build_v2_payload(inferential[:-1], descriptive, {})
        with self.assertRaisesRegex(ValueError, "exactly the three"):
            build_v2_payload(inferential, descriptive[:-1], {})
        with self.assertRaisesRegex(ValueError, "coverage.*aggregate disclosure contract"):
            build_v2_payload(
                inferential, descriptive, {
                    **coverage,
                    "bank": {**coverage["bank"], "contact_rows": {"value": 1, "suppressed": False, "suppression_reason": None}},
                },
            )
        with self.assertRaisesRegex(ValueError, "registered aggregate contract"):
            build_v2_payload(inferential, descriptive, {**coverage, "untrusted_value": "private-customer-name"})
        partial_linkage = json.loads(json.dumps(coverage))
        partial_linkage["e0_linkage"]["matched_cases"] = None
        with self.assertRaisesRegex(ValueError, "both be suppressed or disclosed"):
            build_v2_payload(inferential, descriptive, partial_linkage)
        self.assertEqual(build_v2_payload(inferential, descriptive, coverage)["source_coverage"], coverage)


class V2DescriptiveAggregateTests(unittest.TestCase):
    def test_python_to_e0_cli_bridge_passes_only_hashed_complaint_ids(self):
        expected = {
            "discovery": {"selected": 154, "total": 200},
            "replication": {"selected": 1433, "total": 1800},
            "complaint_ids_matched": 2000,
            "eligible_cases": 2000,
        }
        raw_identifier = "synthetic-complaint-id-never-written"
        digest = __import__("hashlib").sha256(raw_identifier.encode()).digest()
        seen = {}

        def fake_run(command, **kwargs):
            self.assertEqual(command[-2], "--v2")
            self.assertEqual(command[-4], "--")
            digest_path = Path(command[-1])
            digest_text = digest_path.read_text(encoding="ascii")
            seen["text"] = digest_text
            self.assertEqual(digest_text, digest.hex() + "\n")
            self.assertNotIn(raw_identifier, digest_text)
            return SimpleNamespace(stdout=json.dumps(expected))

        with tempfile.TemporaryDirectory() as temporary, patch("generate_v2.subprocess.run", side_effect=fake_run):
            result = _run_e0_v2(Path(temporary) / "datos", {digest})
        self.assertEqual(result, expected)
        self.assertNotIn(raw_identifier, seen["text"])
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(ValueError, "digest source is empty"):
                _run_e0_v2(Path(temporary), set())

    def test_cli_emits_only_relative_artifact_names_and_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            payload = Path(temporary) / "opbench-lite.json"
            audit = Path(temporary) / "cell_audit.json"
            stdout = io.StringIO()
            args = ["generate_v2", "--data-root", temporary, "--e0-data", temporary, "--output-dir", temporary]
            with patch.object(sys, "argv", args), patch("generate_v2.generate", return_value=(payload, audit)):
                with contextlib.redirect_stdout(stdout):
                    self.assertEqual(main(), 0)
            self.assertEqual(json.loads(stdout.getvalue()), {"payload": "opbench-lite.json", "audit": "cell_audit.json", "version": "2"})
            self.assertNotIn(temporary, stdout.getvalue())

    def test_local_file_reader_to_immutable_output_is_synthetic_e2e(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "sources"
            output = Path(temporary) / "results"
            e0 = Path(temporary) / "e0"
            e0.mkdir()
            table_rows = {
                "call_center_interactions": (
                    ["interaction_id", "customer_id", "reason_category", "channel", "was_resolved", "interaction_date"],
                    [{"interaction_id": f"i-{i}", "customer_id": f"c-{i}", "reason_category": "Queja", "channel": "phone", "was_resolved": "false", "interaction_date": "2025-01-01"} for i in range(10)],
                ),
                "complaints": (
                    ["complaint_id", "customer_id", "category", "status", "sla_breached"],
                    [{"complaint_id": f"p-{i}", "customer_id": f"c-{i}", "category": "Technical", "status": "Open", "sla_breached": "true"} for i in range(10)],
                ),
                "satisfaction_surveys": (
                    ["survey_id", "interaction_id", "customer_id", "survey_type", "send_channel", "main_score"],
                    [{"survey_id": f"s-{i}", "interaction_id": f"i-{i}", "customer_id": f"c-{i}", "survey_type": "CSAT", "send_channel": "IVR", "main_score": "1"} for i in range(10)],
                ),
                "digital_events": (
                    ["event_id", "event_type", "action"],
                    [{"event_id": f"d-{i}", "event_type": "Error", "action": "view_transactions"} for i in range(10)]
                    + [{"event_id": f"d2-{i}", "event_type": "Success", "action": "view_help"} for i in range(10)],
                ),
                "campaign_sends": (
                    ["send_id", "customer_id", "send_channel"],
                    [{"send_id": f"m-{i}", "customer_id": f"c-{i}", "send_channel": "Email"} for i in range(10)],
                ),
                "customers": (
                    ["customer_id", "accepts_marketing"],
                    [{"customer_id": f"c-{i}", "accepts_marketing": "false"} for i in range(10)],
                ),
            }
            for table, (fields, rows) in table_rows.items():
                directory = root / table
                directory.mkdir(parents=True)
                with (directory / "part.csv").open("w", encoding="utf-8", newline="") as stream:
                    writer = csv.DictWriter(stream, fieldnames=fields)
                    writer.writeheader()
                    writer.writerows(rows)
            e0_result = {
                "discovery": {"selected": 154, "total": 200},
                "replication": {"selected": 1433, "total": 1800},
                "complaint_ids_matched": 2000, "eligible_cases": 2000,
            }
            with patch("generate_v2._run_e0_v2", return_value=e0_result) as e0_runner:
                payload_path, audit_path = generate(root, e0, output)
            e0_runner.assert_called_once()
            self.assertEqual(e0_runner.call_args.args[0], e0)
            complaint_hashes = e0_runner.call_args.args[1]
            self.assertEqual(len(complaint_hashes), 10)
            self.assertTrue(all(isinstance(value, bytes) and len(value) == 32 for value in complaint_hashes))
            payload = json.loads(payload_path.read_text(encoding="utf-8"))
            audit = json.loads(audit_path.read_text(encoding="utf-8"))
            validate_v2_artifacts(payload, audit)
            emitted = payload_path.read_text(encoding="utf-8") + audit_path.read_text(encoding="utf-8")
            for private in ("c-0", "i-0", "p-0", "s-0", "d-0", "m-0"):
                self.assertNotIn(private, emitted)

    def test_bank_aggregates_cover_v2_reasons_pqr_and_linked_csat_without_ids(self):
        contacts = [
            {"interaction_id": "syn-i1", "customer_id": "syn-c1", "reason_category": "Queja", "channel": "phone", "was_resolved": "false"},
            {"interaction_id": "syn-i2", "customer_id": "syn-c2", "reason_category": "Comercial", "channel": "phone", "was_resolved": "false"},
            {"interaction_id": "syn-i3", "customer_id": "syn-c3", "reason_category": "Retención", "channel": "email", "was_resolved": "true"},
            {"interaction_id": "syn-i4", "customer_id": "syn-c4", "reason_category": "Técnico", "channel": "phone", "was_resolved": "false"},
            {"interaction_id": "syn-i5", "customer_id": "syn-c5", "reason_category": "Transaccional", "channel": "web", "was_resolved": "true"},
            {"interaction_id": "syn-i6", "customer_id": "syn-c6", "reason_category": "Queja", "channel": "whatsapp", "was_resolved": "true"},
            {"interaction_id": "syn-i7", "customer_id": "syn-c7", "reason_category": "Queja", "channel": "web_chat", "was_resolved": "false"},
            {"interaction_id": "syn-i8", "customer_id": "syn-c8", "reason_category": "Queja", "channel": "app", "was_resolved": "false"},
        ]
        pqrs = [
            {"complaint_id": "syn-p1", "customer_id": "syn-c1", "category": "Fees", "status": "Open", "sla_breached": "true"},
            {"complaint_id": "syn-p2", "customer_id": "syn-c2", "category": "Service", "status": "Closed", "sla_breached": "false"},
        ]
        surveys = [
            {"survey_id": "syn-s1", "interaction_id": "syn-i1", "customer_id": "syn-c1", "survey_type": "CSAT", "send_channel": "IVR", "main_score": "1"},
        ]
        metrics, coverage, _ = aggregate_bank_v2_rows(contacts, pqrs, surveys)
        phone_complaints = metrics["M2"].grouped[(('channel', 'phone'),)]["overall"]
        self.assertEqual(phone_complaints, [1, 3])
        phone_unresolved_complaints = metrics["M3"].grouped[(('channel', 'phone'),)]["overall"]
        self.assertEqual(phone_unresolved_complaints, [1, 3])
        fees_open = metrics["M4"].grouped[(('pqr_category', 'fees'),)]["overall"]
        self.assertEqual(fees_open, [1, 1])
        linked_csat = metrics["M6"].grouped[
            tuple(sorted({"reason_category": "complaint", "channel": "phone"}.items()))
        ]["overall"]
        self.assertEqual(linked_csat, [1, 1])
        self.assertEqual(coverage["contact_rows"], 8)
        serialized = repr((metrics, coverage))
        self.assertNotIn("syn-c1", serialized)
        self.assertNotIn("syn-i1", serialized)
        self.assertNotIn("syn-p1", serialized)

    def test_bank_aggregation_fails_closed_on_unrecognized_nonempty_channel(self):
        contact = {
            "interaction_id": "synthetic", "customer_id": "synthetic",
            "reason_category": "Queja", "channel": "unknown channel", "was_resolved": "false",
        }
        with self.assertRaisesRegex(ValueError, "unrecognized contact channel"):
            aggregate_bank_v2_rows([contact], [], [])

    def test_synthetic_v2_end_to_end_is_deterministic_complete_and_private(self):
        contacts = [
            {"interaction_id": f"syn-i-{index}", "customer_id": f"syn-c-{index}",
             "reason_category": reason, "channel": channel, "was_resolved": resolved}
            for index, (reason, channel, resolved) in enumerate([
                ("Queja", "phone", "false"), ("Comercial", "phone", "false"),
                ("Retención", "email", "true"), ("Técnico", "phone", "false"),
                ("Transaccional", "web", "true"), ("Queja", "whatsapp", "true"),
                ("Queja", "web_chat", "false"), ("Queja", "app", "false"),
            ])
        ]
        pqrs = [
            {"complaint_id": "syn-p1", "customer_id": "syn-c-0", "category": "Fees", "status": "Open", "sla_breached": "true"},
            {"complaint_id": "syn-p2", "customer_id": "syn-c-1", "category": "Service", "status": "Closed", "sla_breached": "false"},
        ]
        surveys = [
            {"survey_id": "syn-s1", "interaction_id": "syn-i-0", "customer_id": "syn-c-0", "survey_type": "CSAT", "send_channel": "IVR", "main_score": "1"},
        ]
        digital = (
            [{"event_type": "Error", "action": "view_transactions", "customer_id": "secret-d"}] * 10
            + [{"event_type": "Success", "action": "view_transactions", "customer_id": "secret-d"}] * 10
            + [{"event_type": "Error", "action": "view_help", "customer_id": "secret-d"}] * 10
            + [{"event_type": "Success", "action": "view_help", "customer_id": "secret-d"}] * 10
        )
        sends = (
            [{"customer_id": "syn-marketing-no", "send_channel": "Email"}] * 10
            + [{"customer_id": "syn-marketing-yes", "send_channel": "Email"}] * 10
        )
        consent = {"syn-marketing-no": False, "syn-marketing-yes": True}
        e0 = {
            "discovery": {"selected": 154, "total": 200},
            "replication": {"selected": 1433, "total": 1800},
            "complaint_ids_matched": 2000,
            "eligible_cases": 2000,
        }
        args = (contacts, pqrs, surveys, digital, sends, consent, e0)
        payload, audit = generate_v2_from_rows(*args)
        payload_again, audit_again = generate_v2_from_rows(*args)
        self.assertEqual(json.dumps(payload, sort_keys=True), json.dumps(payload_again, sort_keys=True))
        self.assertEqual(json.dumps(audit, sort_keys=True), json.dumps(audit_again, sort_keys=True))
        self.assertEqual(len(payload["entries"]), 98)
        self.assertEqual(len(audit["cells"]), 95)
        self.assertEqual(audit["planned_cell_count"], 95)

        # Exercise the actual v2 generator output through the independent
        # sensor-scoring contract; this is interface validation, not a real
        # engine score or evidence of benchmark accuracy.
        from scripts.scoring.score_findings import score

        source_entry = next(entry for entry in payload["entries"] if entry["metric_id"] == "M1")
        difference = (source_entry.get("effect") or {}).get("difference")
        direction = "up" if difference is not None and difference > 0.005 else (
            "down" if difference is not None and difference < -0.005 else "none"
        )
        sensor_signal = {
            "metric": source_entry["metric_id"],
            "dims": source_entry["cell"],
            "status": source_entry["status"],
            "direction": direction,
            "discovery": {"diff": difference},
        }
        scored = score(payload, {"cells_explored": 1, "signals": [sensor_signal]})
        self.assertEqual(set(scored["scores"]), {"recall", "precision", "ranking_agreement_spearman"})
        self.assertEqual(scored["benchmark"], "OPBENCH-lite")
        self.assertEqual(payload["negative_controls"]["agent_outliers"]["individuals_emitted"], False)
        tampered_payload = json.loads(json.dumps(payload))
        tampered_payload["source_coverage"]["bank"]["contact_rows"] = {
            "value": 1, "suppressed": False, "suppression_reason": None,
        }
        with self.assertRaisesRegex(ValueError, "bank counts violate"):
            validate_v2_artifacts(tampered_payload, audit)

    def test_v2_audit_rejects_non_finite_numeric_values(self):
        payload, audit = generate_v2_from_rows(
            [], [], [], [], [], {},
            {
                "discovery": {"selected": 154, "total": 200},
                "replication": {"selected": 1433, "total": 1800},
                "complaint_ids_matched": 2000,
                "eligible_cases": 2000,
            },
        )
        audit["cells"][0]["discovery_p_value"] = float("nan")
        with self.assertRaisesRegex(ValueError, "finite JSON number"):
            validate_v2_artifacts(payload, audit)

    def test_v2_audit_rejects_unknown_keys_at_every_contract_level(self):
        payload, audit = generate_v2_from_rows(
            [], [], [], [], [], {},
            {
                "discovery": {"selected": 154, "total": 200},
                "replication": {"selected": 1433, "total": 1800},
                "complaint_ids_matched": 2000,
                "eligible_cases": 2000,
            },
        )
        for mutate in (
            lambda candidate: candidate.update(unregistered="extra"),
            lambda candidate: candidate["cells"][0].update(unregistered="extra"),
            lambda candidate: candidate["cells"][0]["cell"].update(unregistered="extra"),
            lambda candidate: candidate["cells"][0]["snapshot"].update(unregistered="extra"),
        ):
            candidate = json.loads(json.dumps(audit))
            mutate(candidate)
            with self.assertRaisesRegex(ValueError, "audit.*unexpected fields"):
                validate_v2_artifacts(payload, candidate)

    def test_v2_audit_rejects_missing_required_row_and_nested_fields(self):
        payload, audit = generate_v2_from_rows(
            [], [], [], [], [], {},
            {
                "discovery": {"selected": 154, "total": 200},
                "replication": {"selected": 1433, "total": 1800},
                "complaint_ids_matched": 2000,
                "eligible_cases": 2000,
            },
        )
        for mutate in (
            lambda candidate: candidate["cells"].pop(),
            lambda candidate: candidate["cells"][0].pop("metric_id"),
            lambda candidate: candidate["cells"][0]["snapshot"].pop("suppressed"),
            lambda candidate: candidate["cells"][0]["multiple_testing"].pop("method"),
            lambda candidate: candidate["cells"][0]["cells_explored"].pop("family_size"),
        ):
            candidate = json.loads(json.dumps(audit))
            mutate(candidate)
            with self.assertRaisesRegex(ValueError, "audit.*(required|complete|fields|registered 95-cell)"):
                validate_v2_artifacts(payload, candidate)
        combined = json.dumps({"payload": payload, "audit": audit}, sort_keys=True)
        for private in ("syn-c-0", "syn-i-0", "syn-p1", "syn-s1", "syn-marketing-no", "secret-d"):
            self.assertNotIn(private, combined)
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            first_payload, first_audit = write_v2_outputs(Path(first), payload, audit)
            second_payload, second_audit = write_v2_outputs(Path(second), payload, audit)
            self.assertEqual(first_payload.read_bytes(), second_payload.read_bytes())
            self.assertEqual(first_audit.read_bytes(), second_audit.read_bytes())
            self.assertEqual(first_payload.relative_to(first).as_posix(), "v2/opbench-lite.json")
            self.assertFalse((Path(first) / "opbench-lite.json").exists())
            with self.assertRaises(FileExistsError):
                write_v2_outputs(Path(first), payload, audit)

    def test_source_reader_collapses_exact_duplicate_and_rejects_conflicts(self):
        columns = ["event_id", "event_type", "action"]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "digital_events" / "year=2026"
            directory.mkdir(parents=True)
            for name, event_type in (("a.csv", "Error"), ("b.csv", "Error")):
                with (directory / name).open("w", newline="", encoding="utf-8") as stream:
                    writer = csv.DictWriter(stream, fieldnames=columns)
                    writer.writeheader()
                    writer.writerow({"event_id": "synthetic-event", "event_type": event_type, "action": "view_transactions"})
            self.assertEqual(len(list(iter_unique_source_rows(root, "digital_events"))), 1)
            with (directory / "b.csv").open("w", newline="", encoding="utf-8") as stream:
                writer = csv.DictWriter(stream, fieldnames=columns)
                writer.writeheader()
                writer.writerow({"event_id": "synthetic-event", "event_type": "Success", "action": "view_transactions"})
            with self.assertRaisesRegex(ValueError, "conflicting duplicate key"):
                list(iter_unique_source_rows(root, "digital_events"))

    def test_digital_error_groups_are_separate_aggregate_only_and_noncausal(self):
        rows = (
            [{"event_type": "Error", "action": "view_transactions", "customer_id": "secret-c1"}] * 10
            + [{"event_type": "Success", "action": "initiate_transfer", "customer_id": "secret-c2"}] * 10
            + [{"event_type": "Error", "action": "view_help", "customer_id": "secret-c3"}] * 10
            + [{"event_type": "Success", "action": "view_home", "customer_id": "secret-c4"}] * 10
            + [{"event_type": "Error", "action": "login", "customer_id": "secret-c5"}]
        )
        result = aggregate_digital_events(rows)
        self.assertEqual(result["actionability"], "context_only")
        self.assertEqual(result["multiple_testing"]["method"], "not_applicable_preregistered_descriptive")
        self.assertEqual(result["groups"]["transactional_actions"]["numerator"], 10)
        self.assertEqual(result["groups"]["transactional_actions"]["denominator"], 20)
        self.assertEqual(result["groups"]["other_views"]["numerator"], 10)
        self.assertEqual(result["groups"]["other_views"]["denominator"], 20)
        self.assertNotIn("secret-c1", repr(result))
        self.assertNotIn("customer_id", repr(result))

    def test_digital_groups_suppress_small_counts(self):
        result = aggregate_digital_events([
            {"event_type": "Error", "action": "view_transactions"},
            {"event_type": "Success", "action": "view_help"},
        ])
        self.assertTrue(result["groups"]["transactional_actions"]["suppressed"])
        self.assertIsNone(result["groups"]["transactional_actions"]["numerator"])
        self.assertIsNone(result["difference"])

    def test_marketing_consent_is_labeled_risk_not_service_opportunity(self):
        sends = (
            [{"customer_id": "secret-a", "send_channel": "Email"}] * 10
            + [{"customer_id": "secret-b", "send_channel": "Email"}] * 10
            + [{"customer_id": "missing", "send_channel": "SMS"}] * 10
        )
        customers = {"secret-a": "false", "secret-b": "true"}
        result = aggregate_marketing_consent(sends, customers)
        self.assertEqual(result["type"], "risk")
        self.assertEqual(result["actionability"], "context_only")
        self.assertEqual(result["numerator"], 10)
        self.assertEqual(result["denominator"], 20)
        self.assertNotIn("secret-a", repr(result))
        self.assertNotIn("customer_id", repr(result))

    def test_marketing_consent_preserves_false_and_zero_values(self):
        sends = (
            [{"customer_id": "bool-false", "send_channel": "Email"}] * 10
            + [{"customer_id": "zero-false", "send_channel": "Email"}] * 10
            + [{"customer_id": "bool-true", "send_channel": "Email"}] * 10
        )
        result = aggregate_marketing_consent(
            sends, {"bool-false": False, "zero-false": 0, "bool-true": True}
        )
        self.assertEqual(result["numerator"], 20)
        self.assertEqual(result["denominator"], 30)

    def test_marketing_channel_suppression_prevents_overall_complement_recovery(self):
        sends = []
        consent = {}
        for channel in ("Email", "Push", "SMS", "Voice"):
            for index in range(20):
                customer_id = f"synthetic-{channel}-{index}"
                sends.append({"customer_id": customer_id, "send_channel": channel})
                consent[customer_id] = index % 2 == 0
        sends.append({"customer_id": "synthetic-whatsapp-small", "send_channel": "WhatsApp"})
        consent["synthetic-whatsapp-small"] = False

        result = aggregate_marketing_consent(sends, consent)

        self.assertEqual(result["numerator"], 41)
        self.assertEqual(result["denominator"], 81)
        self.assertEqual(set(result["by_channel"]), {"email", "push", "sms", "voice", "whatsapp"})
        for summary in result["by_channel"].values():
            self.assertTrue(summary["suppressed"])
            self.assertIsNone(summary["numerator"])
            self.assertIsNone(summary["denominator"])
            self.assertIsNone(summary["rate"])
            self.assertEqual(summary["suppression_reason"], "complementary_channel_suppression")

    def test_m4_m5_suppress_overall_and_unsupported_category_margins(self):
        pqrs = []
        for category in ("Transactions", "Fees", "Technical", "Branch", "Service"):
            count = 19 if category == "Transactions" else 200
            for index in range(count):
                event = category != "Transactions" and index % 2 == 0
                pqrs.append({
                    "complaint_id": f"synthetic-{category}-{index}",
                    "customer_id": f"synthetic-customer-{category}-{index}",
                    "category": category,
                    "status": "Open" if event else "Resolved",
                    "sla_breached": "true" if event else "false",
                })
        e0 = {
            "discovery": {"selected": 154, "total": 200},
            "replication": {"selected": 1433, "total": 1800},
            "complaint_ids_matched": 2000,
            "eligible_cases": 2000,
        }

        payload, audit = generate_v2_from_rows([], pqrs, [], [], [], {}, e0)

        for metric in ("M4", "M5"):
            overall = next(
                entry for entry in payload["entries"]
                if entry["metric_id"] == metric and entry["cell"] == {"scope": "overall"}
            )
            self.assertIsNone(overall["numerator"])
            self.assertIsNone(overall["denominator"])
            self.assertTrue(overall["snapshot"]["suppressed"])
            self.assertEqual(overall["snapshot"]["suppression_reason"], "complementary_suppression")
            unsupported = next(
                entry for entry in payload["entries"]
                if entry["metric_id"] == metric
                and entry["cell"] == {"pqr_category": "transactions"}
            )
            self.assertIsNone(unsupported["numerator"])
            self.assertIsNone(unsupported["denominator"])
            self.assertTrue(unsupported["snapshot"]["suppressed"])
            self.assertEqual(unsupported["snapshot"]["suppression_reason"], "complementary_suppression")
            audit_overall = next(
                row for row in audit["cells"]
                if row["metric_id"] == metric and row["cell"] == {"scope": "overall"}
            )
            self.assertIsNone(audit_overall["numerator"])
            self.assertTrue(audit_overall["snapshot"]["suppressed"])
            self.assertIsNone(audit_overall["replication"]["numerator"])
            audit_unsupported = next(
                row for row in audit["cells"]
                if row["metric_id"] == metric
                and row["cell"] == {"pqr_category": "transactions"}
            )
            self.assertIsNone(audit_unsupported["numerator"])
            self.assertIsNone(audit_unsupported["dx"])
            self.assertIsNone(audit_unsupported["sx"])

    def test_m6_replicated_contradiction_is_disclosed_but_remains_context_only(self):
        from opbench_v2 import _stat_entry_v2

        row = {
            "metric_id": "M6",
            "cell": {"reason_category": "complaint", "channel": "phone"},
            "status": "corroborated_descriptive",
            "numerator": 20,
            "denominator": 40,
            "snapshot": {"numerator": 20, "denominator": 40, "rate": 0.5, "suppressed": False, "suppression_reason": None},
            "effect": {"difference": 0.1, "ci95_low": 0.06, "ci95_high": 0.14},
            "replication": {"numerator": 20, "denominator": 40, "effect": {"difference": 0.1}, "adjusted_q": 0.2, "p_value": 0.2},
            "cells_explored": {"family_size": 95},
            "multiple_testing": {"method": "two_proportion_z_benjamini_hochberg_fdr", "family_size": 95, "adjusted_q": 0.2},
        }

        entry = _stat_entry_v2(row, 1)

        self.assertEqual(entry["status"], "corroborated_descriptive")
        self.assertEqual(entry["type"], "descriptive_only")
        self.assertEqual(entry["actionability"], "context_only")
        self.assertTrue(any("audit discrepancy" in caveat.casefold() for caveat in entry["caveats"]))

    def test_catalog_rejects_m6_contradiction_without_discrepancy_caveat(self):
        inferential = []
        for metric in ("M1", "M2", "M3", "M4", "M5", "M6", "E1"):
            for index, cell in enumerate(planned_cells_v2(metric)):
                entry = {
                    "id": f"{metric}-{index:02d}", "family": "synthetic", "title": "Synthetic test entry",
                    "metric_id": metric, "cell": cell, "definition": "synthetic test definition",
                    "numerator": None, "denominator": None,
                    "snapshot": {"numerator": None, "denominator": None, "suppressed": True},
                    "cells_explored": {"family_size": 95},
                    "multiple_testing": {
                        "method": "two_proportion_z_benjamini_hochberg_fdr",
                        "family_size": 95, "adjusted_q": None,
                    },
                    "status": "uncertain", "type": "descriptive_only",
                    "actionability": "covered_existing_capability" if metric == "E1" else "context_only",
                    "caveats": ["synthetic"],
                }
                if (metric, tuple(sorted(cell.items()))) in {
                    ("M2", (("channel", "phone"),)), ("M3", (("channel", "phone"),)),
                    ("M4", (("pqr_category", "technical"),)), ("M5", (("pqr_category", "technical"),)),
                }:
                    entry["negative_control_status"] = "audit_consistent"
                inferential.append(entry)
        descriptive = [
            {
                "id": metric, "family": "context", "title": "Synthetic context",
                "metric_id": metric, "cell": {"scope": "overall"},
                "definition": "synthetic context definition", "numerator": None,
                "denominator": None, "snapshot": {"suppressed": True},
                "cells_explored": {"family_size": 0},
                "multiple_testing": {
                    "method": "not_applicable_preregistered_descriptive", "adjusted_q": None,
                },
                "status": "context_only", "type": "risk" if metric == "R1" else "descriptive_only",
                "actionability": "context_only", "caveats": ["synthetic"],
            }
            for metric in ("D1", "R1", "L1")
        ]
        contradictory = next(
            entry for entry in inferential
            if entry["metric_id"] == "M6" and entry["cell"] != {"scope": "overall"}
        )
        contradictory["status"] = "corroborated_descriptive"

        with self.assertRaisesRegex(ValueError, "M6.*audit discrepancy"):
            build_v2_payload(inferential, descriptive, valid_coverage())

    def test_e0_complaint_linkage_releases_only_counts_and_rate(self):
        result = summarize_e0_complaint_linkage(
            total_eligible_cases=2000,
            complaint_ids_found_in_bank=2000,
        )
        self.assertEqual(result["actionability"], "context_only")
        self.assertEqual(result["numerator"], 2000)
        self.assertEqual(result["denominator"], 2000)
        self.assertEqual(result["rate"], 1.0)
        self.assertNotIn("complaint_id", repr(result))
        suppressed = summarize_e0_complaint_linkage(9, 9)
        self.assertTrue(suppressed["suppressed"])
        self.assertIsNone(suppressed["denominator"])
        with self.assertRaises(ValueError):
            summarize_e0_complaint_linkage(10, 11)

    def test_e0_pipeline_rejects_wrong_partition_denominators_and_impossible_linkage(self):
        base = {
            "discovery": {"selected": 154, "total": 200},
            "replication": {"selected": 1433, "total": 1800},
            "complaint_ids_matched": 2000,
            "eligible_cases": 2000,
        }
        for mutate in (
            lambda value: value["discovery"].update(total=199),
            lambda value: value["replication"].update(total=1801),
            lambda value: value["discovery"].update(selected=201),
            lambda value: value.update(complaint_ids_matched=2001),
            lambda value: value.update(eligible_cases=1999),
        ):
            candidate = json.loads(json.dumps(base))
            mutate(candidate)
            with self.assertRaisesRegex(ValueError, "E0 aggregate values"):
                generate_v2_from_rows([], [], [], [], [], {}, candidate)


if __name__ == "__main__":
    unittest.main()
