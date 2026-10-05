"""Offline tests of the probe source (PRB1) on RECORDED battery results (agent-core-assets/eval-battery/results).
Run: python -m unittest discover -s scripts/battery/tests"""
import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import probe_cells as pc  # noqa: E402
import schedule_probes as sp  # noqa: E402

RES = HERE.parents[2] / "agent-core-assets" / "eval-battery" / "results"
BASE = json.loads((RES / "ev2-live-base.json").read_text(encoding="utf-8"))
CAND = json.loads((RES / "ev2-live-candidate-same-release.json").read_text(encoding="utf-8"))


class Sink:
    def __init__(self):
        self.sent = []

    def send(self, req):
        self.sent.append(req)


def later(res, stamp):
    r = copy.deepcopy(res)
    r["finished_at"] = stamp
    return r


def fresh_state():
    return pc.load_state(Path(tempfile.mkdtemp()) / "none.json")


def synth(passes_by_id, agent="a", family="f"):
    """A tiny result: id -> list of rep pass flags (None = infra error)."""
    return {"started_at": "t", "finished_at": "t", "scenarios": [
        {"id": i, "agent": agent, "family": family, "passed": all(p for p in reps if p is not None),
         "reps": [{"passed": bool(p), "error": "turn http 502" if p is None else None} for p in reps]} for i, reps in passes_by_id.items()]}


class FlakeRule(unittest.TestCase):
    def test_two_of_three_fails_one_of_three_is_flaky(self):
        v = pc.scenario_verdicts(synth({"a": [False, False, True], "b": [False, True, True], "c": [True] * 3}))
        self.assertEqual([v[i]["status"] for i in "abc"], ["fail", "pass", "pass"])
        self.assertTrue(v["b"]["flaky"] and not v["c"]["flaky"])

    def test_infra_errors_are_not_failures(self):
        v = pc.scenario_verdicts(synth({"a": [None, None, False], "b": [None, False, False]}))
        self.assertEqual(v["a"]["status"], "inconclusive")
        self.assertEqual(v["b"]["status"], "fail")

    def test_behavioural_error_is_a_failure(self):
        r = synth({"a": [True, True, True]})
        r["scenarios"][0]["reps"][2] = {"passed": False, "error": "confirm step without a pending confirmation"}
        self.assertEqual(pc.scenario_verdicts(r)["a"]["failed_reps"], 1)
        self.assertTrue(pc.scenario_verdicts(r)["a"]["flaky"])

    def test_consecutive_needed(self):
        now = pc.scenario_verdicts(synth({"a": [False] * 3}))
        self.assertEqual(pc.confirmed_ids(now, None), [])
        self.assertEqual(pc.confirmed_ids(now, now), ["a"])
        ok = pc.scenario_verdicts(synth({"a": [True] * 3}))
        self.assertEqual(pc.confirmed_ids(now, ok), [])


class RecordedResults(unittest.TestCase):
    def test_real_defects_are_cells(self):
        v = pc.scenario_verdicts(BASE)
        fails = sorted(i for i, x in v.items() if x["status"] == "fail")
        self.assertEqual(fails, ["atk-consultas-language-switch", "atk-consultas-vague-customer", "atk-disputas-language-switch"])
        # consultas vague_customer (radicado slot accepts any text): 2 of 3 repetitions fail -> a run-level failure
        vc = v["atk-consultas-vague-customer"]
        self.assertEqual((vc["failed_reps"], vc["valid"]), (2, 3))

    def test_first_run_is_candidate_second_corroborated(self):
        v = pc.scenario_verdicts(BASE)
        first = pc.findings(v, None)
        self.assertTrue(first and all(s["status"] in ("candidate", "refuted") for s in first))
        second = pc.findings(v, v)
        corro = {(s["metric"], s["dims"]["agent"], s["dims"].get("scenario_family")) for s in second if s["status"] == "corroborated"}
        self.assertIn(("P1", "consultas", "attacker:language_switch"), corro)
        self.assertIn(("P1", "consultas", "attacker:vague_customer"), corro)
        self.assertIn(("P1", "disputas", "attacker:language_switch"), corro)
        self.assertNotIn(("P1", "disputas", "amount_boundary"), corro)

    def test_cell_table_schema_and_no_ids(self):
        v = pc.scenario_verdicts(BASE)
        rows = pc.cell_table(v, v)
        self.assertTrue(rows)
        for r in rows:
            self.assertEqual(set(r) - {"evidence_class"}, {"metric", "dims", "half", "numerator", "denominator"})
            self.assertEqual(r["evidence_class"], "probe_synthetic")
            self.assertLessEqual(r["numerator"], r["denominator"])
            self.assertTrue(set(r["dims"]) <= {"agent", "scenario_family"})
        strict = [json.loads(x) for x in pc.to_ndjson(rows, strict_schema=True).splitlines()]
        self.assertTrue(all("evidence_class" not in x for x in strict))
        self.assertNotIn("atk-", pc.to_ndjson(rows))

    def test_level_and_contrast_signals_normalise_alike(self):
        lvl = pc.findings(pc.scenario_verdicts(BASE), None)[0]
        contrast = {"metric": "M1", "dims": {"channel": "Phone"}, "status": "corroborated", "direction": "up",
                    "discovery": {"rate": 0.3, "baseline_rate": 0.2, "diff": 0.1}}
        a, b = pc.normalise_signal(lvl), pc.normalise_signal(contrast)
        self.assertEqual((a["kind"], a["baseline_kind"], a["evidence_class"]), ("level_risk", "threshold", "probe_synthetic"))
        self.assertEqual((b["kind"], b["baseline_kind"], b["evidence_class"]), ("contrast", "rest_of_metric", "real"))

    def test_never_mixed_with_real(self):
        probe = pc.findings(pc.scenario_verdicts(BASE), None)
        real = [{"metric": "M1", "status": "corroborated", "dims": {}}]
        out = pc.combine_reports(real, probe)
        self.assertEqual(out["real"]["counts"]["total"], 1)
        self.assertEqual(out["probe_synthetic"]["counts"]["total"], len(probe))
        with self.assertRaises(ValueError):
            pc.combine_reports(probe, probe)
        with self.assertRaises(ValueError):
            pc.combine_reports(real, real)


