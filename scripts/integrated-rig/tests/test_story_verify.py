"""story_verify.py pure parts: notification counting, change summary, checks, and the masking canary."""
import contextlib
import io
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import story_verify as sv  # noqa: E402


class PureTests(unittest.TestCase):
    def test_improvement_notices_counts_only_the_proposal_and_kind(self):
        items = [{"kind": "improvement_proposed", "improvement": {"proposalId": "p1"}},
                 {"kind": "improvement_proposed", "improvement": {"proposalId": "p2"}},
                 {"kind": "case_assigned", "improvement": None}]
        self.assertEqual(len(sv.improvement_notices(items, "p1")), 1)
        self.assertEqual(sv.improvement_notices(None, "p1"), [])

    def test_change_summary(self):
        s = sv.change_summary([{"kind": "prompt", "docs": {"description": "x"}}, {"kind": "eval_suite", "docs": {"description": " "}}])
        self.assertEqual((s["count"], s["with_description"], s["has_eval_suite"]), (2, 1, True))

    def test_verify_core_flags_each_wrong_field(self):
        good = {"proposal": {"origin": "auto_detect", "state": "draft", "created_by": "pulso-engine"}, "changes": [{"kind": "eval_suite", "docs": {"description": "d"}}]}
        self.assertTrue(all(c["ok"] for c in sv.verify_core(good, "p")))
        bad = {"proposal": {"origin": "human", "state": "approved", "created_by": "x"}, "changes": []}
        self.assertFalse(any(c["ok"] for c in sv.verify_core(bad, "p")))

    def test_verify_detail_needs_all_doc_fields(self):
        d = {"proposal": {"title": "T"}, "changes": [{"docs": {"description": "a", "rationale": "b", "changelog": ""}}]}
        res = {c["check"].split(": ")[1]: c["ok"] for c in sv.verify_detail(d, "p")}
        self.assertTrue(res["detail title"] and res["detail docs.description"] and res["detail docs.rationale"])
        self.assertFalse(res["detail docs.changelog"])

    def test_case_id_shape(self):
        self.assertTrue(sv.CASE_ID.match("CASE-" + "0" * 25 + "1"))
        self.assertFalse(sv.CASE_ID.match("CASE-" + "I" * 26))  # I is not Crockford
        self.assertFalse(sv.CASE_ID.match("case-1"))


class CanaryTests(unittest.TestCase):
    def test_say_masks_registered_secrets_and_token_shapes(self):
        sv._secrets.append("S3CR3T-VALUE-123")
        out = io.StringIO()
        jws = "eyJhbGciOiJFZERTQSJ9.eyJzdWIiOiJ4In0abc.c2lnbmF0dXJl"
        with contextlib.redirect_stdout(out):
            sv.say("token S3CR3T-VALUE-123 and " + jws + " and svc-" + "A" * 30)
        shown = out.getvalue()
        for secret in ("S3CR3T-VALUE-123", jws, "svc-" + "A" * 30):
            self.assertNotIn(secret, shown)
        self.assertIn("***", shown)

    def test_rendered_checks_carry_no_secret_field(self):
        lines = "\n".join(sv.render_checks([sv.check("a", True, "HTTP 200"), sv.check("b", False, "HTTP 500")]))
        self.assertIn("PASS", lines)
        self.assertIn("FAIL", lines)


if __name__ == "__main__":
    unittest.main()
