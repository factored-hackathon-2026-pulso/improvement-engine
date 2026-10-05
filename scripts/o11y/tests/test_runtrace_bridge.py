"""Tests for runtrace_bridge. Run: python -m unittest discover -s scripts/o11y/tests -p "test_*.py" -t scripts/o11y"""
import json
import sys
import tempfile
import threading
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import otlp_receiver as rx  # noqa: E402
import runtrace_bridge as rb  # noqa: E402

FIXTURE = json.loads((HERE.parents[0] / "triggers" / "fixtures" / "export_recorded.json").read_text(encoding="utf-8"))
RID = "run-1"


def ev(seq, typ, payload, ts="2026-10-04T10:00:%02dZ"):
    return {"event_id": f"e{seq}", "run_id": RID, "seq": seq, "type": typ, "payload": payload, "ts": ts % seq}


RUN = {"run_id": RID, "cursor": 5, "release": "rel-1", "agent": {"id": "disputas", "version": "1.2.0"},
       "principal_type": "customer", "mode": "chat", "locale": "es", "status": "closed", "outcome": "escalated",
       "created_at": "2026-10-04T10:00:00Z", "closed_at": "2026-10-04T10:00:30Z"}
EVENTS = [
    ev(1, "run_started", {"agent": {"id": "disputas"}}),
    ev(2, "command_emitted", {"command": "start_flow", "flow": "disputa", "source": "understand",
                              "above_threshold": {"intent": True}, "decision_id": "dec-secret-1"}),
    ev(3, "node_entered", {"flow": {"id": "disputa", "version": "1.0.0"}, "node_id": "n1", "node_type": "agent",
                           "resume_kind": "none"}),
    ev(4, "decision_made", {"decision_id": "dec-secret-1", "model": {"id": "understand", "version": "1"},
                            "provider_used": "gateway", "model_version": "xiaomi/mimo-v2.6-flash", "fallback_depth": 1,
                            "value": {"intent": "my card number 4111"}, "p_cal": {"intent": 0.9}, "p_raw": {},
                            "above_threshold": {"intent": True}, "latency_ms": 500, "tokens": 120, "cost_usd": "0.00002",
                            "locale": "es"}),
    ev(5, "tool_called", {"node_id": "n1", "tool": {"id": "lookup", "version": "1"}, "call_id": "call-secret",
                          "status": "ok", "args": {"q": "Bearer abc.def.ghi pan 4111"}, "result": {"k": 1},
                          "latency_ms": 40, "attempt": 1}),
    ev(6, "response_emitted", {"kind": "generated", "validator": {"ok": False, "failures": ["numbers"], "regenerations": 1},
                               "fallback_used": True, "claims": [],
                               "llm": {"calls": 2, "latency_ms": 900, "tokens_in": 1000, "tokens_out": 200,
                                       "cost_usd": "0.0", "cost_known": True, "models": ["xiaomi/mimo-v2.6-pro"]}}),
    ev(7, "escalated", {"reason_code": "validation_failed", "target_queue": "q1", "priority": "p2",
                        "handoff_ref": "handoff-secret"}),
    ev(8, "step_up_requested", {"node_id": "n1", "required_level": "L2", "attempt": 1}),
    ev(9, "run_closed", {"outcome": "escalated", "closed_by": "escalation"}),
]


def attrs(span):
    out = {}
    for a in span["attributes"]:
        out[a["key"]] = next(iter(a["value"].values()))
    return out