class Scheduler(unittest.TestCase):
    def test_trigger_flow_and_suppression(self):
        sink, st = Sink(), fresh_state()
        r1 = sp.process(later(BASE, "2026-10-05T01:00:00+00:00"), st, sink, now=1000.0)
        self.assertEqual((r1["action"], sink.sent), ("none", []))          # first run: candidates only
        r2 = sp.process(later(BASE, "2026-10-05T02:00:00+00:00"), st, sink, now=4600.0)
        self.assertEqual(r2["action"], "triggered")
        req = sink.sent[0]
        self.assertEqual((req["schema"], req["kind"], req["event"]["type"]), ("pulso.trigger.v1", "scheduled", "schedule.tick"))
        self.assertTrue(req["trigger_key"].startswith("sha256:"))
        self.assertEqual(req["evidence_class"], "probe_synthetic")
        cells = req["probe_cells"]
        self.assertTrue(cells and all(c["label"] == "probe" and c["outcome"] == "fail" for c in cells))
        self.assertNotIn("scenario_ids", json.dumps(req))
        self.assertNotIn("atk-", json.dumps(req))
        r3 = sp.process(later(BASE, "2026-10-05T03:00:00+00:00"), st, sink, now=8200.0)
        self.assertEqual((r3["action"], len(sink.sent)), ("suppressed_unchanged_set", 1))
        r4 = sp.process(later(BASE, "2026-10-05T03:00:00+00:00"), st, sink, now=8300.0)   # same file again: not a new run
        self.assertEqual((r4["action"], st["runs"]), ("skip_already_consumed", 3))

    def test_changed_set_retriggers_and_recovery_resets(self):
        sink, st = Sink(), fresh_state()
        sp.process(later(BASE, "t1"), st, sink)
        sp.process(later(BASE, "t2"), st, sink)
        worse = copy.deepcopy(BASE)
        for s in worse["scenarios"]:
            if s["id"] == "atk-consultas-angry-customer":
                for r in s["reps"]:
                    r["passed"] = False
        worse["finished_at"] = "t3"
        sp.process(worse, st, sink)            # new failing scenario: candidate only (first time)
        self.assertEqual(len(sink.sent), 1)
        r = sp.process(later(worse, "t4"), st, sink)       # confirmed: the set changed -> second trigger
        self.assertEqual((r["action"], len(sink.sent)), ("triggered", 2))
        clean = synth({s["id"]: [True] * 3 for s in BASE["scenarios"]})
        for i, s in enumerate(clean["scenarios"]):
            s["agent"], s["family"] = BASE["scenarios"][i]["agent"], BASE["scenarios"][i]["family"]
        clean["finished_at"] = "t5"
        sp.process(clean, st, sink)
        self.assertIsNone(st["triggered_digest"])
        sp.process(later(BASE, "t6"), st, sink)
        r = sp.process(later(BASE, "t7"), st, sink)  # recurrence after recovery triggers again
        self.assertEqual((r["action"], len(sink.sent)), ("triggered", 3))

    def test_flaky_run_never_triggers(self):
        sink, st = Sink(), fresh_state()
        for i in range(3):
            r = sp.process(later(CAND, f"c{i}"), st, sink)
        self.assertTrue(r["flaky"])
        self.assertNotIn("amt-usd-500-es", r["confirmed_scenarios"])

    def test_state_persists(self):
        d = Path(tempfile.mkdtemp()) / "s.json"
        st = pc.load_state(d)
        sp.process(later(BASE, "p1"), st, Sink())
        pc.save_state(d, st)
        r = sp.process(later(BASE, "p2"), pc.load_state(d), Sink())
        self.assertEqual(r["action"], "triggered")


if __name__ == "__main__":
    unittest.main()
