import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, HERE)
import fixtures as fx  # noqa: E402
import score_findings as sf  # noqa: E402


def _producer_stage(diff=0.3):
    baseline_rate = 0.25
    rate = baseline_rate + diff
    return {"numerator": round(rate * 100), "denominator": 100, "rate": rate,
            "baseline_numerator": 25, "baseline_denominator": 100,
            "baseline_rate": baseline_rate, "diff": diff, "p": 0.01}


def _producer_report(signals, cells_explored=1, discards=None):
    return {
        "semantics": "claude-standin",
        "method": {
            "test": "two_proportion_z_pooled_vs_same_channel_excluding_own_reason",
            "multiplicity": "benjamini_hochberg_all_explored_cells",
            "min_ratio": 1.25,
            "replication": "discovery_holdout_hash_split",
            "secondary_replication": "r2_windows_2023-07..2024-12_vs_2025-01..2026-05",
            "alpha": 0.01,
            "min_effect": 0.05,
            "min_support": 500,
            "k_min": 10,
        },
        "cells_explored": cells_explored,
        "signals": signals,
        "discards": discards or [],
    }


# Keep test signals aligned with the producer's required complete Stage shape.
_signals_perfect = fx.signals_perfect


def _complete_signals_perfect():
    signals = _signals_perfect()
    report = _producer_report(signals["signals"], signals["cells_explored"], signals.get("discards"))
    for signal in report["signals"]:
        diff = signal["discovery"]["diff"]
        signal.update({
            "discovery": _producer_stage(diff),
            "holdout": _producer_stage(diff),
            "reason": "replicated_in_holdout",
            "p_adj": 0.001,
            "r2": {"status": "replicated", "w1": _producer_stage(diff), "w2": _producer_stage(diff)},
        })
    return report


_sig = fx.sig


def _complete_sig(*args, **kwargs):
    signal = _sig(*args, **kwargs)
    diff = signal["discovery"]["diff"]
    if signal["status"] == "refuted":
        signal.update({"dims": {}, "reason": "no_differential", "direction": "none"})
        signal.pop("discovery")
        return signal
    signal["discovery"] = _producer_stage(diff)
    if signal["status"] == "candidate":
        signal.update({"reason": "holdout_unavailable", "p_adj": 0.001,
                       "r2": {"status": "not_evaluated"}})
    elif signal["status"] == "uncertain":
        signal["reason"] = "not_significant_after_correction"
    else:
        signal.update({"reason": "replicated_in_holdout", "holdout": _producer_stage(diff),
                       "p_adj": 0.001,
                       "r2": {"status": "replicated", "w1": _producer_stage(diff),
                              "w2": _producer_stage(diff)}})
    return signal




class _ScoringFixtures:
    """Local adapter; do not mutate the shared imported fixture module."""
    def __getattr__(self, name):
        if name == "signals_perfect":
            return _complete_signals_perfect
        if name == "sig":
            return _complete_sig
        return getattr(_source_fixtures, name)


_source_fixtures = fx
fx = _ScoringFixtures()


