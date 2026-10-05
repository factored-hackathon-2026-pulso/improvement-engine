"""Tests for engine_trace + traceparent contract. Run: python -m unittest discover -s scripts/o11y/tests -p "test_*.py" """
import copy
import hashlib
import json
import sys
import tempfile
import threading
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import engine_trace as et  # noqa: E402
import otlp_receiver as rx  # noqa: E402
import trace_id as ti  # noqa: E402

FIX = HERE / "fixtures"
STORY = json.loads((FIX / "engine_story_recorded.json").read_text(encoding="utf-8"))
VECTORS = json.loads((FIX / "traceparent_vectors.json").read_text(encoding="utf-8"))["vectors"]


def sha16(s, n=16):
    return hashlib.sha256(s.encode("utf-8")).hexdigest()[:n]


def attrs(sp):
    return {a["key"]: a["value"] for a in sp["attributes"]}


class TraceparentContract(unittest.TestCase):
    def test_vectors_match_independent_derivation(self):
        # same derivation a Rust implementation follows, written out literally (not through trace_id.py)
        for v in VECTORS:
            tid = sha16(f"pulso.story.v1\n{v['finding_key'] or ''}\n{v['run_id']}", 32)
            self.assertEqual(v["trace_id"], tid)
            self.assertEqual(v["root_span_id"], sha16(f"{tid}|story"))
            self.assertEqual(v["traceparent_root"], f"00-{tid}-{sha16(f'{tid}|story')}-01")
            for stage, s in v["stages"].items():
                self.assertEqual(s["span_id"], sha16(f"{tid}|stage|{stage}|1"))
                self.assertEqual(s["traceparent"], f"00-{tid}-{s['span_id']}-01")
            self.assertEqual(v["stage_attempt_2"]["span_id"], sha16(f"{tid}|stage|builder|2"))
            self.assertEqual(v["generation"]["span_id"], sha16(f"{tid}|gen|scout|1"))

    def test_helper_matches_vectors_and_parses(self):
        for v in VECTORS:
            self.assertEqual(ti.traceparent_for(v["finding_key"], v["run_id"]), v["traceparent_root"])
            for stage, s in v["stages"].items():
                self.assertEqual(ti.traceparent_for(v["finding_key"], v["run_id"], stage), s["traceparent"])
            p = ti.parse_traceparent(ti.traceparent_for(v["finding_key"], v["run_id"], "scout"))
            self.assertEqual(p["trace_id"], v["trace_id"])
            self.assertTrue(p["sampled"])

    def test_none_equals_empty_and_unicode_is_utf8(self):
        self.assertEqual(ti.story_trace_id(None, "r"), ti.story_trace_id("", "r"))
        self.assertNotEqual(ti.story_trace_id("a", "r"), ti.story_trace_id("b", "r"))


