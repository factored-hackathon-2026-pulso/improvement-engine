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
        signals = {"cells_explored": 2, "signals": [
            fx.sig("M1", {"reason_category": "Comercial", "channel": "Phone"}, diff=0.12),
            fx.sig("M6L", {"reason_category": "Queja", "channel": "SMS"}, diff=0.29),
        ]}
        result = sf.score(catalog, signals)
        self.assertEqual(result["scores"]["recall"], 1.0)
        self.assertEqual(result["scores"]["precision"], 1.0)
        self.assertEqual({item["id"] for item in result["matched_findings"]},
                         {"M1-COMMERCIAL-PHONE", "M6-OTHER-COMPLAINT"})

    def test_v2_unsupported_legacy_aliases_do_not_match_frozen_cells(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "2", "entries": [
            {"id": "M1-COMPLAINT-PHONE", "type": "problem", "status": "corroborated",
             "metric_id": "M1", "cell": {"reason_category": "complaint", "channel": "phone"},
             "effect": {"difference": 0.12}},
        ]}
        signals = {"cells_explored": 1, "signals": [
            fx.sig("M1", {"reason_category": "quejas", "channel": "llamada"}, diff=0.12),
        ]}
        result = sf.score(catalog, signals)
        self.assertEqual(result["scores"]["recall"], 0.0)
        self.assertEqual(result["unmatched_engine_findings"][0]["dims"],
                         {"reason_category": "quejas", "channel": "llamada"})

    def test_v2_resolved_submetric_does_not_match_aggregate_m6(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "2", "entries": [
            {"id": "M6-COMPLAINT-PHONE", "type": "problem", "status": "corroborated",
             "metric_id": "M6", "cell": {"reason_category": "complaint", "channel": "phone"},
             "effect": {"difference": 0.12}},
        ]}
        result = sf.score(catalog, {"cells_explored": 1, "signals": [
            fx.sig("M6R", {"reason_category": "Queja", "channel": "Phone"}, diff=0.12),
        ]})
        self.assertEqual(result["scores"]["recall"], 0.0)
        self.assertEqual(result["unmatched_engine_findings"][0]["metric"], "M6R")

    def test_v1_catalog_retains_its_legacy_value_vocabulary(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "1.0.0", "entries": [
            {"id": "legacy", "type": "problem", "status": "corroborated", "metric_id": "M1",
             "cell": {"reason_category": "complaint", "channel": "phone"},
             "effect": {"difference": 0.12}},
        ]}
        result = sf.score(catalog, {"cells_explored": 1, "signals": [
            fx.sig("M1", {"reason_category": "quejas", "channel": "llamada"}, diff=0.12),
        ]})
        self.assertEqual(result["scores"]["recall"], 1.0)

    def test_scorer_rejects_unrecognized_catalog_version(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "3", "entries": []}
        with self.assertRaisesRegex(sf.ScoringError, "unsupported catalog version"):
            sf.score(catalog, {"cells_explored": 0, "signals": []})

    def test_v2_score_discloses_cross_protocol_comparison_limit(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "2", "entries": []}
        result = sf.score(catalog, {"cells_explored": 0, "signals": []})
        self.assertIn("different customer splits", result["validation_limitations"])
        self.assertIn("not independent accuracy", result["interpretation"])

    def test_v2_spanish_survey_alias_matches_refuted_cell_as_nonfinding(self):
        catalog = {"benchmark": "OPBENCH-lite", "version": "2", "entries": [
            {"id": "M6-13", "type": "problem", "status": "refuted", "metric_id": "M6",
             "cell": {"reason_category": "technical", "channel": "mobile_app"},
             "effect": {"difference": 0.0}},
        ]}
        signals = {"cells_explored": 1, "signals": [
            fx.sig("M6L", {"reason_category": "Técnico", "channel": "App"}, diff=0.08),
        ]}
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

    def test_wrong_direction_is_not_a_match(self):
        s = fx.signals_perfect()
        s["signals"][0]["direction"] = "down"
        r = sf.score(fx.catalog(), s)
        self.assertAlmostEqual(r["scores"]["recall"], 2 / 3, places=5)

    def test_non_corroborated_signals_are_not_reported(self):
        s = fx.signals_perfect()
        s["signals"].append(fx.sig("M2", {"channel": "Phone"}, status="refuted", direction="none"))
        s["signals"].append(fx.sig("M4", {"pqr_category": "technical"}, status="uncertain"))
        r = sf.score(fx.catalog(), s)
        self.assertEqual(r["scores"]["precision"], 1.0)

    def test_candidates_counted_only_when_asked(self):
        s = fx.signals_perfect()
        s["signals"][2]["status"] = "candidate"
        self.assertAlmostEqual(sf.score(fx.catalog(), s)["scores"]["recall"], 2 / 3, places=5)
        r = sf.score(fx.catalog(), s, include_candidate=True)
        self.assertEqual(r["scores"]["recall"], 1.0)

    def test_unknown_cell_is_unmatched_and_penalised_but_listed_separately(self):
        s = fx.signals_perfect()
        s["signals"].append(fx.sig("M1", {"reason_category": "Consulta", "channel": "Web"}))
        r = sf.score(fx.catalog(), s)
        self.assertAlmostEqual(r["scores"]["precision"], 3 / 4, places=5)
        self.assertEqual(len(r["unmatched_engine_findings"]), 1)
        self.assertEqual(r["nonfinding_reports"], [])

    def test_acceptable_disagreement_is_excluded_from_precision(self):
        s = fx.signals_perfect()
        s["signals"].append(fx.sig("M1", {"reason_category": "Consulta", "channel": "Web"}))
        ok = [{"metric_id": "M1", "cell": {"reason_category": "consulta", "channel": "web"}}]
        r = sf.score(fx.catalog(), s, acceptable=ok)
        self.assertEqual(r["scores"]["precision"], 1.0)

    def test_ranking_disagreement(self):
        s = fx.signals_perfect()
        s["signals"][0]["discovery"]["diff"] = 0.1  # catalog says T-01 is the largest effect
        r = sf.score(fx.catalog(), s)
        self.assertLess(r["scores"]["ranking_agreement_spearman"], 1.0)

    def test_effect_tolerance_flag(self):
        s = fx.signals_perfect()
        s["signals"][0]["discovery"]["diff"] = 0.10
        r = sf.score(fx.catalog(), s, effect_tolerance=0.05)
        flagged = {m["id"]: m["effect_within_tolerance"] for m in r["matched_findings"]}
        self.assertFalse(flagged["T-01"])
        self.assertTrue(flagged["T-02"])

    def test_empty_engine_output(self):
        r = sf.score(fx.catalog(), {"cells_explored": 0, "signals": []})
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


if __name__ == "__main__":
    unittest.main()