class NormalizationTests(unittest.TestCase):
    def test_vocabulary_aliases(self):
        self.assertEqual(sf.norm_value("Queja"), "complaint")
        self.assertEqual(sf.norm_value("App"), "mobile_app")
        self.assertEqual(sf.norm_value("IVR"), "phone")
        self.assertEqual(sf.norm_value("  Teléfono "), "phone")

    def test_v2_cell_normalization_uses_frozen_reason_and_metric_channel_vocabularies(self):
        self.assertEqual(
            sf.norm_cell({"reason_category": "Comercial", "channel": "Phone"}, "M1"),
            (("channel", "phone"), ("reason_category", "commercial")),
        )
        self.assertEqual(
            sf.norm_cell({"reason_category": "Técnico", "channel": "Web Chat"}, "M1"),
            (("channel", "web_chat"), ("reason_category", "technical")),
        )
        self.assertEqual(
            sf.norm_cell({"reason_category": "Queja", "channel": "SMS"}, "M6L"),
            (("channel", "other"), ("reason_category", "complaint")),
        )

    def test_metric_variants_collapse(self):
        self.assertEqual(sf.norm_metric("M6L"), "M6")
        self.assertEqual(sf.norm_metric("M1"), "M1")

    def test_only_linked_survey_metric_aliases_to_m6(self):
        self.assertEqual(sf.norm_metric("M6L"), "M6")
        self.assertEqual(sf.norm_metric("M6R"), "M6R")
        self.assertEqual(sf.norm_metric("M6U"), "M6U")
        self.assertEqual(sf.norm_metric("M6Z"), "M6Z")

    def test_direction_from_effect(self):
        self.assertEqual(sf.direction_of(0.2), "up")
        self.assertEqual(sf.direction_of(-0.2), "down")
        self.assertEqual(sf.direction_of(0.0005), "none")


