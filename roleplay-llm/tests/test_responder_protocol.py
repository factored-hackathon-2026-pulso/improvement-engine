import unittest

from roleplay_llm.protocol import STEP_CAPS, check_assignments, check_manifest, check_step_budget

REQ = "queue/requests/abc.json"
RESP = "queue/responses/abc.json"


class ResponderManifest(unittest.TestCase):
    def test_responder_that_sees_the_repo_is_rejected(self):
        for reads in (["."], ["roleplay-llm/"], [REQ, "src/engine.rs"], ["queue/requests/"], ["D:/"]):
            r = check_manifest({"reads": reads, "writes": [RESP]}, REQ, RESP)
            self.assertFalse(r.ok, reads)

    def test_exact_queue_file_manifest_is_accepted(self):
        self.assertTrue(check_manifest({"reads": [REQ], "writes": [RESP]}, REQ, RESP).ok)

    def test_write_outside_response_file_is_rejected(self):
        self.assertFalse(check_manifest({"reads": [REQ], "writes": [RESP, "notes.md"]}, REQ, RESP).ok)
        self.assertFalse(check_manifest({"reads": [REQ], "writes": []}, REQ, RESP).ok)

    def test_tools_beyond_read_write_are_rejected(self):
        m = {"reads": [REQ], "writes": [RESP], "tools": ["shell"]}
        self.assertFalse(check_manifest(m, REQ, RESP).ok)


class Assignments(unittest.TestCase):
    good = {"scout": {"responder": "r-scout", "model": "m-a"},
            "verifier": {"responder": "r-ver", "model": "m-b"},
            "builder": {"responder": "r-bld", "model": "m-c"}}

    def test_distinct_responders_and_models_pass(self):
        self.assertTrue(check_assignments(self.good).ok)

    def test_scout_and_verifier_sharing_a_responder_or_model_fail(self):
        a = {**self.good, "verifier": {"responder": "r-scout", "model": "m-b"}}
        self.assertFalse(check_assignments(a).ok)
        b = {**self.good, "verifier": {"responder": "r-ver", "model": "m-a"}}
        self.assertFalse(check_assignments(b).ok)

    def test_responder_that_is_implementer_or_reviewer_fails(self):
        self.assertFalse(check_assignments(self.good, excluded={"r-bld"}).ok)


class Budget(unittest.TestCase):
    def test_caps(self):
        self.assertEqual(STEP_CAPS, {"scout": 5, "verifier": 5, "builder": 7})
        self.assertTrue(check_step_budget("scout", 5).ok)
        self.assertFalse(check_step_budget("scout", 6).ok)
        self.assertTrue(check_step_budget("builder", 7).ok)
        self.assertFalse(check_step_budget("builder", 8).ok)
        self.assertFalse(check_step_budget("writer-unknown", 1).ok)


if __name__ == "__main__":
    unittest.main()