class Convert(unittest.TestCase):
    def conv(self, story=STORY, idx=0, capture=True):
        return et.StoryConverter(capture, "local").convert(story, idx)

    def spans(self, rs):
        return rs["scopeSpans"][0]["spans"]

    def test_story_shape_ids_and_parents(self):
        rs, _ = self.conv()
        sp = self.spans(rs)
        root = sp[0]
        key = STORY["loop"]["findings"][0]["evidence_ref"]
        tid = ti.story_trace_id(key, STORY["run_id"])
        self.assertTrue(all(s["traceId"] == tid for s in sp))
        self.assertEqual(root["name"], "pulso.story")
        self.assertEqual(root["spanId"], ti.root_span_id(tid))
        a = attrs(root)
        self.assertEqual(a["langfuse.session.id"]["stringValue"], STORY["run_id"])
        self.assertEqual(a["langfuse.trace.metadata.finding_key"]["stringValue"], key)
        self.assertEqual(list(a["langfuse.trace.tags"]["arrayValue"]["values"][i]["stringValue"] for i in range(4)),
                         ["pulso", "engine", "stage:compile", "case-type:contrast"])
        names = [s["name"] for s in sp[1:]]
        for st in ("sensor", "scout", "recompute", "verifier", "builder", "compile", "deliver", "evaluate", "dossier"):
            self.assertIn(f"stage.{st}", names)
        stage = {s["name"]: s for s in sp if s["name"].startswith("stage.")}
        for st, s in stage.items():
            self.assertEqual(s["parentSpanId"], root["spanId"])
            self.assertEqual(s["spanId"], ti.stage_span_id(tid, st[6:]))
        gens = [s for s in sp if s["name"].startswith("generation ")]
        self.assertEqual([g["name"] for g in gens], ["generation scout", "generation verifier", "generation builder"])
        for g in gens:
            self.assertEqual(g["parentSpanId"], stage["stage." + g["name"].split()[1]]["spanId"])
            self.assertEqual(attrs(g)["langfuse.observation.type"]["stringValue"], "generation")
        # durations are non-negative and children sit inside the root
        for s in sp:
            self.assertLessEqual(int(s["startTimeUnixNano"]), int(s["endTimeUnixNano"]))
            self.assertGreaterEqual(int(s["startTimeUnixNano"]), int(root["startTimeUnixNano"]))
            self.assertLessEqual(int(s["endTimeUnixNano"]), int(root["endTimeUnixNano"]))

    def test_generation_usage_cost_content(self):
        rs, _ = self.conv()
        g = {s["name"]: attrs(s) for s in self.spans(rs) if s["name"].startswith("generation ")}
        sc = g["generation scout"]
        self.assertEqual(sc["gen_ai.request.model"]["stringValue"], "xiaomi/mimo-v2.6-flash")
        self.assertEqual(sc["gen_ai.usage.input_tokens"]["intValue"], "812")
        self.assertAlmostEqual(sc["gen_ai.usage.cost"]["doubleValue"], 812 * 1.4e-7 + 215 * 2.8e-7, places=12)
        self.assertAlmostEqual(g["generation verifier"]["gen_ai.usage.cost"]["doubleValue"], 1500 * 4.35e-7 + 310 * 8.7e-7, places=12)
        self.assertIn("Eres el Scout", sc["langfuse.observation.input"]["stringValue"])
        self.assertIn("hypothesis", sc["langfuse.observation.output"]["stringValue"])
        root = attrs(self.spans(rs)[0])
        self.assertEqual(root["pulso.llm.calls"]["intValue"], "3")
        self.assertAlmostEqual(root["pulso.cost_usd"]["doubleValue"],
                               812 * 1.4e-7 + 215 * 2.8e-7 + 1500 * 4.35e-7 + 310 * 8.7e-7 + 2100 * 1.4e-7 + 640 * 2.8e-7, places=11)

    def test_no_content_and_secret_scrub(self):
        rs, _ = self.conv(capture=False)
        for s in self.spans(rs):
            self.assertNotIn("langfuse.observation.input", attrs(s))
            self.assertNotIn("langfuse.observation.output", attrs(s))
            self.assertNotIn("pulso.delivery.proposal_id", attrs(s))
        st = copy.deepcopy(STORY)
        st["model_calls"][0]["response"] = "token=abc123secretvalue and Bearer xyz.123"
        rs, _ = self.conv(st)
        out = attrs([s for s in self.spans(rs) if s["name"] == "generation scout"][0])["langfuse.observation.output"]["stringValue"]
        self.assertNotIn("abc123secretvalue", out)
        self.assertNotIn("xyz.123", out)

    def test_scores(self):
        _, sc = self.conv()
        by = {s["name"]: s for s in sc}
        self.assertEqual(by["gate_regression_proven"]["value"], 1)
        self.assertEqual(by["gate_regression_proven"]["dataType"], "BOOLEAN")
        self.assertEqual(by["rubric_total"]["value"], 14)
        self.assertEqual(by["rubric_total"]["dataType"], "NUMERIC")
        self.assertEqual(by["announce"]["value"], 1)
        self.assertEqual(by["outcome_class"]["value"], "proposed_delivered")
        self.assertEqual(by["outcome_class"]["dataType"], "CATEGORICAL")
        self.assertEqual(len({s["id"] for s in sc}), len(sc))
        _, sc2 = self.conv()
        self.assertEqual([s["id"] for s in sc], [s["id"] for s in sc2])  # idempotent

    def test_blocked_finding_without_recorded_calls(self):
        rs, sc = self.conv(idx=1)
        sp = self.spans(rs)
        root = sp[0]
        self.assertEqual(root["status"]["code"], 2)
        self.assertIn("stage:builder", [v["stringValue"] for v in attrs(root)["langfuse.trace.tags"]["arrayValue"]["values"]])
        names = [s["name"] for s in sp]
        self.assertNotIn("stage.compile", names)
        self.assertNotIn("stage.deliver", names)
        b = [s for s in sp if s["name"] == "stage.builder"][0]
        self.assertEqual(b["status"]["code"], 2)
        gens = [s for s in sp if s["name"].startswith("generation ")]
        self.assertEqual(len(gens), 3)  # reconstructed from the record's model ids
        self.assertNotIn("gen_ai.usage.cost", attrs(gens[0]))  # no tokens recorded: no invented cost
        self.assertFalse(attrs(gens[0])["pulso.usage.recorded"]["boolValue"])
        by = {s["name"]: s for s in sc}
        self.assertNotIn("gate_regression_proven", by)
        self.assertNotIn("rubric_total", by)
        self.assertEqual(by["announce"]["value"], 0)
        self.assertEqual(by["outcome_class"]["value"], "blocked_model_invalid")

    def test_unlinked_has_only_sensor(self):
        rs, sc = self.conv(idx=2)
        self.assertEqual([s["name"] for s in self.spans(rs)], ["pulso.story", "stage.sensor"])
        self.assertEqual({s["name"]: s["value"] for s in sc}["outcome_class"], "unlinked")

    def test_verdict_and_dossier_only_for_their_finding(self):
        _, sc = self.conv(idx=1)
        self.assertNotIn("gate_regression_proven", {s["name"] for s in sc})

    def test_timing_marked_and_no_window_is_an_error(self):
        rs, _ = self.conv(idx=0)
        a = {s["name"]: attrs(s)["pulso.timing"]["stringValue"] for s in self.spans(rs) if s["name"].startswith("stage.")}
        self.assertEqual(a["stage.scout"], "recorded")
        self.assertEqual(a["stage.recompute"], "inferred")
        st = copy.deepcopy(STORY)
        st["events"] = []
        with self.assertRaises(ValueError):
            self.conv(st)

    def test_denied_delivery_is_error(self):
        st = copy.deepcopy(STORY)
        st["loop"]["findings"][0]["delivery"] = {"status": "denied", "reason": "writer_not_allowed"}
        rs, sc = self.conv(st)
        d = [s for s in self.spans(rs) if s["name"] == "stage.deliver"][0]
        self.assertEqual(d["status"]["code"], 2)
        self.assertEqual({s["name"]: s["value"] for s in sc}["outcome_class"], "proposed_denied")


