"""GT05 report generator: every number and label comes from an artifact; a missing artifact prints as MISSING."""
import json
import sys
import tempfile
import unittest
from pathlib import Path

GOV = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GOV))
sys.path.insert(0, str(GOV / "tests"))
import gt05_report as rep  # noqa: E402
import test_gt05_gate as tg  # noqa: E402


class Report(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.d = Path(self._t.name)

    def tearDown(self):
        self._t.cleanup()

    def gen(self, mutate=None, **kw):
        m = tg.build(self.d, mutate)
        return rep.generate(m, gt0_evaluator=tg.GT0_OK, **kw)

    def test_report_lists_every_item_with_verdict_and_artifact_facts(self):
        text = self.gen()
        for item in ("rust_flips", "exe_sha256", "fallback_disclosure", "g1_check", "gt0_dependency", "ratchet_default", "crv1", "rg1"):
            self.assertIn(item, text)
        self.assertIn("Verdict: PASS", text)
        for sid in tg.FLIPS:
            self.assertIn(sid, text)
        self.assertIn(tg.EXE_SHA[:12], text)

    def test_failure_and_missing_are_printed_not_hidden(self):
        def mut(d, m):
            m["artifacts"].pop("original_run")
        text = self.gen(mut)
        self.assertIn("Verdict: FAIL", text)
        self.assertIn("MISSING", text)
        self.assertIn("FAIL", text)

    def test_missing_manifest_prints_missing(self):
        self.assertIn("MISSING", rep.generate(self.d / "nope.json"))

    def test_report_is_deterministic(self):
        self.assertEqual(self.gen(), rep.generate(self.d / "manifest.json", gt0_evaluator=tg.GT0_OK))


if __name__ == "__main__":
    unittest.main()
