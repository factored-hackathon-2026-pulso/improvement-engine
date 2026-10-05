"""ART2 offline tests: tool-link and policy-threshold suites (fail on base by construction, boundary guards)."""
import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import build_suite as bs  # noqa: E402
import prove_fails_on_base as pf  # noqa: E402

FINDING = json.loads((HERE / "fixtures" / "finding_m4_pqr_status.json").read_text(encoding="utf-8"))
LINK = {**FINDING, "art2": {"agent": "consultas", "tool": "leer_pqr_cliente", "statuses": ["error", "timeout", "denied"]}}
POL = {**FINDING, "art2": {"agent": "disputas", "policy": "escalamiento-disputa-monto", "old": 500, "new": 250,
                           "boundaries": [249, 250, 251, 499, 500, 501]}}


class ToolLink(unittest.TestCase):
    def test_every_finding_case_seeds_a_tool_failure_of_the_linked_tool(self):
        b = bs.build_suite(LINK, "tool_link:consultas/leer_pqr_cliente")
        self.assertEqual(b["agent"], "consultas")
        by = {s["id"]: s for s in b["suite"]["scenarios"]}
        self.assertEqual(len(b["finding_case_ids"]), 6)
        for cid in b["finding_case_ids"]:
            self.assertIn("leer_pqr_cliente", by[cid]["seed"]["tools"])
            self.assertEqual(by[cid]["expect"], {"outcome": "escalated", "escalated": True})
        self.assertIn("guard-link-pasa-sin-cambio", b["guard_case_ids"])
        self.assertEqual(len(b["guard_case_ids"]), 4)

    def test_without_params_the_suite_is_refused(self):
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(FINDING, "tool_link:consultas/leer_pqr_cliente")
        self.assertEqual(cm.exception.code, "params_missing")

    def test_coverage_says_what_is_not_measured(self):
        b = bs.build_suite(LINK, "tool_link:consultas/leer_pqr_cliente")
        c = pf.coverage(b, {"state": "not_applicable"})
        self.assertIn("answer_uses_tool_data", c["not_measured"])
        self.assertEqual(c["native"][0], "link_tool_failure_exits")


class PolicyThreshold(unittest.TestCase):
    def test_window_cases_and_boundary_guards_on_old_and_new_threshold(self):
        b = bs.build_suite(POL, "policy:escalamiento-disputa-monto")
        self.assertEqual(sorted({c.split("-")[-1] for c in b["finding_case_ids"]}), ["251", "499", "500"])
        g = b["guard_case_ids"]
        for gid in ("guard-monto-249-sin-escalar", "guard-monto-250-sin-escalar", "guard-monto-501-escala"):
            self.assertIn(gid, g)
        by = {s["id"]: s for s in b["suite"]["scenarios"]}
        self.assertEqual(by["guard-monto-250-sin-escalar"]["expect"]["outcome"], "resolved")
        self.assertEqual(by["guard-monto-501-escala"]["expect"]["outcome"], "escalated")

    def test_no_pii_shape_in_generated_text(self):
        b = bs.build_suite(POL, "policy:escalamiento-disputa-monto")
        self.assertEqual(bs.pii_problems(json.dumps([[s["steps"], s.get("seed")] for s in b["suite"]["scenarios"]])), [])


if __name__ == "__main__":
    unittest.main()
