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

    def test_2b_restricted_class_and_provider_are_normalised(self):
        for dc in ("E0-derived", "e0_treated", "CSV", "Original Treated"):
            for prov in ("agent_roleplay", "Hosted", " HOSTED "):
                r = good(); r["steps"][0]["data_class"] = dc
                r["steps"][0]["receipt"] = {"provider": prov}
                self.assertIn("H2", rules(r), (dc, prov))

    def test_3_mapping_mutation(self):
        cats = {"otp_retry": 30, "refund": 12, "login": 5}
        follows_data = lambda c: max(c, key=c.get)  # noqa: E731
        self.assertEqual(er.check_mapping_mutation(follows_data, cats), [])
        keyed = lambda c: "otp_retry" if "otp_retry" in c else max(c, key=c.get)  # noqa: E731
        self.assertEqual({v["rule"] for v in er.check_mapping_mutation(keyed, cats)}, {"H3"})
        self.assertEqual(er.check_mapping_mutation(keyed, {}), [])

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

    def test_5_null_candidate_accepted_when_compile_not_exercised(self):
        r = good(); r["authors"]["candidate_created_at"] = None
        r["steps"].append({"id": "compile", "status": "not_exercised", "data_class": "generated_sample", "target": "local",
                           "sha": "a" * 40, "contract_revision": "c2-1", "host": "rust"})
        self.assertNotIn("H5", rules(r))
        r["steps"][-1]["status"] = "blocked(no-candidate)"
        self.assertNotIn("H5", rules(r))

    def test_5_null_candidate_still_rejected_otherwise(self):
        r = good(); r["authors"]["candidate_created_at"] = None  # no compile step reported
        self.assertIn("H5", rules(r))
        r["steps"].append({"id": "compile", "status": "stand-in", "data_class": "generated_sample", "target": "local",
                           "sha": "a" * 40, "contract_revision": "c2-1", "host": "rust"})
        self.assertIn("H5", rules(r))

    def test_5_null_candidate_does_not_excuse_missing_seal(self):
        r = good(); r["authors"]["candidate_created_at"] = None; del r["authors"]["suite_sealed_at"]
        r["steps"].append({"id": "compile", "status": "not_exercised", "data_class": "generated_sample", "target": "local",
                           "sha": "a" * 40, "contract_revision": "c2-1", "host": "rust"})
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


class Loopholes(unittest.TestCase):
    def test_blocked_forms_and_unhashable(self):
        for bad in ("blocked()", "blocked( )", "blocked(a b)", ["real"], None, "Real"):
            r = good(); r["steps"][3]["status"] = bad
            self.assertIn("S1", rules(r), repr(bad))

    def test_empty_steps_rejected(self):
        r = good(); r["steps"] = []
        self.assertIn("S1", rules(r))

    def test_real_needs_real_provider(self):
        for rc in ({"provider": "claude-standin"}, {}, None):
            r = good(); r["steps"][2].update(status="real", receipt=rc)
            self.assertIn("H1", rules(r), repr(rc))

    def test_author_spelling_bypass(self):
        r = good(); r["authors"]["judge"] = " Suite-X "; r["authors"]["suite"] = "suite_x"
        self.assertIn("H5", rules(r))

    def test_model_spelling_bypass(self):
        r = good(); r["steps"][1]["model"] = "Agent_Roleplay "
        self.assertIn("H7", rules(r))

    def test_data_class_case(self):
        r = good(); r["steps"][0]["data_class"] = "e0"; del r["steps"][0]["receipt"]["scanner_id"]
        self.assertIn("H2", rules(r))

    def test_doubles_lists_lying_real_step(self):
        r = good(); r["steps"][2].update(status="real", receipt={"provider": "scripted"})
        d = er.generate_doubles(r, observed={})
        self.assertTrue(any(x["part"] == "gate" for x in d))


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


def gated(verdict="fail", approval="real-narrow", publish="real-narrow", override=None):
    """A report that has the approval and publish steps and reports the gate verdict."""
    r = good()
    mk = lambda i, status: {"id": i, "status": status, "data_class": "synthetic", "target": "local", "sha": "a" * 40,
                            "contract_revision": "c2-1", "host": "rust", "receipt": {"provider": "core"}}
    r["steps"] += [mk("approval", approval), mk("publish", publish)]
    r["gate"] = {"verdict": verdict}
    if override is not None:
        r["overrides"] = [override]
    return r


OVERRIDE = {"step": "approval", "of": "gate", "verdict": "fail", "by": "human", "label": "human_override",
            "reason": "demo of the Core mechanics"}


class GateHonesty(unittest.TestCase):
    """G1: a failed gate cannot be followed by approval/publish unless an explicit labelled human override says so."""

    def test_publish_after_failed_gate_without_override_is_rejected(self):
        self.assertIn("G1", rules(gated("fail")))
        self.assertIn("G1", rules(gated("not_evaluable")))

    def test_blocked_or_unexercised_steps_after_failed_gate_are_fine(self):
        self.assertNotIn("G1", rules(gated("fail", "blocked(gate)", "blocked(gate)")))
        self.assertNotIn("G1", rules(gated("fail", "not_exercised", "not_exercised")))

    def test_approval_alone_after_failed_gate_is_rejected(self):
        self.assertIn("G1", rules(gated("fail", "real-narrow", "blocked(gate)")))

    def test_labelled_override_allows_it_only_with_quality_claims_forbidden(self):
        self.assertNotIn("G1", rules(gated("fail", override=OVERRIDE)))
        r = gated("fail", override=OVERRIDE); r["quality_claims"] = "allowed"
        self.assertIn("G1", rules(r))

    def test_override_must_name_the_gate_verdict_the_human_and_a_reason(self):
        for k, v in (("verdict", "pass"), ("by", "engine"), ("of", "safety"), ("step", "publish"), ("reason", "")):
            self.assertIn("G1", rules(gated("fail", override={**OVERRIDE, k: v})), k)

    def test_passing_gate_needs_no_override_and_a_stray_override_is_rejected(self):
        self.assertNotIn("G1", rules(gated("pass")))
        self.assertIn("G1", rules(gated("pass", override=OVERRIDE)))

    def test_exercised_publish_without_a_gate_verdict_is_rejected(self):
        r = gated("pass"); del r["gate"]
        self.assertIn("G1", rules(r))

    def test_demo0_override_must_be_labelled_simulated(self):
        r = gated("fail", override=OVERRIDE); r["label"] = "DEMO-0"
        self.assertIn("G1", rules(r))
        r["overrides"] = [{**OVERRIDE, "simulated": True}]
        self.assertNotIn("G1", rules(r))
        d = er.generate_doubles(r, observed={})
        self.assertTrue(any(x["part"] == "gate.override" and "simulated" in x["status"] for x in d))

    def test_doubles_lists_the_override(self):
        d = er.generate_doubles(gated("fail", override=OVERRIDE), observed={})
        self.assertTrue(any(x["part"] == "gate.override" and x["status"] == "human_override" for x in d))


if __name__ == "__main__":
    unittest.main()
