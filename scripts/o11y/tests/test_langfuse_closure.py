"""Offline tests of the Langfuse closure against the MOCK Langfuse: protobuf decode, auth, idempotent models, read-back verification
(fixtures with and without the three sources), the real forwarder + engine_trace + protobuf passthrough end to end, and a canary
(no credential value in anything printed)."""
import contextlib
import io
import json
import os
import sys
import threading
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import engine_trace as et  # noqa: E402
import langfuse_verify as lv  # noqa: E402
import mock_langfuse as ml  # noqa: E402
import otlp_decode as od  # noqa: E402
import otlp_forwarder as fw  # noqa: E402

PK, SK = "pk-lf-canary-0123456789", "sk-lf-canary-9876543210"
FIX = HERE / "fixtures" / "engine_story_recorded.json"
GW = {"service.name": "llm-gateway"}
AC = {"service.name": "agentcore"}


def sp(trace, sid, parent, name, attrs=None, res=None):
    return {"trace_id": trace, "span_id": sid, "parent_id": parent, "name": name, "attrs": attrs or {}, "resource": res or {}}


def gen_attrs(model, content=True, tin=100, tout=50):
    a = {"langfuse.observation.type": "generation", "langfuse.observation.model.name": model,
         "langfuse.observation.usage_details": json.dumps({"input": tin, "output": tout})}
    if content:
        a.update({"langfuse.observation.input": "prompt", "langfuse.observation.output": "answer"})
    return a


def obs_fixture(sources=("engine", "gateway", "agent-core"), content=True, cost=True):
    t = "a" * 32
    rows = [{"id": "1", "traceId": t, "name": "pulso.story", "type": "SPAN", "metadata": {}}]
    if "engine" in sources:
        rows.append({"id": "2", "traceId": t, "name": "generation scout", "type": "GENERATION", "model": "xiaomi/mimo-v2.6-flash",
                     "input": "i" if content else None, "output": "o" if content else None, "calculatedTotalCost": 1e-5 if cost else 0})
    if "gateway" in sources:
        rows.append({"id": "3", "traceId": t, "name": "chat xiaomi/mimo-v2.6-flash", "type": "GENERATION", "model": "xiaomi/mimo-v2.6-flash",
                     "input": "i" if content else None, "output": "o" if content else None, "calculatedTotalCost": 1e-5 if cost else 0})
    if "agent-core" in sources:
        rows.append({"id": "4", "traceId": t, "name": "agentcore.api.request", "type": "SPAN", "metadata": {}})
    return {t: rows}, t


SCORES = [{"name": "rubric_total", "traceId": "a" * 32}, {"name": "announce", "traceId": "a" * 32}]


class Summarize(unittest.TestCase):
    def test_all_three_sources_pass(self):
        traces, t = obs_fixture()
        r = lv.summarize(traces, {t}, SCORES, 2)
        self.assertEqual(r["problems"], [])
        self.assertEqual((r["story_traces_with_all_three_sources"], r["generations_with_content"], r["models_with_cost"]),
                         (1, 2, ["xiaomi/mimo-v2.6-flash"]))

    def test_missing_gateway_names_protobuf(self):
        traces, t = obs_fixture(sources=("engine", "agent-core"))
        r = lv.summarize(traces, {t}, SCORES, 2)
        self.assertTrue(any("GATEWAY" in p and "protobuf" in p for p in r["problems"]), r["problems"])
        self.assertEqual(r["story_traces_with_all_three_sources"], 0)

    def test_missing_agent_core_and_engine(self):
        traces, t = obs_fixture(sources=("gateway",))
        p = " | ".join(lv.summarize(traces, {t}, SCORES, 2)["problems"])
        self.assertIn("AGENT-CORE", p)
        self.assertIn("only 0 of 1 story traces", p)

    def test_no_content_no_cost_no_scores(self):
        traces, t = obs_fixture(content=False, cost=False)
        p = " | ".join(lv.summarize(traces, {t}, [], 2)["problems"])
        self.assertIn("input AND output", p)
        self.assertIn("resolved a model with cost", p)
        self.assertIn("0 scores", p)

    def test_empty_langfuse(self):
        self.assertIn("nothing was ingested", lv.summarize({}, {"a" * 32}, [], 2)["problems"][0])

    def test_service_name_classifies(self):
        self.assertEqual(lv.source_of({"name": "POST /x", "metadata": {"resourceAttributes": {"service.name": "agentcore"}}}), "agent-core")
        self.assertEqual(lv.source_of({"name": "x", "metadata": {"resourceAttributes": {"service.name": "llm-gateway"}}}), "gateway")


