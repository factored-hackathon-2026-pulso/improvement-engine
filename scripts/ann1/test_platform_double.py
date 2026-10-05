import json
import unittest
import urllib.error
import urllib.request

import platform_double as pd

TOKEN = "throwaway-token-for-the-double"
CASE = "CASE-" + "0" * 26


def body(pid="prp_1", **o):
    b = {"proposalId": pid, "title": "t", "problem": "p", "evidence": "e", "expectedEffect": "x", "evidenceLinks": [CASE]}
    b.update(o)
    return b


class DoubleTest(unittest.TestCase):
    def setUp(self):
        self.p = pd.Platform(TOKEN, {"prp_1": "auto_detect", "hand": "builder_chat"})
        self.srv = pd.serve(self.p)
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}{pd.ROUTE}"

    def tearDown(self):
        self.srv.shutdown()

    def post(self, b, token=TOKEN):
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = f"Bearer {token}"
        r = urllib.request.Request(self.url, data=json.dumps(b).encode(), method="POST", headers=headers)
        try:
            with urllib.request.urlopen(r) as x:
                return x.status, json.loads(x.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read())

    def test_idempotent_and_one_notification_per_supervisor(self):
        a, b = self.post(body()), self.post(body())
        self.assertEqual((a[0], b[0], a[1] == b[1]), (200, 200, True))
        self.assertEqual(self.p.notifications, {"prp_1": 2})
        self.assertEqual(a[1]["origin"], "auto_detect")

    def test_token(self):
        self.assertEqual(self.post(body(), token=None)[0], 401)
        self.assertEqual(self.post(body(), token="nope")[0], 401)

    def test_unknown_and_by_hand(self):
        self.assertEqual(self.post(body("nope"))[0], 404)
        self.assertEqual(self.post(body("hand"))[0], 422)

    def test_bad_payloads_are_422_and_adopt_nothing(self):
        bad = [{"title": ""}, {"title": "x" * 121}, {"problem": "x" * 601}, {"evidence": "x" * 601}, {"expectedEffect": "x" * 401}, {"evidenceLinks": [CASE] * 9},
               {"evidenceLinks": ["https://x"]}, {"evidenceLinks": [CASE, CASE]}, {"proposalId": "x" * 65}, {"unknown": 1}, {"problem": "a@b.com"}, {"problem": "llamar 3001234567"}]
        for o in bad:
            self.assertEqual(self.post(body(**o))[0], 422, o)
        self.assertEqual(self.p.adopted, {})
        self.assertEqual(self.post(body(problem="recepcion@1.0.0 con 117.021 casos"))[0], 200)


if __name__ == "__main__":
    unittest.main()
