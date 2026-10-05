import io
import json
import os
import sys
import unittest
from contextlib import redirect_stderr, redirect_stdout

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import judge_calibration  # noqa: E402
from judges import gateway_judge as gj  # noqa: E402
from score_proposal import JUDGED, JudgeError, run_judge  # noqa: E402

KEY = "sk-test-SECRET-123"
ENV = {"PULSO_LLM_GATEWAY_ADDR": "127.0.0.1:8080", "PULSO_LLM_GATEWAY_KEY": KEY}
BUILDER, JUDGE = "xiaomi/mimo-v2.6-flash", "z-ai/glm-5.3-flash"


def answer(**scores):
    return {c: {"score": scores.get(c, 2), "justification": f"{c} ok"} for c in JUDGED}


class Scripted:
    """Scripted transport: pops responses, records bodies; no network."""
    def __init__(self, *responses):
        self.responses, self.bodies = list(responses), []

    def __call__(self, addr, key, body, timeout):
        self.bodies.append(body)
        r = self.responses.pop(0)
        return r if isinstance(r, tuple) else (200, {"output": r})


REQ = {"proposal": {"change": "x", "builder_reasoning": "LEAK-ME", "nested": {"reasoning": "LEAK-TOO"}},
       "base": {}, "rubric_criteria": list(JUDGED)}


class JudgeTests(unittest.TestCase):
    def make(self, *responses, env=None):
        t = Scripted(*responses)
        return gj.GatewayJudge({**ENV, **(env or {})}, t), t

    def test_family_guard(self):
        with self.assertRaises(JudgeError):
            gj.GatewayJudge({**ENV, "PULSO_JUDGE_MODEL": "xiaomi/mimo-v2.6-pro"}, Scripted())
        with self.assertRaises(JudgeError):
            run_judge(lambda r: {"R1": 1}, REQ, BUILDER, "xiaomi/mimo-v2.6-pro")
        j, _ = self.make(answer())
        self.assertEqual(j(REQ), {c: 2 for c in JUDGED})

    def test_defaults_are_other_family(self):
        j = gj.GatewayJudge(ENV, Scripted())
        self.assertEqual((j.judge_model, j.builder_model), (JUDGE, BUILDER))

    def test_double_sampling_takes_min(self):
        j, t = self.make(answer(R1=2, R8=1), answer(R1=1, R8=2))
        res = run_judge(j, REQ, BUILDER, JUDGE)
        self.assertEqual(len(t.bodies), 2)
        self.assertEqual((res["scores"]["R1"], res["scores"]["R8"]), (1, 1))
        self.assertEqual(res["escalate_human"], [])

    def test_escalation_on_gap_over_one(self):
        j, _ = self.make(answer(R2=2), answer(R2=0))
        res = run_judge(j, REQ, BUILDER, JUDGE)
        self.assertEqual(res["scores"]["R2"], 0)
        self.assertEqual(res["escalate_human"], ["R2"])

    def test_malformed_retry_then_ok(self):
        j, t = self.make({"R1": 2}, answer())
        self.assertEqual(j(REQ)["R1"], 2)
        self.assertEqual(len(t.bodies), 2)

    def test_gateway_invalid_output_is_retried_like_malformed(self):
        # real glm-5.3-flash hit max_tokens and the gateway answered 502 invalid_output (live run)
        j, t = self.make((502, {"error": {"kind": "invalid_output"}}), answer())
        self.assertEqual(j(REQ)["R1"], 2)
        self.assertEqual(len(t.bodies), 2)

    def test_verbose_multiline_justification_is_normalised_not_denied(self):
        # live glm answered multi-line / long justifications; the score is what is validated strictly
        a = answer()
        a["R1"]["justification"] = "line one\n\nline two " + "x" * 400
        j, t = self.make(a)
        self.assertEqual(j(REQ)["R1"], 2)
        self.assertEqual(len(t.bodies), 1)
        self.assertNotIn("\n", j.justifications["R1"])
        self.assertLessEqual(len(j.justifications["R1"]), 240)

    def test_transport_retries_once_on_dropped_connection(self):
        import http.client
        from unittest import mock
        from judges import gateway_judge as g
        calls = []

        class Conn:
            def __init__(self, *a, **k):
                calls.append(1)

            def request(self, *a, **k):
                if len(calls) == 1:
                    raise http.client.RemoteDisconnected("x")

            def getresponse(self):
                class R:
                    status = 200

                    def read(self):
                        return b'{"output": {}}'
                return R()

            def close(self):
                pass
        with mock.patch.object(g.http.client, "HTTPConnection", Conn):
            self.assertEqual(g.http_transport("127.0.0.1:1", "k", {}, 1)[0], 200)
            calls.clear()
            Conn.request = lambda self, *a, **k: (_ for _ in ()).throw(http.client.RemoteDisconnected("x"))
            with self.assertRaises(JudgeError):
                g.http_transport("127.0.0.1:1", "k", {}, 1)
        self.assertEqual(len(calls), 2)

    def test_malformed_twice_denies(self):
        bad = answer()
        bad["R1"]["score"] = 3
        j, t = self.make(bad, "not json", env=None)
        with self.assertRaises(JudgeError):
            j(REQ)
        self.assertEqual(len(t.bodies), 2)

    def test_http_error_denies_and_no_extra_keys(self):
        j, _ = self.make((401, {"error": {"kind": "unauthorized"}}))
        with self.assertRaises(JudgeError):
            j(REQ)
        extra = answer()
        extra["R1"]["extra"] = 1
        j, _ = self.make(extra, extra)
        with self.assertRaises(JudgeError):
            j(REQ)

    def test_no_builder_reasoning_in_prompt(self):
        j, t = self.make(answer())
        j(REQ)
        wire = json.dumps(t.bodies[0])
        self.assertNotIn("LEAK-ME", wire)
        self.assertNotIn("LEAK-TOO", wire)
        self.assertIn("schema", t.bodies[0])
        self.assertEqual(t.bodies[0]["profile"]["model"], JUDGE)

    def test_schema_uses_only_gateway_supported_keywords(self):
        # the real gateway rejects 'maxLength' ("palabra clave no soportada"); found in the live run
        from judges.gateway_judge import _schema
        self.assertNotIn("maxLength", json.dumps(_schema(["R1", "R2"])))

    def test_profile_carries_price(self):
        # the real gateway answers 400 "profile.price: required object" without it (found in the live run)
        j, t = self.make(answer())
        j(REQ)
        price = t.bodies[0]["profile"]["price"]
        self.assertEqual(set(price), {"input_per_mtok", "output_per_mtok"})
        self.assertTrue(all(isinstance(v, str) and 0 <= float(v) <= 1000000 for v in price.values()))  # decimal STRINGS

    def test_no_credentials_in_outputs(self):
        body = Scripted(answer())
        j = gj.GatewayJudge(ENV, body)
        j(REQ)
        self.assertNotIn(KEY, json.dumps(body.bodies))
        self.assertNotIn(KEY, json.dumps(j.justifications))
        def leaky(addr, key, b, t):
            raise JudgeError(f"boom {key}")
        j2 = gj.GatewayJudge(ENV, leaky)
        with self.assertRaises(JudgeError) as cm:
            j2(REQ)
        self.assertNotIn(KEY, str(cm.exception))
        with self.assertRaises(JudgeError) as cm:
            gj.GatewayJudge({"PULSO_LLM_GATEWAY_ADDR": "127.0.0.1:1"}, Scripted())
        self.assertNotIn(KEY, str(cm.exception))

    def test_remote_addr_refused(self):
        with self.assertRaises(JudgeError):
            gj.GatewayJudge({**ENV, "PULSO_LLM_GATEWAY_ADDR": "example.com:80"}, Scripted())

    def test_alt_key_var(self):
        j = gj.GatewayJudge({"PULSO_LLM_GATEWAY_ADDR": "localhost:8080", "GATEWAY_TOKEN_AGENT_CORE": KEY}, Scripted())
        self.assertEqual(j.key, KEY)


