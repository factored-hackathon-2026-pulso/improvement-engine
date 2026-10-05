"""Tests for trace_id and otlp_forwarder against a MOCK Langfuse (stdlib server checking Basic auth, headers, shape)."""
import base64
import json
import sys
import threading
import unittest
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import otlp_forwarder as fw  # noqa: E402
import otlp_receiver as rx  # noqa: E402
import runtrace_bridge as rb  # noqa: E402
import trace_id as ti  # noqa: E402
from test_runtrace_bridge import EVENTS, RUN, FakeCore, RID  # noqa: E402

PK, SK = "pk-lf-mock", "sk-lf-mock"


class MockLangfuse:
    def __init__(self, fail_first=0):
        self.got, self.bad, self.fail_first, self.n = [], [], fail_first, 0
        outer = self

        class H(BaseHTTPRequestHandler):
            def do_POST(self):  # noqa: N802
                body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                outer.n += 1
                if outer.n <= outer.fail_first:
                    self.send_response(503)
                    self.end_headers()
                    return
                want = "Basic " + base64.b64encode(f"{PK}:{SK}".encode()).decode()
                if self.headers.get("Authorization") != want:
                    self.send_response(401)
                    self.end_headers()
                    return
                try:
                    doc = json.loads(body)
                    if self.path == "/api/public/otel/v1/traces":
                        assert self.headers.get("x-langfuse-ingestion-version") == "4"
                        rx.validate(doc)
                except Exception as e:  # noqa: BLE001
                    outer.bad.append(str(e))
                    self.send_response(400)
                    self.end_headers()
                    return
                outer.got.append((self.path, doc))
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"{}")

            def log_message(self, *a):
                pass

        self.srv = HTTPServer(("127.0.0.1", 0), H)
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}"
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()

    def close(self):
        self.srv.shutdown()
        self.srv.server_close()


class TraceIdTests(unittest.TestCase):
    def test_deterministic_and_distinct(self):
        a = ti.story_trace_id("fk", "r1")
        self.assertEqual(a, ti.story_trace_id("fk", "r1"))
        self.assertRegex(a, r"^[0-9a-f]{32}$")
        self.assertNotEqual(a, ti.story_trace_id("fk", "r2"))
        self.assertNotEqual(a, ti.story_trace_id("fk2", "r1"))

    def test_traceparent_roundtrip(self):
        t = ti.story_trace_id("fk", "r1")
        h = ti.build_traceparent(t, ti.span_id(t, "scout", "1"))
        p = ti.parse_traceparent(h)
        self.assertEqual((p["trace_id"], p["sampled"]), (t, True))
        self.assertFalse(ti.parse_traceparent(ti.build_traceparent(t, "1" * 16, sampled=False))["sampled"])

    def test_invalid(self):
        for bad in ("", "00-xyz", "00-" + "0" * 32 + "-" + "1" * 16 + "-01", "01-" + "a" * 32 + "-" + "1" * 16 + "-01"):
            with self.assertRaises(ValueError):
                ti.parse_traceparent(bad)
        with self.assertRaises(ValueError):
            ti.build_traceparent("a" * 31, "1" * 16)

    def test_bridge_uses_it(self):
        rs = rb.Converter(finding_key="fk").run_to_trace(RUN, EVENTS)
        self.assertEqual(rs["scopeSpans"][0]["spans"][0]["traceId"], ti.story_trace_id("fk", RID))
        self.assertEqual(rb.trace_id_for(RID), ti.story_trace_id(None, RID))