class ScoringTests(unittest.TestCase):
    def test_v2_spanish_sensor_labels_match_frozen_catalog_cells(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "2", "entries": [
            {"id": "M1-COMMERCIAL-PHONE", "type": "problem", "status": "corroborated",
             "metric_id": "M1", "cell": {"reason_category": "commercial", "channel": "phone"},
             "effect": {"difference": 0.12}},
            {"id": "M6-OTHER-COMPLAINT", "type": "problem", "status": "corroborated",
             "metric_id": "M6", "cell": {"reason_category": "complaint", "channel": "other"},
             "effect": {"difference": 0.29}},
        ]}
        signals = _producer_report([
            fx.sig("M1", {"reason_category": "Comercial", "channel": "Phone"}, diff=0.12),
            fx.sig("M6L", {"reason_category": "Queja", "channel": "SMS"}, diff=0.29),
        ], cells_explored=2)
        result = sf.score(catalog, signals)
        self.assertEqual(result["scores"]["recall"], 1.0)
        self.assertEqual(result["scores"]["precision"], 1.0)
        self.assertEqual({item["id"] for item in result["matched_findings"]},
                         {"M1-COMMERCIAL-PHONE", "M6-OTHER-COMPLAINT"})

    def test_v2_unknown_source_label_fails_closed_instead_of_becoming_a_cell(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "2", "entries": [
            {"id": "M1-COMPLAINT-PHONE", "type": "problem", "status": "corroborated",
             "metric_id": "M1", "cell": {"reason_category": "complaint", "channel": "phone"},
             "effect": {"difference": 0.12}},
        ]}
        signals = _producer_report([
            fx.sig("M1", {"reason_category": "quejas", "channel": "llamada"}, diff=0.12),
        ])
        with self.assertRaisesRegex(sf.ScoringError, "dimension value is unsupported"):
            sf.score(catalog, signals)

    def test_v2_resolved_submetric_does_not_match_aggregate_m6(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "2", "entries": [
            {"id": "M6-COMPLAINT-PHONE", "type": "problem", "status": "corroborated",
             "metric_id": "M6", "cell": {"reason_category": "complaint", "channel": "phone"},
             "effect": {"difference": 0.12}},
        ]}
        result = sf.score(catalog, _producer_report([
            fx.sig("M6R", {"reason_category": "Queja", "channel": "Phone"}, diff=0.12),
        ]))
        self.assertEqual(result["scores"]["recall"], 0.0)
        self.assertEqual(result["unmatched_engine_findings"][0]["metric"], "M6R")

    def test_v1_catalog_retains_its_legacy_value_vocabulary(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "1.0.0", "entries": [
            {"id": "legacy", "type": "problem", "status": "corroborated", "metric_id": "M1",
             "cell": {"reason_category": "complaint", "channel": "phone"},
             "effect": {"difference": 0.12}},
        ]}
        result = sf.score(catalog, _producer_report([
            fx.sig("M1", {"reason_category": "queja", "channel": "llamada"}, diff=0.12),
        ]))
        self.assertEqual(result["scores"]["recall"], 1.0)

    def test_scorer_rejects_unrecognized_catalog_version(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "3", "entries": []}
        with self.assertRaisesRegex(sf.ScoringError, "unsupported catalog version"):
            sf.score(catalog, _producer_report([], cells_explored=0))

    def test_v2_score_discloses_cross_protocol_comparison_limit(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "2", "entries": []}
        result = sf.score(catalog, _producer_report([], cells_explored=0))
        self.assertIn("different customer splits", result["validation_limitations"])
        self.assertIn("not independent accuracy", result["interpretation"])

    def test_v2_spanish_survey_alias_matches_refuted_cell_as_nonfinding(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "2", "entries": [
            {"id": "M6-13", "type": "problem", "status": "refuted", "metric_id": "M6",
             "cell": {"reason_category": "technical", "channel": "mobile_app"},
             "effect": {"difference": 0.0}},
        ]}
        signals = _producer_report([
            fx.sig("M6L", {"reason_category": "Técnico", "channel": "App"}, diff=0.08),
        ])
        result = sf.score(catalog, signals)
        self.assertEqual(result["nonfinding_reports"][0]["id"], "M6-13")

    def test_perfect_run(self):
        r = sf.score(fx.catalog(), fx.signals_perfect())
        self.assertEqual(r["scores"]["recall"], 1.0)
        self.assertEqual(r["scores"]["precision"], 1.0)
        self.assertEqual(r["scores"]["ranking_agreement_spearman"], 1.0)
        self.assertEqual(r["nonfinding_reports"], [])

    def test_missing_finding_lowers_recall(self):
        s = fx.signals_perfect()
        s["signals"].pop()
        r = sf.score(fx.catalog(), s)
        self.assertAlmostEqual(r["scores"]["recall"], 2 / 3, places=5)
        self.assertEqual([m["id"] for m in r["unmatched_benchmark_findings"]], ["T-03"])

    def test_reporting_a_nonfinding_hurts_precision(self):
        s = fx.signals_perfect()
        s["signals"].append(fx.sig("M2", {"channel": "Phone"}, diff=0.2))
        r = sf.score(fx.catalog(), s)
        self.assertAlmostEqual(r["scores"]["precision"], 3 / 4, places=5)
        self.assertEqual(r["nonfinding_reports"][0]["id"], "T-05")

    def test_wrong_direction_is_rejected_by_current_producer_contract(self):
        s = fx.signals_perfect()
        s["signals"][0]["direction"] = "down"
        with self.assertRaisesRegex(sf.ScoringError, "cell signal direction"):
            sf.score(fx.catalog(), s)

    def test_non_corroborated_signals_are_not_reported(self):
        s = fx.signals_perfect()
        s["signals"].append(fx.sig("M2", {"channel": "Phone"}, status="refuted", direction="none"))
        s["signals"].append(fx.sig("M4", {"category": "technical"}, status="uncertain"))
        r = sf.score(fx.catalog(), s)
        self.assertEqual(r["scores"]["precision"], 1.0)

    def test_candidates_counted_only_when_asked(self):
        s = fx.signals_perfect()
        s["signals"][2]["status"] = "candidate"
        s["signals"][2]["reason"] = "holdout_unavailable"
        s["signals"][2].pop("holdout")
        s["signals"][2]["r2"] = {"status": "not_evaluated"}
        self.assertAlmostEqual(sf.score(fx.catalog(), s)["scores"]["recall"], 2 / 3, places=5)
        r = sf.score(fx.catalog(), s, include_candidate=True)
        self.assertEqual(r["scores"]["recall"], 1.0)

    def test_unknown_cell_is_unmatched_and_penalised_but_listed_separately(self):
        s = fx.signals_perfect()
        s["signals"].append(fx.sig("M1", {"reason_category": "retention", "channel": "Web"}))
        r = sf.score(fx.catalog(), s)
        self.assertAlmostEqual(r["scores"]["precision"], 3 / 4, places=5)
        self.assertEqual(len(r["unmatched_engine_findings"]), 1)
        self.assertEqual(r["nonfinding_reports"], [])

    def test_acceptable_disagreement_is_excluded_from_precision(self):
        s = fx.signals_perfect()
        s["signals"].append(fx.sig("M1", {"reason_category": "retention", "channel": "Web"}))
        ok = [{"metric_id": "M1", "cell": {"reason_category": "retention", "channel": "web"}}]
        r = sf.score(fx.catalog(), s, acceptable=ok)
        self.assertEqual(r["scores"]["precision"], 1.0)

    def test_ranking_disagreement(self):
        s = fx.signals_perfect()
        s["signals"][0]["discovery"]["diff"] = 0.1  # catalog says T-01 is the largest effect
        s["signals"][0]["discovery"].update(
            baseline_numerator=40, baseline_rate=0.4, rate=0.5, numerator=50
        )
        r = sf.score(fx.catalog(), s)
        self.assertLess(r["scores"]["ranking_agreement_spearman"], 1.0)

    def test_effect_tolerance_flag(self):
        s = fx.signals_perfect()
        s["signals"][0]["discovery"]["diff"] = 0.10
        s["signals"][0]["discovery"].update(
            baseline_numerator=40, baseline_rate=0.4, rate=0.5, numerator=50
        )
        r = sf.score(fx.catalog(), s, effect_tolerance=0.05)
        flagged = {m["id"]: m["effect_within_tolerance"] for m in r["matched_findings"]}
        self.assertFalse(flagged["T-01"])
        self.assertTrue(flagged["T-02"])

    def test_empty_engine_output(self):
        r = sf.score(fx.catalog(), _producer_report([], cells_explored=0))
        self.assertEqual(r["scores"]["recall"], 0.0)
        self.assertIsNone(r["scores"]["precision"])
        self.assertIsNone(r["scores"]["ranking_agreement_spearman"])

    def test_catalog_without_positive_entries_has_no_recall(self):
        c = fx.catalog()
        c["entries"] = [e for e in c["entries"] if e["status"] != "corroborated"]
        r = sf.score(c, fx.signals_perfect())
        self.assertIsNone(r["scores"]["recall"])

    def test_malformed_input_rejected(self):
        with self.assertRaises(sf.ScoringError):
            sf.score({"nope": 1}, fx.signals_perfect())
        with self.assertRaises(sf.ScoringError):
            sf.score(fx.catalog(), {"signals": "x"})

    def test_signal_stage_rejects_sub_k_positive_support(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["discovery"].update(numerator=9, denominator=100)
        with self.assertRaisesRegex(sf.ScoringError, "privacy floor"):
            sf.score(fx.catalog(), signals)

    def test_signal_stage_rejects_sub_k_complement_support(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["discovery"].update(numerator=91, denominator=100)
        with self.assertRaisesRegex(sf.ScoringError, "privacy floor"):
            sf.score(fx.catalog(), signals)

    def test_signal_stage_rejects_sub_k_denominator_and_accepts_zero_binary_side(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["discovery"].update(numerator=0, denominator=9)
        with self.assertRaisesRegex(sf.ScoringError, "privacy floor"):
            sf.score(fx.catalog(), signals)

        signals["signals"][0]["discovery"].update(numerator=0, denominator=20)
        signals["signals"][0]["discovery"].update(
            rate=0.0, baseline_numerator=20, baseline_denominator=100, baseline_rate=0.2, diff=-0.2
        )
        sf._validate_signal_stage(signals["signals"][0]["discovery"], "synthetic")

    def test_signal_stage_rejects_unavailable_effect(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["discovery"]["diff"] = None
        with self.assertRaisesRegex(sf.ScoringError, "diff must be finite"):
            sf.score(fx.catalog(), signals)

    def test_present_stages_require_full_producer_count_envelope(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["discovery"] = {"diff": 0.2}
        with self.assertRaisesRegex(sf.ScoringError, "producer stage fields"):
            sf.score(fx.catalog(), signals)

    def test_producer_reason_and_discard_kinds_are_closed_vocabularies(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["reason"] = "made_up_reason"
        with self.assertRaisesRegex(sf.ScoringError, "reason is unsupported"):
            sf.score(fx.catalog(), signals)

        signals = fx.signals_perfect()
        signals["discards"] = [{"kind": "unknown_bucket", "count": 10}]
        with self.assertRaisesRegex(sf.ScoringError, "discard kind is unsupported"):
            sf.score(fx.catalog(), signals)

    def test_all_current_producer_reason_codes_are_accepted(self):
        corroborated = fx.signals_perfect()["signals"][0]
        reversed_holdout = json.loads(json.dumps(corroborated))
        reversed_holdout.update(status="refuted", reason="holdout_direction_reversed",
                                holdout=_producer_stage(diff=-0.1))
        uncertain_holdout = json.loads(json.dumps(corroborated))
        uncertain_holdout.update(status="uncertain", reason="holdout_not_significant",
                                 holdout=_producer_stage(diff=0.01))
        cases = [
            fx.sig("M1", {"reason_category": "complaint", "channel": "phone"}, status="uncertain"),
            fx.sig("M1", {"reason_category": "complaint", "channel": "phone"}, status="candidate"),
            reversed_holdout, corroborated, uncertain_holdout,
            fx.sig("M9", {}, status="refuted", direction="none"),
        ]
        for signal in cases:
            sf.score(fx.catalog(), _producer_report([signal]))

    def test_adjusted_p_value_must_be_a_probability(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["p_adj"] = 1.1
        with self.assertRaisesRegex(sf.ScoringError, "p_adj"):
            sf.score(fx.catalog(), signals)

    def test_unhashable_method_and_dependency_values_fail_closed(self):
        report = fx.signals_perfect()
        report["method"]["multiplicity"] = []
        with self.assertRaises(sf.ScoringError):
            sf.score(fx.catalog(), report)

        report = fx.signals_perfect()
        report["signals"][0]["reason"] = []
        with self.assertRaises(sf.ScoringError):
            sf.score(fx.catalog(), report)

        report = fx.signals_perfect()
        report["signals"][0]["discovery"]["diff"] = []
        with self.assertRaises(sf.ScoringError):
            sf.score(fx.catalog(), report)

        report = fx.signals_perfect()
        report["signals"][0]["depends_on"] = []
        with self.assertRaises(sf.ScoringError):
            sf.score(fx.catalog(), report)

    def test_metric_and_dimension_vocabularies_fail_closed(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["metric"] = "customer_id"
        with self.assertRaisesRegex(sf.ScoringError, "metric is unsupported"):
            sf.score(fx.catalog(), signals)

        signals = fx.signals_perfect()
        signals["signals"][0]["dims"] = {"pqr_category": "technical"}
        with self.assertRaisesRegex(sf.ScoringError, "closed aggregate keys"):
            sf.score(fx.catalog(), signals)

        signals = fx.signals_perfect()
        signals["signals"][0]["dims"]["reason_category"] = "arbitrary-short-label"
        with self.assertRaisesRegex(sf.ScoringError, "dimension value is unsupported"):
            sf.score(fx.catalog(), signals)

        signals = fx.signals_perfect()
        signals["signals"][0]["metric"] = "M7"
        signals["signals"][0]["dims"] = {"action": "initiate_payment", "channel": "web"}
        with self.assertRaisesRegex(sf.ScoringError, "dimension vocabulary is not documented"):
            sf.score(fx.catalog(), signals)

    def test_no_differential_is_the_only_aggregate_signal_shape(self):
        aggregate = {"metric": "M9", "dims": {}, "status": "refuted", "reason": "no_differential",
                     "direction": "none", "claim": "association"}
        report = _producer_report([aggregate], cells_explored=0)
        sf.score(fx.catalog(), report)

        aggregate["discovery"] = _producer_stage()
        with self.assertRaisesRegex(sf.ScoringError, "no_differential aggregate shape"):
            sf.score(fx.catalog(), _producer_report([aggregate], cells_explored=0))

    def test_cell_signal_status_requires_producer_specific_evidence(self):
        signal = fx.signals_perfect()["signals"][0]
        signal["status"] = "candidate"
        signal["reason"] = "holdout_unavailable"
        signal.pop("holdout")
        signal.pop("r2")
        with self.assertRaisesRegex(sf.ScoringError, "status requires"):
            sf.score(fx.catalog(), _producer_report([signal]))

    def test_stage_rate_and_effect_must_match_rounded_counts(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["discovery"]["rate"] = 0.75
        with self.assertRaisesRegex(sf.ScoringError, "rate does not match counts"):
            sf.score(fx.catalog(), signals)

    def test_stage_requires_baseline_support_proof_and_enforces_its_privacy_floor(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["discovery"].pop("baseline_numerator", None)
        signals["signals"][0]["discovery"].pop("baseline_denominator", None)
        with self.assertRaisesRegex(sf.ScoringError, "producer stage fields"):
            sf.score(fx.catalog(), signals)

        for numerator, denominator in ((9, 100), (91, 100), (0, 9)):
            signals = fx.signals_perfect()
            signals["signals"][0]["discovery"].update(
                baseline_numerator=numerator, baseline_denominator=denominator
            )
            with self.subTest(numerator=numerator, denominator=denominator), self.assertRaisesRegex(
                    sf.ScoringError, "privacy floor"):
                sf.score(fx.catalog(), signals)

    def test_baseline_rate_must_match_its_aggregate_support(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["discovery"]["baseline_rate"] = 0.41
        with self.assertRaisesRegex(sf.ScoringError, "baseline_rate does not match counts"):
            sf.score(fx.catalog(), signals)

    def test_r2_status_must_agree_with_both_window_stages(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["r2"]["status"] = "reversed"
        with self.assertRaisesRegex(sf.ScoringError, "R2 status contradicts windows"):
            sf.score(fx.catalog(), signals)

        signals = fx.signals_perfect()
        signals["signals"][0]["r2"]["w1"] = _producer_stage(diff=-0.1)
        with self.assertRaisesRegex(sf.ScoringError, "R2 status contradicts windows"):
            sf.score(fx.catalog(), signals)

    def test_holdout_reason_status_must_match_observed_holdout(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["holdout"] = _producer_stage(diff=-0.1)
        with self.assertRaisesRegex(sf.ScoringError, "holdout reason contradicts evidence"):
            sf.score(fx.catalog(), signals)

        signals = fx.signals_perfect()
        signals["signals"][0]["status"] = "candidate"
        with self.assertRaisesRegex(sf.ScoringError, "status requires holdout evidence"):
            sf.score(fx.catalog(), signals)

    def test_report_requires_exact_producer_envelope(self):
        with self.assertRaisesRegex(sf.ScoringError, "producer summary fields"):
            sf.score(fx.catalog(), {"cells_explored": 0, "signals": []})

        signal = fx.signals_perfect()["signals"][0]
        signal.pop("claim")
        with self.assertRaisesRegex(sf.ScoringError, "required producer fields"):
            sf.score(fx.catalog(), _producer_report([signal]))

    def test_holdout_and_secondary_window_counts_use_the_same_privacy_floor(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["holdout"] = {**_producer_stage(), "numerator": 9, "denominator": 100}
        with self.assertRaisesRegex(sf.ScoringError, "privacy floor"):
            sf.score(fx.catalog(), signals)

        signals = fx.signals_perfect()
        signals["signals"][0]["r2"] = {
            "status": "replicated",
            "w1": {**_producer_stage(), "numerator": 91, "denominator": 100},
        }
        with self.assertRaisesRegex(sf.ScoringError, "privacy floor"):
            sf.score(fx.catalog(), signals)

    def test_signal_summary_and_discard_counts_fail_closed(self):
        base = _producer_report([], cells_explored=10)
        below_k = json.loads(json.dumps(base))
        below_k["discards"] = [{"kind": "k_violation", "count": 9}]
        extra_field = json.loads(json.dumps(base))
        extra_field["discards"] = [{"kind": "k_violation", "count": 10, "extra": True}]
        bad_discard_type = json.loads(json.dumps(base))
        bad_discard_type["discards"] = "none"
        bool_cells = json.loads(json.dumps(base))
        bool_cells["cells_explored"] = True
        negative_cells = json.loads(json.dumps(base))
        negative_cells["cells_explored"] = -1
        invalid_summaries = [
            bool_cells, negative_cells, bad_discard_type, below_k, extra_field,
        ]
        for summary in invalid_summaries:
            with self.subTest(summary=summary), self.assertRaises(sf.ScoringError):
                sf.score(fx.catalog(), summary)

    def test_signal_summary_accepts_suppressed_discard_count(self):
        signals = fx.signals_perfect()
        signals["discards"] = [{"kind": "k_violation", "count": None, "suppressed": True}]
        result = sf.score(fx.catalog(), signals)
        self.assertAlmostEqual(result["scores"]["recall"], 1.0)

    def test_signal_dimensions_reject_source_identifiers_and_unknown_fields(self):
        signals = fx.signals_perfect()
        signals["signals"][0]["dims"] = {"customer_id": "synthetic-customer-123"}
        with self.assertRaisesRegex(sf.ScoringError, "privacy"):
            sf.score(fx.catalog(), signals)

        signals = fx.signals_perfect()
        signals["signals"][0]["free_text"] = "synthetic only"
        with self.assertRaisesRegex(sf.ScoringError, "unknown signal field"):
            sf.score(fx.catalog(), signals)

    def test_cli_roundtrip(self):
        with tempfile.TemporaryDirectory() as d:
            cp, sp, op = (os.path.join(d, n) for n in ("c.json", "s.json", "o.json"))
            with open(cp, "w") as f:
                json.dump(fx.catalog(), f)
            with open(sp, "w") as f:
                json.dump(fx.signals_perfect(), f)
            p = subprocess.run([sys.executable, os.path.join(os.path.dirname(HERE), "score_findings.py"),
                                "--catalog", cp, "--signals", sp, "--out", op], capture_output=True, text=True,
                               cwd=d)
            self.assertEqual(p.returncode, 0, p.stderr)
            with open(op) as f:
                self.assertEqual(json.load(f)["scores"]["recall"], 1.0)

    def test_cli_rejects_sub_k_signal_counts(self):
        with tempfile.TemporaryDirectory() as d:
            cp, sp = (os.path.join(d, n) for n in ("c.json", "s.json"))
            catalog = fx.catalog()
            signals = fx.signals_perfect()
            signals["signals"][0]["discovery"].update(numerator=4, denominator=100)
            with open(cp, "w") as f:
                json.dump(catalog, f)
            with open(sp, "w") as f:
                json.dump(signals, f)
            p = subprocess.run([sys.executable, os.path.join(os.path.dirname(HERE), "score_findings.py"),
                                "--catalog", cp, "--signals", sp], capture_output=True, text=True, cwd=d)
            self.assertEqual(p.returncode, 2)
            self.assertIn("privacy floor", p.stderr)


if __name__ == "__main__":
    unittest.main()