class CalibrationTests(unittest.TestCase):
    def golden(self):
        with open(judge_calibration.DEFAULT_GOLDEN, encoding="utf-8") as f:
            return json.load(f)

    def test_golden_shape(self):
        g = self.golden()
        self.assertEqual(len(g["proposals"]), 12)
        for p in g["proposals"]:
            self.assertEqual(set(p["expected"]), set(JUDGED))
        self.assertTrue(any(0 in p["expected"].values() for p in g["proposals"]))
        self.assertTrue(any(all(v == 2 for v in p["expected"].values()) for p in g["proposals"]))

    def test_perfect_and_off_by_one_judge(self):
        g = self.golden()
        exp = {p["proposal"]["id"]: p["expected"] for p in g["proposals"]}
        perfect = lambda r: dict(exp[r["proposal"]["id"]])
        res = judge_calibration.calibrate(g, perfect, BUILDER, JUDGE)
        self.assertEqual((res["exact"], res["within_1"], res["hard_gate"]), (1.0, 1.0, 1.0))
        self.assertEqual(res["pairs"], 72)
        shifted = lambda r: {c: min(2, v + 1) for c, v in exp[r["proposal"]["id"]].items()}
        res = judge_calibration.calibrate(g, shifted, BUILDER, JUDGE)
        self.assertEqual(res["within_1"], 1.0)
        self.assertLess(res["exact"], 1.0)
        self.assertLess(res["hard_gate"], 1.0)

    def test_escalated_excluded_and_family_refusal(self):
        g = self.golden()
        calls = []
        alt = lambda r: (calls.append(1), {c: 2 if len(calls) % 2 else 0 for c in JUDGED})[1]  # samples differ by 2
        res = judge_calibration.calibrate(g, alt, BUILDER, JUDGE, limit=1)
        self.assertTrue(res["escalated_to_human"])
        with self.assertRaises(JudgeError):
            judge_calibration.calibrate(g, alt, BUILDER, "xiaomi/mimo-v2.6-pro")

    def test_denied_proposal_is_counted_not_fatal(self):
        g = self.golden()
        exp = {p["id"]: p["expected"] for p in g["proposals"]}
        n = []

        def flaky(r):
            n.append(1)
            if len(n) == 1:
                raise JudgeError("malformed judge output after retry, denied")
            return dict(exp[r["proposal"]["id"]])
        res = judge_calibration.calibrate(g, flaky, BUILDER, JUDGE, limit=2)
        self.assertEqual(len(res["denied"]), 1)
        self.assertEqual(res["proposals"], 2)
        self.assertEqual(res["exact"], 1.0)

    def test_gateway_unreachable_aborts_calibration(self):
        def down(r):
            raise JudgeError("gateway unreachable (ConnectionRefusedError)")
        with self.assertRaises(JudgeError):
            judge_calibration.calibrate(self.golden(), down, BUILDER, JUDGE, limit=2)

    def test_cli_not_exercised_without_live(self):
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(io.StringIO()):
            self.assertEqual(judge_calibration.main([]), 0)
        self.assertEqual(json.loads(out.getvalue())["status"], "not_exercised")


if __name__ == "__main__":
    unittest.main()
