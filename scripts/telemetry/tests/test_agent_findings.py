import os
import unittest
from pathlib import Path

from scripts.telemetry import agent_findings as F
from scripts.telemetry import agent_signals as S
from scripts.telemetry.tests.fixtures import balanced, make_export, make_run

CLI = os.environ.get("TEL1_STEPS_CLI", "D:/cargo-targets/claude-tel1/debug/steps_cli.exe")


def planted_export():
    """disputas/pt hands off 65% (others 25%); disputas always calls obtener_pqr; consultas varies."""
    def b(i, half):
        agent = ["disputas", "consultas", "recepcion"][i % 3]
        locale = "pt" if (i // 3) % 2 else "es"
        hot = agent == "disputas" and locale == "pt"
        h = (i // 6) % 20 < (13 if hot else 5)
        tool = "obtener_pqr" if agent == "disputas" else ["obtener_pqr", "radicar_pqr", "leer_movimientos"][(i // 3) % 3]
        return make_run(i, agent, locale, [{"tool": tool}], outcome="escalated" if h else "resolved",
                        closed_by="escalation" if h else "flow", fallback=0)
    return make_export(balanced(400, b))


class MappingTests(unittest.TestCase):
    def test_every_level_and_metric_has_a_mapping_row_with_association_caveat(self):
        for m in S.METRIC_NAMES:
            row = F.map_finding({"metric": m, "dims": {"agent": "x", "locale": "es"}})
            self.assertIsNotNone(row, m)
            self.assertIn("association_not_cause", row["caveats"])
        self.assertEqual(F.map_finding({"kind": "always_same_tool"})["hypothesis"], "prompt_or_flow_review")
        self.assertEqual(F.map_finding({"metric": "A1", "dims": {"agent": "d", "locale": "pt"}})["hypothesis"],
                         "prompt_language_policy_check")


@unittest.skipUnless(Path(CLI).exists(), "steps_cli not built")
class EndToEnd(unittest.TestCase):
    def test_planted_locale_handoff_and_always_tool_found_in_separate_lists(self):
        rep = S.aggregate_agent_signals(planted_export())
        out = F.build_findings(rep, F.run_sensor(S.render_ndjson(rep["cells"]), CLI), True)
        self.assertEqual(out["evidence_class"], "agent_runs")
        self.assertEqual(out["traffic"], "synthetic_battery_not_real_customers")
        hand = [f for f in out["comparative_findings"] if f["metric"] == "A1" and f["dims"] == {"agent": "disputas", "locale": "pt"}]
        self.assertTrue(hand and hand[0]["status"] in {"corroborated", "candidate"})
        self.assertEqual(hand[0]["mapping"]["hypothesis"], "prompt_language_policy_check")
        self.assertEqual([l["agent"] for l in out["level_findings"]], ["disputas"])
        self.assertTrue(all(f["evidence_class"] == "agent_runs" for f in out["comparative_findings"]))


if __name__ == "__main__":
    unittest.main()
