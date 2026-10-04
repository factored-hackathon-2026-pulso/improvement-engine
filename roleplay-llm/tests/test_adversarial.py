import json
import tempfile
import unittest
from pathlib import Path

from roleplay_llm.scanner import scan_payload
from roleplay_llm.shim import Shim
from tests.test_scanner import payload
from tests.test_shim import SYSTEM, FINAL_STEP, TOOL_STEP, chat_body, clean, respond


class ScannerAttacks(unittest.TestCase):
    def test_large_integer_phone_in_inputs_rejected(self):
        self.assertFalse(scan_payload(payload(inputs={"x": 56912345678})).ok)

    def test_large_integer_in_args_rejected(self):
        p = payload()
        p["observations"][0]["args"] = {"n": 10**15}
        self.assertFalse(scan_payload(p).ok)

    def test_nan_and_inf_rejected(self):
        self.assertFalse(scan_payload(payload(inputs={"x": float("nan")})).ok)
        self.assertFalse(scan_payload(payload(inputs={"x": float("inf")})).ok)

    def test_trailing_newline_rejected(self):
        self.assertFalse(scan_payload(payload(inputs={"x": "abc\n"})).ok)
        p = payload()
        p["observations"][0]["status"] = "ok\n"
        self.assertFalse(scan_payload(p).ok)

    def test_fullwidth_at_email_in_goal_rejected(self):
        self.assertFalse(scan_payload(payload(goal="mail juan＠example.com")).ok)

    def test_huge_count_rejected(self):
        r = {"metric_id": "m", "window_id": "w", "count": 56912345678}
        self.assertFalse(scan_payload(payload([r])).ok)


class ShimAttacks(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.q = Path(self.tmp.name)
        self.shim = Shim(self.q, hold_s=0.0)

    def tearDown(self):
        self.tmp.cleanup()

    def test_pii_in_system_prompt_never_queued(self):
        st, body = self.shim.handle(chat_body(clean(), system=SYSTEM + " Draft: mail bob@example.com"))
        self.assertEqual(st, 422)
        self.assertEqual(list((self.q / "requests").glob("*.json")), [])

    def test_deeply_nested_body_is_400_or_422_not_crash(self):
        body = ('{"model":"m","messages":[{"role":"system","content":"s"},{"role":"user","content":"'
                + "[" * 100000 + '"}]}').encode()
        st, _ = self.shim.handle(body)
        self.assertIn(st, (400, 422))
        user = "[" * 100000 + "]" * 100000
        st, _ = self.shim.handle(json.dumps({"model": "m", "messages": [
            {"role": "system", "content": "s"}, {"role": "user", "content": user}]}).encode())
        self.assertIn(st, (400, 422))

    def test_content_extra_quality_fields_rejected(self):
        key = None
        st, body = self.shim.handle(chat_body(clean()))
        key = body["error"]["key"]
        respond(self.q, key, {"kind": "final", "output": {}, "confidence": 0.99, "quality_claims": "high"})
        st, body = self.shim.handle(chat_body(clean()))
        self.assertEqual(st, 502)

    def test_fault_status_must_be_error_code(self):
        (self.q / "faults" / "next.json").write_text(json.dumps({"status": 200, "type": "x"}))
        st, _ = self.shim.handle(chat_body(clean()))
        self.assertNotEqual(st, 200)


class HttpAttacks(unittest.TestCase):
    def test_non_ascii_auth_header_is_401_not_crash(self):
        import threading, urllib.request, urllib.error
        from roleplay_llm.shim import serve
        with tempfile.TemporaryDirectory() as d:
            srv = serve(Shim(d, hold_s=0.0), port=0, api_key="k")
            threading.Thread(target=srv.serve_forever, daemon=True).start()
            try:
                import http.client
                c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=5)
                c.putrequest("POST", "/v1/chat/completions")
                c.putheader("Authorization", "Bearer é".encode("latin-1"))
                c.putheader("Content-Length", "2")
                c.endheaders(b"{}")
                self.assertEqual(c.getresponse().status, 401)
            finally:
                srv.shutdown()


if __name__ == "__main__":
    unittest.main()
