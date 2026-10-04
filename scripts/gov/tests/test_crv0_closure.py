"""CRV0: review-log format, reviewer != author, closure script."""
import copy
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

GOV = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GOV))
import crv0_closure as crv  # noqa: E402

REVIEWS = GOV.parents[1] / "docs" / "reviews" / "claude"


def log(**kw):
    d = {"schema": "review-log/v1", "wps": ["TPS"], "author": {"id": "implementer-a"},
         "reviewer": {"id": "reviewer-b", "context": "fresh"}, "provenance": "contemporaneous",
         "findings": [{"id": "TPS-1", "loop": 1, "summary": "x", "status": "fixed", "fix_ref": "abc1234"}],
         "verdict": "closed"}
    d.update(kw)
    return d


def write(d, name, doc):
    Path(d, name).write_text(json.dumps(doc), encoding="utf-8")


class Format(unittest.TestCase):
    def test_good_log_is_valid_and_closed(self):
        self.assertEqual(crv.check_log(log()), [])
        self.assertTrue(crv.is_closed(log()))

    def test_reviewer_must_differ_from_author_after_normalisation(self):
        for rid in ("implementer-a", " Implementer_A "):
            self.assertTrue(any("reviewer" in p for p in crv.check_log(log(reviewer={"id": rid, "context": "fresh"}))))

    def test_missing_reviewer_or_author_is_invalid(self):
        self.assertTrue(crv.check_log(log(reviewer={})))
        self.assertTrue(crv.check_log(log(author={})))

    def test_fixed_needs_fix_ref_and_accepted_needs_reason(self):
        f = {"id": "X", "loop": 1, "summary": "s", "status": "fixed"}
        self.assertTrue(crv.check_log(log(findings=[f])))
        f = {"id": "X", "loop": 1, "summary": "s", "status": "accepted"}
        self.assertTrue(crv.check_log(log(findings=[f])))
        f["reason"] = "documented residual risk"
        self.assertEqual(crv.check_log(log(findings=[f])), [])

    def test_closed_verdict_with_open_finding_is_invalid(self):
        f = {"id": "X", "loop": 1, "summary": "s", "status": "open"}
        self.assertTrue(any("open" in p for p in crv.check_log(log(findings=[f]))))
        self.assertFalse(crv.is_closed(log(findings=[f], verdict="open")))

    def test_reconstructed_needs_sources(self):
        self.assertTrue(crv.check_log(log(provenance="reconstructed")))
        self.assertEqual(crv.check_log(log(provenance="reconstructed", sources=["journal CL-0041"])), [])

    def test_unknown_status_or_schema_is_invalid(self):
        f = {"id": "X", "loop": 1, "summary": "s", "status": "meh"}
        self.assertTrue(crv.check_log(log(findings=[f])))
        self.assertTrue(crv.check_log(log(schema="other")))


class Closure(unittest.TestCase):
    def run_cli(self, d, required="TPS"):
        r = subprocess.run([sys.executable, str(GOV / "crv0_closure.py"), "--reviews", d, "--required", required],
                           capture_output=True, text=True)
        return r

    def test_first_red_no_logs_exits_1(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(self.run_cli(d).returncode, 1)

    def test_closed_log_for_every_required_wp_exits_0(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "tps.review.json", log())
            r = self.run_cli(d)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_open_finding_exits_1(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "tps.review.json", log(verdict="open", findings=[{"id": "X", "loop": 1, "summary": "s", "status": "open"}]))
            r = self.run_cli(d)
            self.assertEqual(r.returncode, 1)
            self.assertIn("TPS", r.stdout)

    def test_required_wp_without_log_exits_1(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "tps.review.json", log())
            self.assertEqual(self.run_cli(d, "TPS,DC0").returncode, 1)

    def test_same_reviewer_and_author_exits_1(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "tps.review.json", log(reviewer={"id": "implementer-a", "context": "fresh"}))
            self.assertEqual(self.run_cli(d).returncode, 1)

    def test_invalid_json_exits_1(self):
        with tempfile.TemporaryDirectory() as d:
            Path(d, "bad.review.json").write_text("{", encoding="utf-8")
            self.assertEqual(self.run_cli(d).returncode, 1)


class BackfilledLogs(unittest.TestCase):
    """The committed logs: format-valid, honestly reconstructed, and covering the CRV0 dependency set."""

    def logs(self):
        return [json.loads(p.read_text(encoding="utf-8")) for p in sorted(REVIEWS.glob("*.review.json"))]

    def test_every_committed_log_is_format_valid(self):
        self.assertTrue(self.logs())
        for lg in self.logs():
            self.assertEqual(crv.check_log(lg), [], lg.get("wps"))

    def test_backfilled_logs_are_marked_reconstructed_and_cite_the_journal(self):
        for lg in self.logs():
            self.assertEqual(lg["provenance"], "reconstructed")
            self.assertTrue(any("CL-00" in s for s in lg["sources"]))
            self.assertIs(lg["reviewer"]["identity_recorded"], False)

    def test_required_dependency_set_is_covered(self):
        covered = {w for lg in self.logs() for w in lg["wps"]}
        self.assertEqual(set(crv.DEFAULT_REQUIRED) - covered, set())


if __name__ == "__main__":
    unittest.main()
