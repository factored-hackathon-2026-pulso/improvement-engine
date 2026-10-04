import json
import tempfile
import threading
import unittest
import uuid
from pathlib import Path

from roleplay_llm.jev import JevShim, jev_replay_key, serve_jev
from roleplay_llm.jev_client import HttpJevTransport, JevTransportError
from roleplay_llm.scanner import Registry

QUESTIONS = {
    "intent": {"type": "choice", "criteria": {"billing": None, "other": None},
               "instructions": "Pick the intent id."},
    "urgent": {"type": "noul", "instructions": "Is it urgent?"},
}
ANSWERS = {"intent": {"type": "choice", "choice": "billing", "probabilities": {"billing": 0.8, "other": 0.2}},
           "urgent": {"type": "noul", "noul": 0.3}}


def req(binding=None, question_ids=None, model="jev-test", state_input=None):
    qs = QUESTIONS if question_ids is None else {k: QUESTIONS[k] for k in question_ids}
    return {"state": {"locale": "es", "input": state_input or {
        "family_id": "fam_001", "binding_id": binding or f"binding-{uuid.uuid4().hex[:12]}"}},
        "model": model, "questions": qs}


def body(r):
    return json.dumps(r).encode()


def respond(queue: Path, key: str, content, **over):
    doc = {"protocol": "roleplay-queue/1", "key": key, "provenance": "agent_roleplay",
           "quality_claims": "forbidden", "responder": {"id": "r1", "role": "jev"}, "content": content}
    doc.update(over)
    d = queue / "jev" / "responses"
    d.mkdir(parents=True, exist_ok=True)
    (d / f"{key}.tmp").write_text(json.dumps(doc))
    (d / f"{key}.tmp").replace(d / f"{key}.json")


CONTENT = {"answers": ANSWERS, "usage": {"input_tokens": 40, "output_tokens": 5}}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.queue = Path(self.tmp.name)

    def jev(self, **kw):
        kw.setdefault("hold_s", 0.3)
        kw.setdefault("poll_s", 0.02)
        return JevShim(self.queue, **kw)

    def requests(self):
        d = self.queue / "jev" / "requests"
        return sorted(d.glob("*.json")) if d.exists() else []


class Scanner(Base):
    def test_free_text_never_reaches_the_queue(self):
        r = req(state_input={"message": "Hola soy Maria Gonzalez, mi correo es maria@example.com"})
        status, out = self.jev().handle(body(r))
        self.assertEqual(status, 422)
        self.assertEqual(out["error"]["type"], "treated_payload_rejected")
        self.assertEqual(self.requests(), [])

    def test_unregistered_name_like_token_rejected_registered_token_served(self):
        r = req(state_input={"category": "closing_reply_unclear"})
        self.assertEqual(self.jev().handle(body(r))[0], 422)
        ok = self.jev(replay_only=True, registry=Registry({"closing_reply_unclear"}))
        self.assertEqual(ok.handle(body(r))[0], 409)  # passes the scanner, then a replay miss

    def test_pii_in_question_text_and_unknown_keys_rejected(self):
        r = req()
        r["questions"]["urgent"]["instructions"] = "Call 3001234567 now"
        self.assertEqual(self.jev().handle(body(r))[0], 422)
        r = req()
        r["extra"] = 1
        self.assertEqual(self.jev().handle(body(r))[0], 422)
        r = req()
        r["state"]["note"] = "x"
        self.assertEqual(self.jev().handle(body(r))[0], 422)
        self.assertEqual(self.requests(), [])

    def test_bad_bodies_are_400(self):
        self.assertEqual(self.jev().handle(b"nope")[0], 400)
        self.assertEqual(self.jev().handle(body({"model": "m"}))[0], 400)
        self.assertEqual(self.jev().handle(body(req(question_ids=[])))[0], 400)


