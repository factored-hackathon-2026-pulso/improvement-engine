import copy
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import engine_run as er  # noqa: E402


def good():
    step = lambda i, status, dc="generated_sample", **kw: {
        "id": i, "status": status, "data_class": dc, "target": "local", "sha": "a" * 40,
        "contract_revision": "c2-1", "host": "rust", **kw}
    return {
        "contract_revision": "c2-1", "target": "local", "sha": "a" * 40, "host": "rust",
        "label": "DEMO-1a", "quality_claims": "forbidden",
        "steps": [
            step("scout", "agent_roleplay", receipt={"provider": "agent_roleplay", "scanner_id": "tps-1"},
                 actor="scout-1", model="agent_roleplay", stage_output={"source": "model"}),
            step("verifier", "agent_roleplay", receipt={"provider": "agent_roleplay", "scanner_id": "tps-1"},
                 actor="verifier-1", model="agent_roleplay-b", stage_output={"source": "model"}),
            step("gate", "stand-in", receipt={"provider": "claude-standin"}),
            step("memory", "not_exercised"),
        ],
        "ports": [{"port": "registry", "provenance": "core:manifest-digest", "price_source": "registry"}],
        "authors": {"world": "a", "suite": "b", "effect": "c", "judge": "d",
                    "suite_sealed_at": "2026-01-01T00:00:00Z", "candidate_created_at": "2026-01-02T00:00:00Z"},
    }


def rules(report):
    return {v["rule"] for v in er.check(report)}


class Honesty(unittest.TestCase):
    def test_good_report_clean(self):
        self.assertEqual(er.check(good()), [])

    def test_1_real_with_nonreal_provider(self):
        for p in ("agent_roleplay", "recorded", "scripted"):
            r = good(); r["steps"][2].update(status="real", receipt={"provider": p})
            self.assertIn("H1", rules(r), p)

    def test_2_third_party_needs_scanner(self):
        r = good(); r["steps"][0]["data_class"] = "E0"; del r["steps"][0]["receipt"]["scanner_id"]
        self.assertIn("H2", rules(r))
        r["steps"][0]["receipt"]["scanner_id"] = "tps-1"
        self.assertNotIn("H2", rules(r))
        r["steps"][0]["receipt"].update(provider="hosted", model_kind="real")
        self.assertIn("H2", rules(r))  # needs third_party_ok
        r["steps"][0]["receipt"]["third_party_ok"] = "user sentence"
        self.assertNotIn("H2", rules(r))

    @unittest.expectedFailure
    def test_3_mapping_mutation_skeleton(self):
        self.assertEqual(er.check_mapping_mutation(lambda cats: {}, {}), [])  # turned green by ED0/SMAP

    def test_4_ports_and_template_fallback(self):
        r = good(); r["ports"] = []
        self.assertIn("H4", rules(r))
        r = good(); del r["ports"][0]["price_source"]
        self.assertIn("H4", rules(r))
        r = good(); r["steps"][0]["stage_output"]["source"] = "template_fallback"
        self.assertIn("H4", rules(r))

    def test_5_authors_and_sealing(self):
        r = good(); del r["authors"]["judge"]
        self.assertIn("H5", rules(r))
        r = good(); r["authors"]["judge"] = r["authors"]["suite"]
        self.assertIn("H5", rules(r))
        r = good(); r["authors"]["suite_sealed_at"] = "2026-01-03T00:00:00Z"
        self.assertIn("H5", rules(r))

    def test_6_python_host_label_cap(self):
        r = good(); r["host"] = "python"; r["label"] = "DEMO-1b"
        self.assertIn("H6", rules(r))
        r["label"] = "DEMO-1a"
        self.assertNotIn("H6", rules(r))
        r = good(); r["steps"][0]["host"] = "python"; r["label"] = "DEMO-2"
        self.assertIn("H6", rules(r))

    def test_7_distinct_actors_models(self):
        r = good(); r["steps"][1]["actor"] = "scout-1"
        self.assertIn("H7", rules(r))
        r = good(); r["steps"][1]["model"] = r["steps"][0]["model"]
        self.assertIn("H7", rules(r))

    def test_8_e0_receipt_body_uses_dc0_scanner(self):
        self.assertTrue(er.scan_receipt("data=E0 rows"))
        self.assertFalse(er.scan_receipt("data=generated_sample"))
        self.assertEqual(er.SCANNER_ID, "dc0-content-scan/1")
        r = good(); r["steps"][0]["receipt"]["body"] = '{"data": "E0"}'
        self.assertIn("H8", rules(r))


class Schema(unittest.TestCase):
    def test_per_step_provenance_required(self):
        for k in ("target", "sha", "contract_revision", "host"):
            r = good(); del r["steps"][0][k]
            self.assertIn("S1", rules(r), k)

    def test_status_vocab(self):
        r = good(); r["steps"][0]["status"] = "kinda-real"
        self.assertIn("S1", rules(r))
        r = good(); r["steps"][3]["status"] = "blocked(jev-pr-28)"
        self.assertNotIn("S1", rules(r))


class Doubles(unittest.TestCase):
    def test_generated_from_observed(self):
        r = good()
        d = er.generate_doubles(r, observed={"gateway_model": "agent_roleplay", "core_version": "1.3.0"})
        kinds = {(x["part"], x["status"]) for x in d}
        self.assertIn(("scout", "agent_roleplay"), kinds)
        self.assertIn(("memory", "not_exercised"), kinds)
        self.assertIn(("gateway.model", "agent_roleplay"), kinds)
        self.assertIn(("scanner", "tps-1"), kinds)
        self.assertFalse(any(x["part"] == "gate" and x["status"] == "real" for x in d))

    def test_real_steps_not_listed_and_input_not_mutated(self):
        r = good(); r["steps"][2].update(status="real", receipt={"provider": "core"})
        before = copy.deepcopy(r)
        d = er.generate_doubles(r, observed={})
        self.assertFalse(any(x["part"] == "gate" for x in d))
        self.assertEqual(r, before)


if __name__ == "__main__":
    unittest.main()
