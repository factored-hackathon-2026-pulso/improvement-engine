"""GT0 report generator: every number and label in docs/reports/gates/gt0-report.md comes from an artifact."""
import json
import sys
import tempfile
import unittest
from pathlib import Path

GOV = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GOV))
sys.path.insert(0, str(GOV / "tests"))
import gt0_report as rep  # noqa: E402
from test_gt0_gate import build  # noqa: E402


class Report(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.d = Path(self._t.name)

    def tearDown(self):
        self._t.cleanup()

    def gen(self, mutate=None):
        m = build(self.d, mutate)
        return rep.generate(m)

    def test_report_states_the_gate_verdict_and_every_item(self):
        text = self.gen()
        self.assertIn("Verdict: PASS", text)
        for item in ("g0p", "trn0", "crv0", "honesty", "scanner_ids", "doubles", "freeze", "replay", "live_window", "capacity"):
            self.assertIn(f"| {item} |", text)

    def test_failing_item_makes_the_verdict_fail_and_is_listed(self):
        text = self.gen(lambda d, m: (d / "trn0.json").unlink())
        self.assertIn("Verdict: FAIL", text)
        self.assertIn("missing: trn0.json", text)

    def test_numbers_come_from_the_artifacts(self):
        def mut(d, m):
            r = json.loads((d / "replay.json").read_text())
            r.update(run_id="run-from-artifact-7", calls=4242)
            (d / "replay.json").write_text(json.dumps(r))
        text = self.gen(mut)
        self.assertIn("run-from-artifact-7", text)
        self.assertIn("4242", text)

    def test_step_table_and_doubles_are_copied_from_the_report_artifact(self):
        text = self.gen()
        self.assertIn("| scout | agent_roleplay | generated_sample | python |", text)
        self.assertIn("| jev |", text)

    def test_missing_artifact_is_marked_missing_never_invented(self):
        text = self.gen(lambda d, m: (d / "live.json").unlink())
        self.assertIn("MISSING: live.json", text)
        self.assertNotIn("4.1", text)

    def test_overrides_are_disclosed_as_simulated(self):
        text = self.gen()
        self.assertIn("simulated human", text)
        self.assertIn("quality_claims: forbidden", text)

    def test_open_review_findings_are_listed_as_open_risks(self):
        def mut(d, m):
            p = d / "reviews" / "x.review.json"
            r = json.loads(p.read_text())
            r["findings"] = [{"id": "TPS-9", "loop": 1, "summary": "opaque ids pass", "status": "open"}]
            r["verdict"] = "open"
            p.write_text(json.dumps(r))
        text = self.gen(mut)
        self.assertIn("TPS-9", text)
        self.assertIn("opaque ids pass", text)

    def test_frozen_digests_are_read_from_the_pin_file(self):
        text = self.gen()
        pin = json.loads((GOV.parents[1] / "contracts" / "engine-run" / "FREEZE.json").read_text())
        self.assertIn(pin["contracts"]["C-2"]["digest"], text)
        self.assertIn(pin["contracts"]["C-12"]["digest"], text)

    def test_write_creates_the_report_file(self):
        m = build(self.d)
        out = self.d / "out" / "gt0-report.md"
        rep.write(m, out)
        self.assertTrue(out.read_text(encoding="utf-8").startswith("# GT0"))


if __name__ == "__main__":
    unittest.main()
