"""ED0 first RED: the winning category must follow the DATA, not the label.

Needs the built Rust local-sim sensor: set ED0_RUNNER_EXE to improvement-engine.exe
(built with CARGO_TARGET_DIR=D:/cargo-targets/claude-ed0). Fixtures are synthetic.
"""
import os
import tempfile
import unittest

from claude_standin import ed0_detect as ed0

EXE = os.environ.get("ED0_RUNNER_EXE")


@unittest.skipUnless(EXE and os.path.exists(EXE), "ED0_RUNNER_EXE not set")
class RenamePermuteMutation(unittest.TestCase):
    def detect(self, labels):
        with tempfile.TemporaryDirectory() as tmp:
            pkg = os.path.join(tmp, "pkg")
            ed0.write_synthetic_e0(pkg, labels)
            return ed0.detect(EXE, pkg, os.path.join(tmp, "out"))

    def test_rename_changes_winning_category(self):
        base = self.detect({"A": "alpha-pattern", "B": "beta-pattern"})
        renamed = self.detect({"A": "zeta-pattern", "B": "beta-pattern"})
        self.assertNotEqual(base["winner"], renamed["winner"])
        self.assertEqual(base["winner_support"], renamed["winner_support"])

    def test_permute_moves_winner_with_label(self):
        base = self.detect({"A": "alpha-pattern", "B": "beta-pattern"})
        swapped = self.detect({"A": "beta-pattern", "B": "alpha-pattern"})
        # group A (bigger) now carries the label that B had: winner identity follows A's rows
        self.assertEqual(base["winner_support"], swapped["winner_support"])
        again = self.detect({"A": "alpha-pattern", "B": "beta-pattern"})
        self.assertEqual(base["winner"], again["winner"])  # deterministic
        self.assertNotEqual(base["winner"], swapped["winner"])

    def test_report_labels(self):
        d = self.detect({"A": "alpha-pattern", "B": "beta-pattern"})
        self.assertEqual(d["data_origin"], "generated_sample")
        self.assertEqual(d["admitted_family"], "e0_recurring_copilot_query_cases")
        self.assertIsInstance(d["discards"], list)
        self.assertEqual(d["providers"], ["local"])


if __name__ == "__main__":
    unittest.main()
