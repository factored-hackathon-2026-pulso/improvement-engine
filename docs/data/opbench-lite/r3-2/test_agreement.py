import unittest
import json
import subprocess
import sys
import tempfile
from pathlib import Path

from agreement import ALL, JUDGED, compare, compare_labelers, validate_golden
from generate import generate_corpus


def proposal(pid, pair_id, variant, scores):
    labels = {f"R{i}": 1 for i in range(1, 13)}
    labels.update(scores)
    return {
        "id": pid,
        "pair_id": pair_id,
        "variant": variant,
        "proposal": {"id": pid, "agent_id": "disputas", "finding": "same synthetic finding",
                     "artifact_change": {"kind": "template", "target_id": "t/aclarar_cargo"},
                     "evidence_refs": [{"id": "ev-fixture", "k": 20}]},
        "base": {"artifact_id": "t/aclarar_cargo"},
        "labels": labels,
        "expected": scores,
    }


class AgreementTests(unittest.TestCase):
    def setUp(self):
        self.golden = {"proposals": [
            proposal("p-good", "pair-1", "good", {c: 2 for c in JUDGED}),
            proposal("p-bad", "pair-1", "bad", {c: 0 for c in JUDGED}),
        ]}
        self.artifacts = {
            "source": "fixture@abc123",
            "agents": ["disputas"],
            "artifacts": [{"id": "t/aclarar_cargo", "kind": "template"}],
            "synthetic_evidence": [{"id": "ev-fixture", "k": 20}],
        }
        self.judge = {"format": "pulso.judge-rows.v1", "rows": [
            {"id": pid, "criterion": criterion, "score": score}
            for pid, values in (("p-good", {c: 2 for c in JUDGED}), ("p-bad", {c: 0 for c in JUDGED}))
            for criterion, score in values.items()
        ]}

    def test_accepts_pair_complete_golden_and_fixture_ids(self):
        self.assertEqual([], validate_golden(self.golden, self.artifacts))

    def test_reports_exact_within_one_zero_gate_and_kappa(self):
        report = compare(self.golden, self.judge)
        self.assertEqual("pass1_unblinded_codex_labels", report["label_source"])
        self.assertEqual(12, report["n_compared"])
        self.assertEqual(1.0, report["exact_agreement"])
        self.assertEqual(1.0, report["within_one_agreement"])
        self.assertEqual(1.0, report["hard_gate_agreement"])
        self.assertEqual(1.0, report["cohen_kappa_unweighted"])
        self.assertEqual(1.0, report["pairwise_winner_agreement"])
        self.assertIn("pass1_order", report["pairwise"][0])
        self.assertNotIn("human_order", report["pairwise"][0])

    def test_incomplete_judge_pair_is_not_included_in_pairwise_ranking(self):
        golden = {"proposals": [*self.golden["proposals"],
                                proposal("p-good-2", "pair-2", "good", {c: 2 for c in JUDGED}),
                                proposal("p-bad-2", "pair-2", "bad", {c: 0 for c in JUDGED})]}
        report = compare(golden, self.judge)
        self.assertEqual(24, report["n_expected"])
        self.assertEqual(12, report["n_compared"])
        self.assertEqual(1, report["pairwise_n"])
        self.assertEqual(["pair-1"], [pair["pair_id"] for pair in report["pairwise"]])

    def test_mismatch_output_names_pass1_not_human(self):
        judge = {"format": "pulso.judge-rows.v1", "rows": [dict(row) for row in self.judge["rows"]]}
        judge["rows"][0]["score"] = 1
        mismatch = compare(self.golden, judge)["mismatches"][0]
        self.assertEqual(2, mismatch["pass1"])
        self.assertEqual(1, mismatch["judge"])
        self.assertNotIn("human", mismatch)

    def test_refuses_aggregate_only_calibration_output_for_kappa(self):
        with self.assertRaisesRegex(ValueError, "row-level"):
            compare(self.golden, {"status": "exercised", "exact": 0.9, "within_1": 1.0})

    def test_accepts_legacy_aggregate_summary_but_never_invents_kappa(self):
        legacy = {"status": "exercised", "proposals": 12, "pairs": 70,
                  "exact": 0.909, "within_1": 0.986, "hard_gate": 0.97,
                  "proposal_gate_agreement": 0.92,
                  "per_criterion": {"R1": {"n": 12, "exact": 0.91, "within_1": 1.0, "hard_gate": 1.0}}}
        report = compare(self.golden, legacy)
        self.assertEqual("aggregate_only_not_comparable", report["status"])
        self.assertEqual(0.909, report["exact_agreement"])
        self.assertEqual(0.986, report["within_one_agreement"])
        self.assertEqual(0.97, report["hard_gate_agreement"])
        self.assertIsNone(report["cohen_kappa_unweighted"])
        self.assertIn("pass-1 labels", report["cohen_kappa_note"])
        self.assertTrue(report["not_comparable_to_current_golden"])

    def test_not_exercised_summary_reports_no_agreement_metrics(self):
        report = compare(self.golden, {"status": "not_exercised", "reason": "no gateway"})
        self.assertEqual("not_exercised", report["status"])
        self.assertIsNone(report["exact_agreement"])
        self.assertIsNone(report["cohen_kappa_unweighted"])

    def test_compares_two_blind_labelers_and_pairwise_order(self):
        pass1 = {
            "good": {"scores": [2] * 12, "note": "synthetic"},
            "bad": {"scores": [0] * 12, "note": "synthetic"},
        }
        manifest = {"format": "pulso.blind-label-manifest.v1", "items": {
            "opaque-a": {"source_id": "good", "pair_id": "pair-1", "variant": "good"},
            "opaque-b": {"source_id": "bad", "pair_id": "pair-1", "variant": "bad"},
        }}
        pass2 = {"scores": {"opaque-a": [2] * 12, "opaque-b": [0] * 12}}
        report = compare_labelers(pass1, manifest, pass2)
        self.assertEqual(24, report["n_compared"])
        self.assertEqual(1.0, report["exact_agreement"])
        self.assertEqual(1.0, report["within_one_agreement"])
        self.assertEqual(1.0, report["cohen_kappa_unweighted"])
        self.assertEqual(1.0, report["hard_gate_agreement"])
        self.assertEqual(1.0, report["pairwise_winner_agreement"])

    def test_cli_runs_blind_labeler_comparison(self):
        pass1 = {"good": {"scores": [2] * 12}, "bad": {"scores": [0] * 12}}
        manifest = {"format": "pulso.blind-label-manifest.v1", "items": {
            "opaque-a": {"source_id": "good", "pair_id": "pair-1", "variant": "good"},
            "opaque-b": {"source_id": "bad", "pair_id": "pair-1", "variant": "bad"},
        }}
        pass2 = {"scores": {"opaque-a": [2] * 12, "opaque-b": [0] * 12}}
        with tempfile.TemporaryDirectory() as tmp:
            paths = []
            for name, value in (("pass1.json", pass1), ("manifest.json", manifest), ("pass2.json", pass2)):
                path = Path(tmp) / name
                path.write_text(json.dumps(value), encoding="utf-8")
                paths.append(path)
            script = Path(__file__).with_name("agreement.py")
            result = subprocess.run(
                [sys.executable, str(script), "--pass1", str(paths[0]), "--manifest", str(paths[1]),
                 "--pass2", str(paths[2])], capture_output=True, text=True, check=False)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(1.0, json.loads(result.stdout)["exact_agreement"])

    def test_rejects_missing_or_invalid_second_rater_labels(self):
        pass1 = {"good": {"scores": [2] * 12}, "bad": {"scores": [0] * 12}}
        manifest = {
            "opaque-a": {"source_id": "good", "pair_id": "pair-1", "variant": "good"},
            "opaque-b": {"source_id": "bad", "pair_id": "pair-1", "variant": "bad"},
        }
        with self.assertRaisesRegex(ValueError, "case IDs"):
            compare_labelers(pass1, manifest, {"scores": {"opaque-a": [2] * 12}})
        with self.assertRaisesRegex(ValueError, "0, 1, or 2"):
            compare_labelers(pass1, manifest, {"scores": {"opaque-a": [True] * 12, "opaque-b": [0] * 12}})

    def test_rejects_manifest_that_duplicates_a_pass1_proposal(self):
        pass1 = {"good": {"scores": [2] * 12}, "bad": {"scores": [0] * 12}}
        manifest = {
            "opaque-a": {"source_id": "good", "pair_id": "pair-1", "variant": "good"},
            "opaque-b": {"source_id": "good", "pair_id": "pair-1", "variant": "bad"},
        }
        pass2 = {"scores": {"opaque-a": [2] * 12, "opaque-b": [0] * 12}}
        with self.assertRaisesRegex(ValueError, "each pass-1 proposal exactly once"):
            compare_labelers(pass1, manifest, pass2)

    def test_rejects_duplicate_or_out_of_range_scores(self):
        bad = {"format": "pulso.judge-rows.v1", "rows": [
            {"id": "p-good", "criterion": "R1", "score": 2},
            {"id": "p-good", "criterion": "R1", "score": 1},
        ]}
        with self.assertRaisesRegex(ValueError, "duplicate"):
            compare(self.golden, bad)

    def test_detects_invalid_manual_labels_and_unregistered_target(self):
        altered = {"proposals": [dict(x) for x in self.golden["proposals"]]}
        altered["proposals"][0] = dict(altered["proposals"][0])
        altered["proposals"][0]["labels"] = dict(altered["proposals"][0]["labels"])
        altered["proposals"][0]["labels"]["R3"] = True
        errors = validate_golden(altered, self.artifacts)
        self.assertTrue(any("R3" in error for error in errors))
        changed = {"proposals": [dict(x) for x in self.golden["proposals"]]}
        changed["proposals"][0] = dict(changed["proposals"][0])
        changed["proposals"][0]["proposal"] = dict(changed["proposals"][0]["proposal"])
        changed["proposals"][0]["proposal"]["artifact_change"] = {"kind": "template", "target_id": "t/not-real"}
        self.assertTrue(any("target_id" in error for error in validate_golden(changed, self.artifacts)))

    def test_generator_builds_full_paired_synthetic_corpus(self):
        golden, artifact_index = generate_corpus()
        self.assertEqual(40, len(golden["proposals"]))
        self.assertEqual(20, len({x["pair_id"] for x in golden["proposals"]}))
        self.assertEqual([], validate_golden(golden, artifact_index))
        self.assertEqual({"patch", "template", "new_agent"},
                         {x["proposal"]["artifact_change"]["kind"] for x in golden["proposals"]})
        self.assertTrue(all("labels" not in x["proposal"] and "quality_band" not in x["proposal"]
                            for x in golden["proposals"]))
        self.assertEqual({"clearly_good", "borderline", "clearly_bad"},
                         {x["design_stratum"] for x in golden["proposals"]})
        self.assertIn("es", {loc for x in golden["proposals"]
                              for loc in x["proposal"]["artifact_change"].get("locales", {})})
        self.assertIn("pt", {loc for x in golden["proposals"]
                              for loc in x["proposal"]["artifact_change"].get("locales", {})})

    def test_generator_cli_emits_json_to_stdout_without_writing_artifacts(self):
        path = Path(__file__).with_name("generate.py")
        result = subprocess.run([sys.executable, str(path), "--emit", "corpus"],
                                capture_output=True, text=True, check=False)
        self.assertEqual(0, result.returncode, result.stderr)
        emitted = json.loads(result.stdout)
        self.assertEqual(40, len(emitted["proposals"]))
        self.assertEqual("synthetic_only", emitted["authorship"])


if __name__ == "__main__":
    unittest.main()