class ForwarderTests(unittest.TestCase):
    def setUp(self):
        self.lf = MockLangfuse()
        self.env = {"LANGFUSE_BASE_URL": self.lf.url, "LANGFUSE_PUBLIC_KEY": PK, "LANGFUSE_SECRET_KEY": SK}

    def tearDown(self):
        self.lf.close()

    def test_policy(self):
        with self.assertRaises(ValueError):
            fw.Forwarder({**self.env, "LANGFUSE_BASE_URL": "https://cloud.langfuse.com"}, False)
        with self.assertRaises(ValueError):
            fw.Forwarder({**self.env, "LANGFUSE_BASE_URL": "http://cloud.langfuse.com"}, True)
        with self.assertRaises(ValueError):
            fw.Forwarder({"LANGFUSE_BASE_URL": self.lf.url}, False)

    def test_bridge_to_forwarder_to_mock_langfuse(self):
        f = fw.Forwarder(self.env, False, sleep=lambda s: None)
        srv = fw.make_server(f, 0)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        port = srv.server_address[1]
        try:
            send, api = rb.build_endpoints("forwarder", {"PULSO_O11Y_FORWARDER_ENDPOINT": f"http://127.0.0.1:{port}"}, False)
            core = FakeCore([RUN], {RID: EVENTS}, [])
            import tempfile
            with tempfile.TemporaryDirectory() as d:
                conv = rb.Converter(capture_content=True)
                r = rb.Bridge(fetch=core, send=send, score=api.score, converter=conv,
                              state_path=Path(d) / "s.json").poll_once()
            self.assertEqual((r["run_traces"], r["scores"]), (1, 2))
            while f.drain_once():
                pass
        finally:
            srv.shutdown()
            srv.server_close()
        paths = [p for p, _ in self.lf.got]
        self.assertEqual(paths.count("/api/public/otel/v1/traces"), 1)
        self.assertEqual(paths.count("/api/public/scores"), 2)
        self.assertEqual(self.lf.bad, [])
        blob = json.dumps(self.lf.got)
        self.assertIn("langfuse.observation.input", blob)  # content present
        self.assertNotIn("abc.def.ghi", blob)  # secrets masked
        self.assertNotIn(SK, blob)

    def test_masks_secrets_in_forwarded_body(self):
        f = fw.Forwarder(self.env, False, sleep=lambda s: None)
        body = {"resourceSpans": [{"scopeSpans": [{"spans": [{"traceId": "a" * 32, "spanId": "b" * 16, "name": "x",
                "startTimeUnixNano": "1", "endTimeUnixNano": "2",
                "attributes": [{"key": "k", "value": {"stringValue": "Authorization: Bearer tok123 api_key=zzz"}}]}]}]}]}
        f.enqueue("traces", body)
        f.drain_once()
        sent = json.dumps(self.lf.got)
        self.assertNotIn("tok123", sent)
        self.assertNotIn("zzz", sent)

    def test_retry_with_backoff_on_503(self):
        self.lf.fail_first = 2
        sleeps = []
        f = fw.Forwarder(self.env, False, sleep=sleeps.append)
        f.post("scores", {"id": "1"})
        self.assertEqual(f.stats["retries"], 2)
        self.assertEqual(len(self.lf.got), 1)
        self.assertTrue(any(s >= 2 for s in sleeps))  # exponential backoff reached 2s

    def test_auth_error_not_retried_and_secret_not_in_error(self):
        f = fw.Forwarder({**self.env, "LANGFUSE_SECRET_KEY": "wrong"}, False, sleep=lambda s: None)
        with self.assertRaises(RuntimeError) as c:
            f.post("scores", {"id": "1"})
        self.assertEqual(f.stats["retries"], 0)
        self.assertNotIn("wrong", str(c.exception))

    def test_rate_limits_encoded(self):
        self.assertGreaterEqual(fw.TRACE_INTERVAL * 1000, 60 / 1000 * 1000 - 1e-9)
        self.assertGreaterEqual(fw.SCORE_INTERVAL, 60 / 30)

    def test_listener_is_loopback(self):
        f = fw.Forwarder(self.env, False)
        srv = fw.make_server(f, 0)
        try:
            self.assertEqual(srv.server_address[0], "127.0.0.1")
        finally:
            srv.server_close()


class ContentDefaultTests(unittest.TestCase):
    def test_defaults(self):
        self.assertTrue(rb.resolve_capture("langfuse", False, False, None))
        self.assertFalse(rb.resolve_capture("langfuse", False, True, None))
        self.assertFalse(rb.resolve_capture("langfuse", True, True, "1"))
        self.assertFalse(rb.resolve_capture("langfuse", False, False, "0"))
        self.assertFalse(rb.resolve_capture("local", False, False, None))
        self.assertTrue(rb.resolve_capture("forwarder", False, False, "1"))


if __name__ == "__main__":
    unittest.main()
