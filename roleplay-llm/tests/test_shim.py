import json
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
import uuid
from pathlib import Path

from roleplay_llm.shim import Shim, normalise_inputs, replay_key, serve

SYSTEM = "You are the scout. Respond with one JSON step."
TOOL = "pulso/lab_query@1.0.0"


def inputs(run_id=None, step=1, binding=None, rows=None):
    run_id = run_id or str(uuid.uuid4())
    return {
        "goal": "Find the largest drop.",
        "inputs": {"family_id": "fam_001", "binding_id": binding or f"binding-{uuid.uuid4().hex[:12]}"},
        "step": step,
        "tools": [{"tool": TOOL, "description": "Query treated aggregates.",
                   "args_schema": {"type": "object"}}],
        "observations": [] if rows is None else [{"tool": TOOL, "args": {"metric_id": "m1"}, "status": "ok",
                                                  "result": {"rows": rows}, "error": None}],
        "feedback": None,
        "output_schema": {"type": "object"},
        "run_id_probe": None,
    }


def clean(**kw):
    d = inputs(**kw)
    del d["run_id_probe"]
    return d


def chat_body(inp, model="external-reasoning-model", system=SYSTEM):
    return json.dumps({
        "model": model, "temperature": 0, "max_tokens": 1000,
        "messages": [{"role": "system", "content": system},
                     {"role": "user", "content": json.dumps(inp, sort_keys=True, separators=(",", ":"))}],
    }).encode()


def respond(queue: Path, key: str, content, **over):
    doc = {"protocol": "roleplay-queue/1", "key": key, "provenance": "agent_roleplay",
           "quality_claims": "forbidden", "responder": {"id": "r1", "role": "scout"}, "content": content}
    doc.update(over)
    tmp = queue / "responses" / f"{key}.tmp"
    tmp.parent.mkdir(parents=True, exist_ok=True)
    tmp.write_text(json.dumps(doc))
    tmp.replace(queue / "responses" / f"{key}.json")


TOOL_STEP = {"kind": "tool_call", "tool": TOOL, "args": {"metric_id": "m1"}}
FINAL_STEP = {"kind": "final", "output": {"summary": "ok"}}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.queue = Path(self.tmp.name)

    def shim(self, **kw):
        kw.setdefault("hold_s", 0.3)
        kw.setdefault("poll_s", 0.02)
        return Shim(self.queue, **kw)

    def request_files(self):
        d = self.queue / "requests"
        return sorted(d.glob("*.json")) if d.exists() else []


class ReplayKey(Base):
    def test_new_run_turn_session_and_ids_give_the_same_key(self):
        a = clean(binding="binding-aaaaaaaaaaaa")
        b = clean(binding="binding-bbbbbbbbbbbb")
        self.assertEqual(replay_key(SYSTEM, a), replay_key(SYSTEM, b))
        self.assertEqual(normalise_inputs(a), normalise_inputs(b))

    def test_semantic_change_changes_key(self):
        a, b = clean(), clean()
        b["goal"] = "Find the smallest drop."
        self.assertNotEqual(replay_key(SYSTEM, a), replay_key(SYSTEM, b))
        self.assertNotEqual(replay_key(SYSTEM, a), replay_key(SYSTEM + " x", a))

    def test_distinct_ids_keep_distinct_ordinals(self):
        a = clean()
        a["inputs"]["other"] = str(uuid.uuid4())
        a["inputs"]["same_as_other"] = a["inputs"]["other"]
        n = normalise_inputs(a)
        self.assertEqual(n["inputs"]["other"], n["inputs"]["same_as_other"])
        self.assertNotEqual(n["inputs"]["other"], n["inputs"]["binding_id"])


class RoundTrip(Base):
    def test_json_step_round_trip_and_late_replays_with_new_run_ids(self):
        shim = self.shim()
        first = clean()
        key = replay_key(SYSTEM, first)
        threading.Timer(0.05, respond, (self.queue, key, TOOL_STEP)).start()
        status, body = shim.handle(chat_body(first))
        self.assertEqual(status, 200, body)
        msg = body["choices"][0]["message"]
        self.assertEqual(msg["role"], "assistant")
        self.assertNotIn("tool_calls", msg)
        self.assertEqual(json.loads(msg["content"]), TOOL_STEP)
        self.assertEqual(body["model"], "external-reasoning-model")
        self.assertTrue(body["usage_estimated"])
        self.assertEqual(body["x_roleplay"]["provenance"], "agent_roleplay")
        self.assertEqual(body["x_roleplay"]["quality_claims"], "forbidden")
        # replay the same stage 3 times with fresh ids: 0 misses, nothing new queued
        n_requests = len(self.request_files())
        for _ in range(3):
            status, body = self.shim(replay_only=True).handle(chat_body(clean()))
            self.assertEqual(status, 200, body)
            self.assertEqual(json.loads(body["choices"][0]["message"]["content"]), TOOL_STEP)
        self.assertEqual(len(self.request_files()), n_requests)

    def test_final_step_round_trip(self):
        inp = clean(step=2, rows=[])
        key = replay_key(SYSTEM, inp)
        respond(self.queue, key, FINAL_STEP)
        status, body = self.shim().handle(chat_body(inp))
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(body["choices"][0]["message"]["content"]), FINAL_STEP)

    def test_request_file_is_normalised_and_labelled(self):
        shim = self.shim(hold_s=0.05)
        inp = clean()
        shim.handle(chat_body(inp))
        (path,) = self.request_files()
        doc = json.loads(path.read_text())
        self.assertEqual(doc["protocol"], "roleplay-queue/1")
        self.assertEqual(doc["provenance"], "agent_roleplay")
        self.assertEqual(doc["system_prompt"], SYSTEM)
        self.assertNotIn(inp["inputs"]["binding_id"], path.read_text())
        self.assertEqual(doc["respond_to"], f"responses/{doc['key']}.json")