class RoundTrip(Base):
    def test_round_trip_then_replay_zero_misses(self):
        r = req()
        key = jev_replay_key(r)
        threading.Timer(0.05, respond, (self.queue, key, CONTENT)).start()
        status, out = self.jev().handle(body(r))
        self.assertEqual(status, 200, out)
        self.assertEqual(out["answers"], ANSWERS)
        self.assertEqual(out["usage"], {"input_tokens": 40, "output_tokens": 5})
        self.assertEqual(out["model"], "jev-test")
        self.assertEqual(out["x_roleplay"]["provenance"], "agent_roleplay")
        self.assertEqual(out["x_roleplay"]["quality_claims"], "forbidden")
        queued = json.loads(self.requests()[0].read_text())
        self.assertEqual(queued["provenance"], "agent_roleplay")
        self.assertEqual(queued["scanner_id"], "tps-1")
        self.assertEqual(queued["respond_to"], f"responses/{key}.json")
        n = len(self.requests())
        replay = self.jev(replay_only=True)
        for _ in range(3):
            status, out = replay.handle(body(req()))  # fresh binding ids normalise to the same key
            self.assertEqual(status, 200, out)
            self.assertEqual(out["answers"], ANSWERS)
        self.assertEqual(len(self.requests()), n)

    def test_semantic_change_is_a_replay_miss_with_diff(self):
        r = req()
        respond(self.queue, jev_replay_key(r), CONTENT)
        self.assertEqual(self.jev().handle(body(r))[0], 200)
        other = req(question_ids=["urgent"])
        self.assertNotEqual(jev_replay_key(r), jev_replay_key(other))
        status, out = self.jev(replay_only=True).handle(body(other))
        self.assertEqual(status, 409)
        self.assertEqual(out["error"]["type"], "replay_miss")

    def test_timeout_is_typed_504_and_late_answer_serves_retry(self):
        r = req()
        status, out = self.jev(hold_s=0.1).handle(body(r))
        self.assertEqual((status, out["error"]["type"]), (504, "responder_timeout"))
        self.assertEqual(len(self.requests()), 1)
        respond(self.queue, jev_replay_key(r), CONTENT)
        self.assertEqual(self.jev().handle(body(r))[0], 200)

    def test_invalid_answers_are_502_and_rejected(self):
        usage = CONTENT["usage"]
        bad = [
            ({"answers": {"intent": ANSWERS["intent"]}, "usage": usage}, "missing question"),
            ({"answers": {**ANSWERS, "ghost": ANSWERS["urgent"]}, "usage": usage}, "extra question"),
            ({"answers": {**ANSWERS, "urgent": {"type": "noul", "noul": 1.5}}, "usage": usage}, "range"),
            ({"answers": {**ANSWERS, "intent": {"type": "choice", "choice": "zzz",
                                                "probabilities": {"zzz": 1}}}, "usage": usage}, "option"),
            ({"answers": ANSWERS, "usage": {"input_tokens": "x", "output_tokens": 1}}, "usage"),
            ({"answers": ANSWERS, "usage": usage, "confidence": 0.9}, "extra field"),
        ]
        for content, why in bad:
            with self.subTest(why):
                r = req()
                respond(self.queue, jev_replay_key(r), content)
                status, out = self.jev().handle(body(r))
                self.assertEqual((status, out["error"]["type"]), (502, "invalid_output"))
        r = req()
        respond(self.queue, jev_replay_key(r), CONTENT, quality_claims="allowed")
        self.assertEqual(self.jev().handle(body(r))[0], 502)
        r = req()
        respond(self.queue, jev_replay_key(r), CONTENT, provenance="real")
        self.assertEqual(self.jev().handle(body(r))[0], 502)

    def test_scripted_fault_only_via_side_channel(self):
        r = req()
        (self.queue / "jev" / "faults").mkdir(parents=True)
        (self.queue / "jev" / "faults" / "next.json").write_text('{"status":429,"type":"rate_limited"}')
        status, out = self.jev().handle(body(r))
        self.assertEqual((status, out["error"]["type"]), (429, "rate_limited"))
        self.assertEqual(self.requests(), [])


class Http(Base):
    def start(self, **kw):
        server = serve_jev(self.jev(**kw), port=0, api_key="k")
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return f"http://127.0.0.1:{server.server_address[1]}"

    def transport(self, base, key="k", path="/v1/jev"):
        return HttpJevTransport(base, lambda: key, path=path)

    def test_port_round_trip_request_queue_responder_response(self):
        base = self.start()
        r = req()
        threading.Timer(0.05, respond, (self.queue, jev_replay_key(r), CONTENT)).start()
        out = self.transport(base).send(r, 5000)
        self.assertEqual(out["answers"], ANSWERS)
        # direct systemone path serves the same answer (what the gateway forwards to)
        self.assertEqual(self.transport(base, path="/v1/systemone").send(req(), 5000)["answers"], ANSWERS)

    def test_replay_over_http_zero_misses(self):
        r = req()
        respond(self.queue, jev_replay_key(r), CONTENT)
        self.jev().handle(body(r))
        base = self.start(replay_only=True)
        t = self.transport(base)
        for _ in range(3):
            self.assertEqual(t.send(req(), 5000)["answers"], ANSWERS)
        ledger = (self.queue / "jev" / "ledger.jsonl").read_text()
        self.assertNotIn("replay_miss", ledger)

    def test_port_handles_4xx_and_502_and_timeout(self):
        base = self.start(hold_s=0.2)
        with self.assertRaises(JevTransportError) as c:
            self.transport(base, key="").send(req(), 5000)  # header stripped
        self.assertEqual(c.exception.status, 401)
        with self.assertRaises(JevTransportError) as c:
            self.transport(base, key="wrong").send(req(), 5000)
        self.assertEqual(c.exception.status, 401)
        with self.assertRaises(JevTransportError) as c:
            self.transport(base).send(req(state_input={"m": "Maria Gonzalez Perez"}), 5000)
        self.assertEqual(c.exception.status, 422)
        r = req()
        respond(self.queue, jev_replay_key(r), {"answers": {}, "usage": CONTENT["usage"]})
        with self.assertRaises(JevTransportError) as c:
            self.transport(base).send(r, 5000)
        self.assertEqual(c.exception.status, 502)
        with self.assertRaises(TimeoutError):  # 504 responder_timeout maps to TimeoutError
            self.transport(base).send(req(), 5000)

    def test_unknown_path_404(self):
        base = self.start()
        with self.assertRaises(JevTransportError) as c:
            self.transport(base, path="/v1/other").send(req(), 1000)
        self.assertEqual(c.exception.status, 404)


if __name__ == "__main__":
    unittest.main()