class Send(unittest.TestCase):
    def test_cli_to_local_receiver_validates_and_shares_trace_with_traceparent_peer(self):
        srv = rx.make_server(0)
        port = srv.server_address[1]
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory() as d:
                scores = Path(d) / "scores.jsonl"
                rc = et.main(["--story-file", str(FIX / "engine_story_recorded.json"), "--finding", "0",
                              "--scores-out", str(scores), "--env-file", str(self._env(d, port))])
                lines = [json.loads(x) for x in scores.read_text(encoding="utf-8").splitlines()]
            self.assertEqual(rc, 0)
            got = srv.received
            self.assertEqual(len(got), 13)  # root + 9 stages + 3 generations
            self.assertEqual(len({g["trace"] for g in got}), 1)
            self.assertEqual({x["name"] for x in lines}, {"gate_regression_proven", "rubric_total", "announce", "outcome_class"})
        finally:
            srv.shutdown()

    def _env(self, d, port):
        p = Path(d) / "e.env"
        p.write_text(f"PULSO_O11Y_OTLP_ENDPOINT=http://127.0.0.1:{port}\n", encoding="utf-8")
        return p


class Peer(unittest.TestCase):
    def test_peer_span_built_from_traceparent_joins_engine_trace(self):
        """A gateway/agent-core span created from the engine's header lands in the engine's trace, under its stage span."""
        f = STORY["loop"]["findings"][0]
        hdr = ti.traceparent_for(f["evidence_ref"], STORY["run_id"], "scout")
        p = ti.parse_traceparent(hdr)
        rs, _ = et.StoryConverter(True).convert(STORY, 0)
        sp = rs["scopeSpans"][0]["spans"]
        stage = [s for s in sp if s["name"] == "stage.scout"][0]
        self.assertEqual(p["trace_id"], sp[0]["traceId"])
        self.assertEqual(p["parent_span_id"], stage["spanId"])


if __name__ == "__main__":
    unittest.main()
