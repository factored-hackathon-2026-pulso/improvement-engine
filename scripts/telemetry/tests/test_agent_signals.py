import copy
import json
import unittest

from scripts.aggregate.agent_runs.cell_table import parse_cell_ndjson, render_cell_ndjson
from scripts.telemetry import agent_signals as S
from scripts.telemetry.tests.fixtures import balanced, make_export, make_run

ALW = [{"tool": "obtener_pqr"}]


def _cells(report, metric):
    return [c for c in report["cells"] if c["metric"] == metric]


class MetricTests(unittest.TestCase):
    def build(self):
        # disputas/es: handoff in 40%, always tool lookup, error 30%; consultas/pt: varied tools, no handoff
        def b(i, half):
            if i % 2 == 0:
                hand = (i // 2) % 5 < 2
                err = (i // 2) % 10 < 3
                return make_run(i, "disputas", "es", [{"tool": "obtener_pqr", "status": "error" if err else "ok", "attempt": 2 if err else 1}],
                                outcome="escalated" if hand else "resolved", closed_by="escalation" if hand else "flow",
                                fallback=1 if err else 0)
            tool = ["obtener_pqr", "radicar_pqr", "leer_movimientos"][(i // 2) % 3]
            return make_run(i, "consultas", "pt", [{"tool": tool}] * (3 if (i // 2) % 4 == 0 else 1), outcome="resolved", fallback=0)
        return make_export(balanced(80, b))

    def setUp(self):
        self.report = S.aggregate_agent_signals(self.build())

    def test_evidence_and_protocol(self):
        self.assertEqual(self.report["evidence_class"], "agent_runs")
        self.assertEqual(self.report["source_label_class"], "synthetic")

    def test_cells_follow_six_field_schema_and_k(self):
        text = S.render_ndjson(self.report["cells"])
        rows = parse_cell_ndjson(text)  # raises on a k violation or extra field
        self.assertTrue(rows)
        self.assertTrue(all(r["metric"].startswith("A") for r in rows))

    def test_handoff_rate_by_agent_locale(self):
        cells = [c for c in _cells(self.report, "A1") if c["dims"] == {"agent": "disputas", "locale": "es"}]
        self.assertEqual({c["half"] for c in cells}, {"discovery", "holdout"})
        num = sum(c["numerator"] for c in cells)
        den = sum(c["denominator"] for c in cells)
        self.assertEqual(den, 80)
        self.assertAlmostEqual(num / den, 0.4, delta=0.1)

    def test_tool_error_over_tool_calling_runs_only(self):
        for c in _cells(self.report, "A4"):
            self.assertIn(c["dims"]["agent"], {"disputas", "consultas"})
        a4 = [c for c in _cells(self.report, "A4") if c["dims"]["agent"] == "disputas"]
        self.assertTrue(a4 and all(c["numerator"] >= 10 for c in a4))

    def test_tool_share_cells_and_always_same_tool_level(self):
        a5 = [c for c in _cells(self.report, "A5") if c["dims"] == {"agent": "disputas", "tool": "obtener_pqr"}]
        self.assertTrue(a5 and all(c["numerator"] == c["denominator"] for c in a5))
        levels = S.level_findings(self.report["cells"])
        hit = [l for l in levels if l["agent"] == "disputas"]
        self.assertEqual(len(hit), 1)
        self.assertEqual(hit[0]["kind"], "always_same_tool")
        self.assertEqual(hit[0]["tool"], "obtener_pqr")
        self.assertEqual(hit[0]["evidence_class"], "agent_runs")
        self.assertEqual(hit[0]["claim"], "association")
        self.assertFalse([l for l in levels if l["agent"] == "consultas"])

    def test_repeated_call_retry_fallback_closed_early(self):
        a6 = [c for c in _cells(self.report, "A6") if c["dims"]["agent"] == "consultas"]
        self.assertTrue(a6)  # 3 identical calls in a run
        for m in ("A2", "A7"):
            self.assertTrue(_cells(self.report, m), m)

    def test_deterministic_under_permutation(self):
        e = self.build()
        p = copy.deepcopy(e)
        p["runs"]["pages"][0]["items"].reverse()
        a = json.dumps(S.aggregate_agent_signals(e), sort_keys=True)
        b = json.dumps(S.aggregate_agent_signals(p), sort_keys=True)
        self.assertEqual(a, b)

    def test_no_identifier_in_output(self):
        blob = json.dumps(self.report)
        for banned in ("run-0", "c1", "release", "1.0.0"):
            self.assertNotIn(banned, blob)


class PrivacyTests(unittest.TestCase):
    def test_small_volume_publishes_nothing_and_says_so(self):
        specs = [make_run(i, "disputas", "es", ALW) for i in range(6)]
        r = S.aggregate_agent_signals(make_export(specs))
        self.assertEqual(r["cells"], [])
        self.assertEqual(r["availability"], "below_privacy_floor")

    def test_unknown_agent_and_tool_and_locale_coarsened(self):
        def b(i, half):
            return make_run(i, "secret-agent-x", "fr-CA", [{"tool": "secret-tool"}] * 1,
                            outcome="escalated" if i % 2 else "resolved", closed_by="escalation" if i % 2 else "flow")
        r = S.aggregate_agent_signals(make_export(balanced(60, b)))
        blob = json.dumps(r)
        self.assertNotIn("secret", blob)
        self.assertNotIn("fr-CA", blob)
        self.assertTrue(any(c["dims"].get("agent") == "other" for c in r["cells"]))
        self.assertTrue(any(c["dims"].get("locale") == "other" for c in r["cells"]))

    def test_incomplete_event_pages_fail_closed(self):
        e = make_export([make_run(1, "disputas", "es", ALW)])
        e["events"]["run-00001"] = e["events"]["run-00001"][:1]
        with self.assertRaises(S.ExportContractError):
            S.aggregate_agent_signals(e)

    def test_k_masking_drops_complement_below_k(self):
        # 30 tool runs per half, only 3 errors => positives < k => no A4 cell
        def b(i, half):
            return make_run(i, "disputas", "es", [{"tool": "obtener_pqr", "status": "error" if i % 20 == 0 else "ok"}])
        r = S.aggregate_agent_signals(make_export(balanced(30, b)))
        self.assertFalse(_cells(r, "A4"))
        self.assertGreater(r["suppressed_cells"], 0)


class DescriptiveTests(unittest.TestCase):
    def test_latency_buckets_and_steps_by_agent(self):
        def b(i, half):
            return make_run(i, "disputas", "es", [{"tool": "obtener_pqr", "latency": 100}, {"tool": "obtener_pqr", "latency": 900}], fallback=0, decisions=2)
        r = S.aggregate_agent_signals(make_export(balanced(30, b)))
        d = [x for x in r["descriptive"] if x["agent"] == "disputas"][0]
        self.assertEqual(d["tool_latency_ms_p50_bucket"], "<=100")
        self.assertEqual(d["tool_latency_ms_p90_bucket"], "<=1000")
        self.assertEqual(d["steps_per_run_p50"], 4)
        self.assertEqual(d["runs"], 60)

    def test_descriptive_withheld_below_k(self):
        r = S.aggregate_agent_signals(make_export([make_run(i, "disputas", "es", ALW) for i in range(5)]))
        self.assertEqual(r["descriptive"], [])


if __name__ == "__main__":
    unittest.main()