class MockAndClient(unittest.TestCase):
    def setUp(self):
        self.m = ml.MockLangfuse(PK, SK)
        self.port = self.m.start()
        self.base = f"http://127.0.0.1:{self.port}"
        self.c = lv.Client(self.base, PK, SK, min_interval=0, sleep=lambda s: None)
        self.addCleanup(self.m.stop)

    def test_protobuf_roundtrip(self):
        spans = [sp("b" * 32, "c" * 16, "d" * 16, "chat m", {"s": "x", "n": 5, "f": 1.5, "b": True, "l": ["a", "b"]}, GW)]
        got = od.decode_protobuf(ml.encode_protobuf(spans))
        self.assertEqual(got, spans)

    def test_bad_credentials_are_401_and_not_leaked(self):
        bad = lv.Client(self.base, PK, "wrong-secret-value", min_interval=0, sleep=lambda s: None)
        with self.assertRaises(RuntimeError) as cm:
            bad.request("GET", "/api/public/models")
        self.assertIn("HTTP 401", str(cm.exception))
        self.assertNotIn("wrong-secret-value", str(cm.exception))

    def test_health_needs_no_auth(self):
        self.assertEqual(lv.health(lv.Client(self.base, "x", "y")), "HTTP 200 OK")

    def test_models_idempotent_and_priced(self):
        first = lv.ensure_models(self.c)
        self.assertEqual(len(first["created"]), 3)
        again = lv.ensure_models(self.c)
        self.assertEqual((again["created"], len(again["existing"])), ([], 3))
        m = self.m.models["xiaomi/mimo-v2.6-pro"]
        self.assertEqual((m["inputPrice"], m["outputPrice"], m["unit"]), (4.35e-7, 8.7e-7, "TOKENS"))

    def test_429_is_retried(self):
        self.m.rate_limit = 2
        self.assertEqual(lv.ensure_models(self.c)["existing"], [])

    def test_external_host_policy(self):
        with self.assertRaises(ValueError):
            lv.Client("https://cloud.langfuse.com", "a", "b")
        with self.assertRaises(ValueError):
            lv.Client("http://example.com", "a", "b", allow_external=True)
        lv.Client("https://us.cloud.langfuse.com", "a", "b", allow_external=True)


