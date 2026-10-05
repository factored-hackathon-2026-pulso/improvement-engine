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

    def test_metric_variants_collapse(self):
        self.assertEqual(sf.norm_metric("M6L"), "M6")
        self.assertEqual(sf.norm_metric("M1"), "M1")

    def test_direction_from_effect(self):
        self.assertEqual(sf.direction_of(0.2), "up")
        self.assertEqual(sf.direction_of(-0.2), "down")
        self.assertEqual(sf.direction_of(0.0005), "none")


class ScoringTests(unittest.TestCase):
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
                                "--catalog", cp, "--signals", sp, "--out", op], capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stderr)
            with open(op) as f:
                self.assertEqual(json.load(f)["scores"]["recall"], 1.0)


class LevelRiskScoringTests(unittest.TestCase):
    def cat(self, *entries):
        c = fx.catalog()
        c["entries"].extend(entries)
        return c

    def sigs(self, *extra):
        s = fx.signals_perfect()
        s["signals"].extend(extra)
        return s

    def test_level_risk_matches_a_risk_catalog_entry(self):
        r = sf.score(self.cat(fx.risk_entry()), self.sigs(fx.level_sig()))
        self.assertEqual(r["scores"]["recall"], 1.0)
        self.assertEqual(r["scores"]["precision"], 1.0)
        self.assertEqual(r["risk"]["positives"], 1)
        self.assertEqual(r["risk"]["matched"], 1)
        self.assertEqual(r["risk"]["recall"], 1.0)
        self.assertEqual(r["counts"]["nonfinding_reports"], 0)

    def test_missing_level_risk_lowers_only_risk_recall(self):
        r = sf.score(self.cat(fx.risk_entry()), fx.signals_perfect())
        self.assertEqual(r["scores"]["recall"], 1.0)
        self.assertEqual(r["risk"]["recall"], 0.0)
        self.assertEqual([e["id"] for e in r["risk"]["unmatched_benchmark_risks"]], ["T-R1"])

    def test_level_risk_without_a_catalog_entry_is_listed_not_penalised(self):
        r = sf.score(fx.catalog(), self.sigs(fx.level_sig()))
        self.assertEqual(r["scores"]["precision"], 1.0)
        self.assertEqual(r["scores"]["recall"], 1.0)
        self.assertEqual(r["counts"]["reported"], 3)
        self.assertEqual(len(r["risk"]["unmatched_level_risks"]), 1)
        self.assertIsNone(r["risk"]["recall"])

    def test_level_risk_is_never_matched_to_a_problem_entry(self):
        c = self.cat()
        c["entries"].append({"id": "T-P", "type": "problem", "status": "corroborated", "metric_id": "M8",
                             "cell": {}, "effect": {"difference": 0.4}})
        r = sf.score(c, self.sigs(fx.level_sig()))
        self.assertAlmostEqual(r["scores"]["recall"], 3 / 4, places=5)
        self.assertEqual(r["risk"]["positives"], 0)

    def test_contrast_signal_is_never_matched_to_a_risk_entry(self):
        r = sf.score(self.cat(fx.risk_entry()), self.sigs(fx.sig("M8", {"channel": "Sms"})))
        self.assertEqual(r["risk"]["matched"], 0)

    def test_refuted_risk_entry_is_a_nonfinding_for_a_level_risk(self):
        r = sf.score(self.cat(fx.risk_entry(status="refuted")), self.sigs(fx.level_sig()))
        self.assertEqual(r["counts"]["nonfinding_reports"], 1)
        self.assertEqual(r["nonfinding_reports"][0]["id"], "T-R1")

    def test_non_corroborated_level_risk_is_not_reported(self):
        r = sf.score(self.cat(fx.risk_entry()), self.sigs(fx.level_sig(status="refuted")))
        self.assertEqual(r["risk"]["matched"], 0)
        self.assertEqual(r["risk"]["unmatched_level_risks"], [])


if __name__ == "__main__":
    unittest.main()
