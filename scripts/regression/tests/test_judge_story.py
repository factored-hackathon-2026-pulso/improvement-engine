"""judge_story.py (W11): raw runs -> verdict story with the same rules as prove_fails_on_base.py. Offline, stdlib only."""
import json
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import judge_story  # noqa: E402

BUNDLE = json.loads((HERE / "results" / "reg-consultas-ffbb5234.bundle.json").read_text(encoding="utf-8"))
CAND = lambda n: json.loads((HERE / "fixtures" / "candidates" / f"{n}.json").read_text(encoding="utf-8"))["changes"]  # noqa: E731
ALL = BUNDLE["finding_case_ids"] + BUNDLE["guard_case_ids"]


def raw(verdict="pass", failing=()):
    return {"label": "x", "proposal_id": "p1", "verdict": verdict, "gate_items": [{"metric": "scenario/a", "phase": "gate", "passed": True}],
            "per_case_native": {c: {"passed": c not in failing, "reason": ""} for c in ALL}, "detail": None, "problem": None, "infra_retries": []}


class JudgeStory(unittest.TestCase):
    def test_static_base_fails_wording_probes_and_a_state_template_is_proven(self):
        doc = {"bundle": BUNDLE, "base": raw(), "attempts": [{"attempt": 1, "changes": CAND("estado_pqr_attempt1"), "run": raw()}]}
        s = judge_story.judge(doc)
        self.assertEqual((s["outcome"], s["announce"], s["judge"]), ("regression_suite_proven", True, "w11.judge_story/1"))
        self.assertEqual(len(s["base"]["failed_cases"]), 8)

    def test_noop_candidate_is_not_fixed_and_not_announced(self):
        s = judge_story.judge({"bundle": BUNDLE, "base": raw(), "attempts": [{"attempt": 1, "changes": CAND("estado_pqr_noop"), "run": raw()}]})
        self.assertEqual((s["outcome"], s["announce"]), ("not_fixed", False))

    def test_a_base_that_renders_the_state_is_non_discriminating(self):
        live = {"artifacts": [{"id": "t/estado_pqr", "locales": {"es": "Estado: {{ facts.pqr.value.status }}", "pt": "Estado: {{ facts.pqr.value.status }}"}}]}
        s = judge_story.judge({"bundle": BUNDLE, "base": raw(), "base_artifacts": live, "attempts": []})
        self.assertEqual(s["outcome"], "non_discriminating")

    def test_failed_infra_and_guard_regression(self):
        self.assertEqual(judge_story.judge({"bundle": BUNDLE, "base": raw("failed_infra"), "attempts": []})["outcome"], "infra_failed")
        g = BUNDLE["guard_case_ids"][0]
        s = judge_story.judge({"bundle": BUNDLE, "base": raw(), "attempts": [{"attempt": 1, "changes": CAND("estado_pqr_attempt1"), "run": raw("fail", [g])}]})
        self.assertEqual(s["outcome"], "guard_regressed")

    def test_cli_reads_stdin_and_rejects_garbage(self):
        ok = subprocess.run([sys.executable, str(HERE / "judge_story.py")], input=json.dumps({"bundle": BUNDLE, "base": raw(), "attempts": []}).encode(), capture_output=True)
        self.assertEqual((ok.returncode, json.loads(ok.stdout)["outcome"]), (0, "base_only"))
        bad = subprocess.run([sys.executable, str(HERE / "judge_story.py")], input=b"{not json", capture_output=True)
        self.assertEqual(bad.returncode, 2)


if __name__ == "__main__":
    unittest.main()