class EndToEnd(unittest.TestCase):
    """Real forwarder -> mock Langfuse. Engine spans (JSON, engine_trace.py), gateway and agent-core spans (protobuf, byte-exact)."""

    def setUp(self):
        self.m = ml.MockLangfuse(PK, SK)
        port = self.m.start()
        self.addCleanup(self.m.stop)
        self.env = {"LANGFUSE_BASE_URL": f"http://127.0.0.1:{port}", "LANGFUSE_PUBLIC_KEY": PK, "LANGFUSE_SECRET_KEY": SK}
        self.fwd = fw.Forwarder(self.env, False, max_retries=1, backoff=0.01)
        self.stop = threading.Event()
        threading.Thread(target=self.fwd.run_worker, args=(self.stop,), daemon=True).start()
        self.srv = fw.make_server(self.fwd, 0)
        self.fport = self.srv.server_address[1]
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.addCleanup(lambda: (self.stop.set(), self.srv.shutdown(), self.srv.server_close()))
        self.c = lv.Client(self.env["LANGFUSE_BASE_URL"], PK, SK, min_interval=0, sleep=lambda s: None)
        lv.ensure_models(self.c)

    def flush(self):
        import time
        quiet = 0
        for _ in range(200):
            quiet = quiet + 1 if self.fwd.pending == 0 else 0
            if quiet >= 3:
                return
            time.sleep(0.05)
        self.fail("forwarder did not drain")

    def send_protobuf(self, spans):
        import urllib.request
        urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{self.fport}/v1/traces", data=ml.encode_protobuf(spans), method="POST",
                                                      headers={"Content-Type": "application/x-protobuf"}), timeout=10).close()

    def engine_story(self):
        with mock.patch.dict(os.environ, {"PULSO_O11Y_FORWARDER_ENDPOINT": f"http://127.0.0.1:{self.fport}"}):
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(et.main(["--story-file", str(FIX), "--finding", "0", "--target", "forwarder", "--capture-content"]), 0)
        return out.getvalue().split("trace_id=")[1].split()[0]

    def test_three_sources_share_one_trace_and_verify_passes(self):
        t = self.engine_story()
        self.flush()
        root = [s for s in self.m.spans if s["trace_id"] == t and s["name"] == "pulso.story"][0]
        stage = [s for s in self.m.spans if s["trace_id"] == t and s["name"].startswith("stage.")][0]
        self.send_protobuf([sp(t, "e" * 16, stage["span_id"], "chat xiaomi/mimo-v2.6-flash", gen_attrs("xiaomi/mimo-v2.6-flash"), GW),
                            sp(t, "f" * 16, root["span_id"], "agentcore.api.request", {"http.route": "/v1/runs"}, AC)])
        self.flush()
        self.assertEqual(self.m.hits["protobuf"], 1)
        self.assertEqual(self.m.problems, [])
        rep = lv.verify(self.c, {"stories": [t]})
        self.assertEqual(rep["story_traces_with_all_three_sources"], 1)
        self.assertTrue(rep["scores"] >= 2, rep)
        self.assertEqual(rep["problems"], [], rep["problems"])
        self.assertGreater(rep["generations_with_content"], 0)
        self.assertIn("xiaomi/mimo-v2.6-flash", rep["models_with_cost"])

    def test_agent_core_missing_is_diagnosed(self):
        t = self.engine_story()
        self.flush()
        self.send_protobuf([sp(t, "e" * 16, "1" * 16, "chat xiaomi/mimo-v2.6-flash", gen_attrs("xiaomi/mimo-v2.6-flash"), GW)])
        self.flush()
        rep = lv.verify(self.c, {"stories": [t]})
        self.assertTrue(any("AGENT-CORE" in p for p in rep["problems"]))

    def test_langfuse_without_protobuf_is_diagnosed(self):
        self.m.reject_protobuf = True
        t = self.engine_story()
        self.flush()
        self.send_protobuf([sp(t, "e" * 16, "1" * 16, "chat xiaomi/mimo-v2.6-flash", gen_attrs("xiaomi/mimo-v2.6-flash"), GW)])
        self.flush()
        rep = lv.verify(self.c, {"stories": [t]})
        self.assertTrue(any("GATEWAY" in p and "application/x-protobuf" in p for p in rep["problems"]), rep["problems"])
        self.assertEqual(self.fwd.stats["failed"], 1)

    def test_canary_cli_output_has_no_credential(self):
        t = self.engine_story()
        self.flush()
        man = Path(os.environ.get("TEMP", ".")) / "lfc-canary-manifest.json"
        man.write_text(json.dumps({"stories": [t]}), encoding="utf-8")
        self.addCleanup(man.unlink)
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.dict(os.environ, self.env), contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = lv.main(["verify", "--manifest", str(man), "--interval", "0"])
            lv.main(["models", "--interval", "0"])
            lv.main(["health"])
        self.assertEqual(rc, 1)  # gateway and agent-core spans are absent here
        text = out.getvalue() + err.getvalue()
        for secret in (PK, SK):
            self.assertNotIn(secret, text)
        self.assertIn("VERIFY FAILED", text)
        self.assertNotIn("Basic ", text)


if __name__ == "__main__":
    unittest.main()