class SlowResponder(Base):
    def test_slow_call_returns_typed_504_and_late_answer_serves_retry(self):
        shim = self.shim(hold_s=0.15)
        inp = clean()
        key = replay_key(SYSTEM, inp)
        status, body = shim.handle(chat_body(inp))
        self.assertEqual(status, 504)
        self.assertEqual(body["error"]["type"], "responder_timeout")
        self.assertEqual(len(self.request_files()), 1)
        respond(self.queue, key, FINAL_STEP)  # the late answer
        t0 = time.monotonic()
        status, body = shim.handle(chat_body(clean()))  # retry with new ids
        self.assertEqual(status, 200)
        self.assertLess(time.monotonic() - t0, 0.1)
        self.assertEqual(len(self.request_files()), 1)  # no second job


class ScannerEnforced(Base):
    def test_rejected_payload_never_reaches_the_queue(self):
        inp = clean()
        inp["observations"] = [{"tool": TOOL, "args": {}, "status": "ok", "error": None,
                                "result": {"rows": [{"metric_id": "m", "window_id": "w", "count": 50,
                                                     "note": "customer wrote this free text"}]}}]
        status, body = self.shim().handle(chat_body(inp))
        self.assertEqual(status, 422)
        self.assertEqual(body["error"]["type"], "treated_payload_rejected")
        self.assertNotIn("customer wrote", json.dumps(body))
        self.assertEqual(self.request_files(), [])
        ledger = (self.queue / "ledger.jsonl").read_text()
        self.assertIn("scanner_rejected", ledger)
        self.assertNotIn("customer wrote", ledger)

    def test_non_json_user_message_rejected(self):
        body = json.dumps({"model": "m", "messages": [{"role": "system", "content": "s"},
                                                      {"role": "user", "content": "free text, not JSON"}]})
        status, resp = self.shim().handle(body.encode())
        self.assertEqual(status, 422)
        self.assertEqual(self.request_files(), [])

    def test_malformed_requests_are_400(self):
        for raw in (b"not json", json.dumps({"model": "m"}).encode(),
                    json.dumps({"model": "m", "messages": [], "stream": True}).encode()):
            self.assertEqual(self.shim().handle(raw)[0], 400)


class ResponseValidation(Base):
    def _bad(self, **over):
        inp = clean()
        key = replay_key(SYSTEM, inp)
        respond(self.queue, key, over.pop("content", FINAL_STEP), **over)
        return self.shim().handle(chat_body(inp))

    def test_quality_claims_forbidden(self):
        status, body = self._bad(quality_claims="high")
        self.assertEqual(status, 502)
        self.assertEqual(body["error"]["type"], "invalid_output")
        status, _ = self._bad(content={"kind": "final", "output": {}}, quality_score=0.9)
        self.assertEqual(status, 502)

    def test_must_be_labelled_agent_roleplay(self):
        self.assertEqual(self._bad(provenance="real")[0], 502)

    def test_content_must_be_a_step(self):
        self.assertEqual(self._bad(content={"kind": "other"})[0], 502)
        self.assertEqual(self._bad(content="text")[0], 502)
        self.assertEqual(self._bad(content={"kind": "tool_call"})[0], 502)


class FaultChannel(Base):
    def test_scripted_fault_is_returned_once_and_then_normal_flow(self):
        inp = clean()
        key = replay_key(SYSTEM, inp)
        respond(self.queue, key, FINAL_STEP)
        faults = self.queue / "faults"
        faults.mkdir()
        (faults / "next.json").write_text(json.dumps({"status": 502, "type": "invalid_output"}))
        shim = self.shim()
        status, body = shim.handle(chat_body(inp))
        self.assertEqual((status, body["error"]["type"]), (502, "invalid_output"))
        self.assertFalse((faults / "next.json").exists())
        self.assertEqual(shim.handle(chat_body(inp))[0], 200)


class ReplayDrift(Base):
    def test_miss_in_replay_mode_is_typed_and_shows_digest_diff(self):
        inp = clean()
        key = replay_key(SYSTEM, inp)
        shim = self.shim(hold_s=0.05)
        shim.handle(chat_body(inp))  # records the request
        respond(self.queue, key, FINAL_STEP)
        drifted = clean()
        drifted["goal"] = "Find the largest drop, quickly."
        status, body = self.shim(replay_only=True).handle(chat_body(drifted))
        self.assertEqual(status, 409)
        self.assertEqual(body["error"]["type"], "replay_miss")
        self.assertIn("goal", json.dumps(body["error"]["diff"]))
        self.assertEqual(len(self.request_files()), 1)


class Http(Base):
    def test_http_endpoint_chat_completions_with_bearer(self):
        server = serve(self.shim(), host="127.0.0.1", port=0, api_key="dummy")
        self.addCleanup(server.server_close)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.shutdown)
        url = f"http://127.0.0.1:{server.server_address[1]}/v1/chat/completions"
        inp = clean()
        respond(self.queue, replay_key(SYSTEM, inp), FINAL_STEP)

        def post(auth):
            req = urllib.request.Request(url, data=chat_body(inp), method="POST",
                                         headers={"Content-Type": "application/json", "Authorization": auth})
            return urllib.request.urlopen(req, timeout=5)

        with post("Bearer dummy") as r:
            self.assertEqual(r.status, 200)
            self.assertEqual(json.load(r)["choices"][0]["message"]["role"], "assistant")
        with self.assertRaises(urllib.error.HTTPError) as ctx:
            post("Bearer wrong")
        self.assertEqual(ctx.exception.code, 401)


if __name__ == "__main__":
    unittest.main()