class ConverterTests(unittest.TestCase):
    def setUp(self):
        self.rs = rb.Converter().run_to_trace(RUN, EVENTS)
        self.spans = self.rs["scopeSpans"][0]["spans"]
        self.by = {s["name"]: s for s in self.spans}

    def test_one_trace_one_span_per_event_plus_root(self):
        self.assertEqual(len({s["traceId"] for s in self.spans}), 1)
        self.assertEqual(len(self.spans), 1 + len(EVENTS) - 2)  # run_started/closed live on the root
        self.assertEqual(self.spans[0]["traceId"], rb.trace_id_for(RID))
        self.assertTrue(all(s["parentSpanId"] == self.spans[0]["spanId"] for s in self.spans[1:]))

    def test_receiver_accepts_shape(self):
        self.assertEqual(len(rx.validate({"resourceSpans": [self.rs]})), len(self.spans))

    def test_root_attributes(self):
        a = attrs(self.spans[0])
        self.assertEqual(a["pulso.agent"], "disputas")
        self.assertEqual(a["pulso.locale"], "es")
        self.assertEqual(a["pulso.outcome"], "escalated")
        self.assertTrue(a["pulso.closed_early"])
        self.assertEqual(a["langfuse.session.id"], RID)
        self.assertEqual(a["langfuse.observation.type"], "agent")
        self.assertIn("release:rel-1", [v["stringValue"] for v in a["langfuse.trace.tags"]["values"]])
        self.assertEqual(self.spans[0]["status"]["code"], 2)

    def test_trace_attrs_on_every_span(self):
        for s in self.spans:
            self.assertEqual(attrs(s)["langfuse.release"], "rel-1")

    def test_decision_reason_fields(self):
        a = attrs(self.by["decision understand@1"])
        self.assertEqual(a["langfuse.observation.type"], "generation")
        self.assertEqual(a["pulso.reason.fallback_depth"], "1")
        self.assertTrue(a["pulso.reason.fallback"])
        self.assertEqual(a["gen_ai.request.model"], "xiaomi/mimo-v2.6-flash")
        self.assertEqual(a["gen_ai.usage.total_tokens"], "120")

    def test_response_cost_from_price_table(self):
        a = attrs(self.by["response generated"])
        self.assertAlmostEqual(a["gen_ai.usage.cost"], 1000 * 4.35e-7 + 200 * 8.7e-7)
        self.assertEqual(a["pulso.cost.source"], "price_table")
        self.assertEqual(a["gen_ai.usage.input_tokens"], "1000")
        self.assertFalse(a["pulso.reason.validator_ok"])
        self.assertTrue(a["pulso.reason.fallback_used"])
        root = attrs(self.spans[0])
        self.assertEqual(root["gen_ai.usage.output_tokens"], "200")

    def test_handoff_tool_interrupt_types(self):
        self.assertEqual(attrs(self.by["handoff escalated"])["pulso.reason.reason_code"], "validation_failed")
        self.assertEqual(attrs(self.by["tool lookup@1"])["gen_ai.tool.name"], "lookup@1")
        self.assertEqual(attrs(self.by["step_up_requested"])["pulso.step.type"], "interrupt")
        self.assertEqual(attrs(self.by["node n1"])["pulso.step.type"], "agent")

    def test_no_content_and_no_instance_ids_by_default(self):
        blob = json.dumps(self.rs)
        for leak in ("4111", "Bearer", "dec-secret-1", "call-secret", "handoff-secret", "langfuse.observation.input",
                     "langfuse.observation.output"):
            self.assertNotIn(leak, blob)

    def test_content_opt_in_is_scrubbed(self):
        blob = json.dumps(rb.Converter(capture_content=True).run_to_trace(RUN, EVENTS))
        self.assertIn("langfuse.observation.input", blob)
        self.assertNotIn("abc.def.ghi", blob)
        self.assertNotIn("handoff-secret", blob)

    def test_deterministic(self):
        self.assertEqual(json.dumps(rb.Converter().run_to_trace(RUN, EVENTS)), json.dumps(self.rs))

    def test_scores(self):
        sc = rb.Converter().scores_for(RUN, self.rs)
        self.assertEqual({s["name"]: s["value"] for s in sc}, {"run_completed": 0, "closed_early": 1})
        self.assertEqual(sc[0]["traceId"], rb.trace_id_for(RID))


class FakeCore:
    def __init__(self, runs, events, registry):
        self.runs, self.events, self.registry = runs, events, registry

    def __call__(self, path, params):
        after, limit = params["after"], params["limit"]
        if path == "/v1/export/runs":
            items = [r for r in self.runs if r["cursor"] > after][:limit]
            return {"items": items, "next_after": items[-1]["cursor"] if items else after}
        if path == "/v1/export/registry-events":
            items = self.registry[after:after + limit]
            return {"items": items, "next_after": after + len(items)}
        rid = path.split("/")[4]
        items = [e for e in self.events.get(rid, []) if e["seq"] > after][:limit]
        return {"items": items, "next_after": items[-1]["seq"] if items else after}


class BridgeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.state = Path(self.tmp.name) / "s.json"
        self.sent, self.scores = [], []
        self.runs = [dict(RUN, run_id="a", cursor=1), dict(RUN, run_id="b", cursor=2, closed_at=None, status="open"),
                     dict(RUN, run_id="c", cursor=3)]
        self.events = {r["run_id"]: [dict(e, run_id=r["run_id"]) for e in EVENTS] for r in self.runs}
        self.core = FakeCore(self.runs, self.events, FIXTURE["registry"]["items"])

    def tearDown(self):
        self.tmp.cleanup()

    def bridge(self, **kw):
        return rb.Bridge(fetch=self.core, send=self.sent.append, state_path=self.state, page_limit=2,
                         score=self.scores.append, **kw)

    def test_only_closed_runs_and_open_run_is_retried(self):
        r = self.bridge().poll_once()
        self.assertEqual(r["run_traces"], 2)
        self.assertEqual(r["registry_traces"], len(FIXTURE["registry"]["items"]))
        self.assertEqual(r["scores"], 4)
        self.assertEqual(json.loads(self.state.read_text())["runs_cursor"], 1)  # held before the open run
        self.runs[1].update(closed_at="2026-10-04T10:01:00Z", status="closed", outcome="completed")
        r2 = self.bridge().poll_once()
        self.assertEqual(r2["run_traces"], 1)  # only b; a and c deduped
        self.assertEqual(r2["registry_traces"], 0)
        self.assertEqual(json.loads(self.state.read_text())["runs_cursor"], 3)

    def test_failure_keeps_cursor(self):
        def boom(_):
            raise RuntimeError("down")
        b = rb.Bridge(fetch=self.core, send=boom, state_path=self.state)
        with self.assertRaises(RuntimeError):
            b.poll_once()
        self.assertEqual(json.loads(self.state.read_text())["runs_cursor"], 0)
        self.assertEqual(self.bridge().poll_once()["run_traces"], 2)

    def test_score_failure_is_queued_not_lost(self):
        calls = []

        def flaky(sc):
            calls.append(sc)
            if len(calls) == 1:
                raise RuntimeError("429")
        b = rb.Bridge(fetch=self.core, send=self.sent.append, state_path=self.state, score=flaky)
        with self.assertRaises(RuntimeError):
            b.poll_once()
        n = rb.Bridge(fetch=self.core, send=self.sent.append, state_path=self.state, score=self.scores.append).poll_once()
        self.assertEqual(n["scores"], 4)  # nothing lost, nothing re-traced
        self.assertEqual(n["run_traces"], 0)


class PolicyTests(unittest.TestCase):
    LF = {"LANGFUSE_BASE_URL": "https://cloud.langfuse.com", "LANGFUSE_PUBLIC_KEY": "pk-test", "LANGFUSE_SECRET_KEY": "sk-test"}

    def test_local_default_and_loopback_only(self):
        self.assertEqual(rb.build_sender("local", {}, False).url, "http://127.0.0.1:4318/v1/traces")
        with self.assertRaises(ValueError):
            rb.build_sender("local", {"PULSO_O11Y_OTLP_ENDPOINT": "http://example.com:4318"}, True)

    def test_external_needs_flag_and_https(self):
        with self.assertRaises(ValueError):
            rb.build_sender("langfuse", self.LF, False)
        with self.assertRaises(ValueError):
            rb.build_sender("langfuse", {**self.LF, "LANGFUSE_BASE_URL": "http://cloud.langfuse.com"}, True)
        s = rb.build_sender("langfuse", self.LF, True)
        self.assertEqual(s.url, "https://cloud.langfuse.com/api/public/otel/v1/traces")
        self.assertTrue(s.headers["Authorization"].startswith("Basic "))
        self.assertEqual(s.headers["x-langfuse-ingestion-version"], "4")

    def test_missing_keys(self):
        with self.assertRaises(ValueError):
            rb.build_sender("langfuse", {"LANGFUSE_BASE_URL": "https://x.example"}, True)

    def test_error_text_never_has_secret(self):
        self.assertNotIn("sk-test", rb.redact("failed sk-test", ["sk-test"]))


class EndToEndReceiverTests(unittest.TestCase):
    def test_bridge_to_local_receiver(self):
        srv = rx.make_server(0)
        port = srv.server_address[1]
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory() as d:
                send = rb.build_sender("local", {"PULSO_O11Y_OTLP_ENDPOINT": f"http://127.0.0.1:{port}"}, False)
                core = FakeCore([RUN], {RID: EVENTS}, [])
                r = rb.Bridge(fetch=core, send=send, state_path=Path(d) / "s.json").poll_once()
            self.assertEqual(r["run_traces"], 1)
            self.assertEqual(len(srv.received), 8)
        finally:
            srv.shutdown()


if __name__ == "__main__":
    unittest.main()
