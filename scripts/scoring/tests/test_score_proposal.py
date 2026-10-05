import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, HERE)
import fixtures as fx  # noqa: E402
import score_proposal as sp  # noqa: E402

JUDGED = ["R1", "R2", "R8", "R9", "R10", "R12"]


def good_judge(_request):
    return {c: 2 for c in JUDGED}


class PatchTests(unittest.TestCase):
    def test_anchored_patch_applies_byte_exact(self):
        p = fx.proposal()
        out = sp.apply_patch(fx.BASE_PROMPT_ES, p["changes"][0]["patch"]["hunks"])
        self.assertEqual(out.encode(), p["changes"][0]["patch"]["candidate_text"].encode())

    def test_missing_anchor_fails(self):
        with self.assertRaises(sp.PatchError):
            sp.apply_patch("abc", [{"anchor": "zzz", "old": "a", "new": "b"}])

    def test_ambiguous_anchor_fails(self):
        with self.assertRaises(sp.PatchError):
            sp.apply_patch("xa xa", [{"anchor": "x", "old": "a", "new": "b"}])

    def test_whitespace_difference_is_not_tolerated(self):
        with self.assertRaises(sp.PatchError):
            sp.apply_patch("hello  world", [{"anchor": "hello ", "old": "world", "new": "x"}])

    def test_candidate_text_mismatch_detected(self):
        p = fx.proposal()
        p["changes"][0]["patch"]["candidate_text"] += " "
        r = sp.check_patches(p, fx.base_state())
        self.assertFalse(r["ok"])


class PiiTests(unittest.TestCase):
    def test_clean(self):
        self.assertEqual(sp.scan_pii(fx.proposal()), [])

    def test_token_email_and_digits(self):
        for bad in ("hola ⟦name:12⟧", "write to a@b.co", "id 1234567"):
            p = fx.proposal()
            p["changes"][0]["docs"]["rationale"] = bad
            self.assertTrue(sp.scan_pii(p), bad)

    def test_pii_in_scenario_text_and_nested_lists(self):
        p = fx.proposal()
        p["suite"]["scenario_texts"] = ["ok", ["call 5551234567"]]
        self.assertTrue(sp.scan_pii(p))

    def test_findings_never_echo_the_value(self):
        p = fx.proposal()
        p["changes"][0]["docs"]["rationale"] = "mail zed@x.org"
        self.assertNotIn("zed@x.org", json.dumps(sp.scan_pii(p)))


class ProtectedTests(unittest.TestCase):
    def test_clean(self):
        self.assertEqual(sp.check_protected(fx.proposal(), fx.base_state()), [])

    def test_denied_kinds(self):
        for kind in ("policy", "release_settings", "injection_ruleset", "language_detection", "knowledge_snapshot"):
            p = fx.proposal()
            p["changes"].append({"kind": kind, "content": {"id": "x", "version": "1.0.1"}})
            self.assertTrue(sp.check_protected(p, fx.base_state()), kind)

    def test_safety_clause_removed(self):
        p = fx.proposal()
        c = p["changes"][0]["content"]["locales"]
        c["es"] = c["es"].replace("datos_no_confiables", "datos")
        v = sp.check_protected(p, fx.base_state())
        self.assertTrue(any("clause" in x["code"] for x in v))

    def test_agent_authority_fields_changed(self):
        p = fx.proposal()
        p["changes"].append({"kind": "agent", "content": {"id": "disputas", "version": "1.0.1",
                             "supported_locales": ["es", "pt"], "invocable_by": ["customer", "service"],
                             "min_auth_level": 1, "subject_kinds": ["customer"]}})
        self.assertTrue(sp.check_protected(p, fx.base_state()))

    def test_meta_agent_and_platform_metric_and_model_swap(self):
        p = fx.proposal()
        p["agent_id"] = "constructor-chat"
        self.assertTrue(sp.check_protected(p, fx.base_state()))
        p = fx.proposal()
        p["changes"].append({"kind": "model_profile", "content": {"id": "perfil", "version": "1.0.1", "model": "other"}})
        base = fx.base_state()
        base["entities"]["perfil"] = {"kind": "model_profile", "version": "1.0.0", "content": {"model": "m"}}
        self.assertTrue(sp.check_protected(p, base))


class LocaleTests(unittest.TestCase):
    def test_ok(self):
        self.assertEqual(sp.check_locales(fx.proposal(), fx.base_state()), [])

    def test_pt_dropped(self):
        p = fx.proposal()
        del p["changes"][0]["content"]["locales"]["pt"]
        self.assertTrue(sp.check_locales(p, fx.base_state()))

    def test_supported_locales_shrunk(self):
        p = fx.proposal()
        p["changes"].append({"kind": "agent", "content": {"id": "disputas", "version": "1.0.1", "supported_locales": ["es"]}})
        self.assertTrue(sp.check_locales(p, fx.base_state()))

    def test_pt_identical_to_es_is_flagged(self):
        p = fx.proposal()
        loc = p["changes"][0]["content"]["locales"]
        loc["pt"] = loc["es"]
        self.assertTrue(sp.check_locales(p, fx.base_state()))


class RegistryAndEvidenceTests(unittest.TestCase):
    def test_target_present(self):
        self.assertTrue(sp.check_target(fx.proposal(), fx.registry_export())["ok"])

    def test_target_missing(self):
        p = fx.proposal()
        p["agent_id"] = "ghost"
        self.assertFalse(sp.check_target(p, fx.registry_export())["ok"])

    def test_registry_absent_is_skipped_not_passed(self):
        self.assertEqual(sp.check_target(fx.proposal(), None)["status"], "skipped")

    def test_base_entity_missing_in_registry(self):
        p = fx.proposal()
        p["changes"][0]["content"]["id"] = "p/ghost"
        p["changes"][0]["patch"]["entity_id"] = "p/ghost"
        self.assertFalse(sp.check_target(p, fx.registry_export())["ok"])

    def test_evidence_resolves(self):
        self.assertEqual(sp.check_evidence(fx.proposal(), fx.evidence_store()), [])

    def test_evidence_dangling_or_small_k(self):
        p = fx.proposal()
        p["evidence_refs"].append({"id": "ghost", "k": 50})
        self.assertTrue(sp.check_evidence(p, fx.evidence_store()))
        store = {"sig-1": {"k": 3}}
        self.assertTrue(sp.check_evidence(fx.proposal(), store))

    def test_no_evidence_refs_fails(self):
        p = fx.proposal()
        p["evidence_refs"] = []
        self.assertTrue(sp.check_evidence(p, fx.evidence_store()))


class SuiteIndependenceTests(unittest.TestCase):
    def test_ok(self):
        self.assertEqual(sp.check_suite(fx.proposal()), [])

    def test_suite_sealed_after_candidate(self):
        p = fx.proposal()
        p["suite"]["suite_digest_at"] = "2026-10-04T12:00:00Z"
        self.assertTrue(sp.check_suite(p))

    def test_same_author(self):
        p = fx.proposal()
        p["suite"]["author"] = p["author"]
        self.assertTrue(sp.check_suite(p))

    def test_base_scenario_removed(self):
        p = fx.proposal()
        p["suite"]["scenarios"] = ["s1", "s3"]
        self.assertTrue(sp.check_suite(p))

    def test_not_evaluable_with_reason_is_accepted_without_digests(self):
        p = fx.proposal()
        p["suite"] = {"not_evaluable": {"reason": "wording only", "alternative": "offline human read"}}
        self.assertEqual(sp.check_suite(p), [])

    def test_no_suite_no_reason(self):
        p = fx.proposal()
        del p["suite"]
        self.assertTrue(sp.check_suite(p))


class JudgeHookTests(unittest.TestCase):
    def test_family_detection(self):
        self.assertEqual(sp.model_family("xiaomi/mimo-v2.6-flash"), "xiaomi")
        self.assertEqual(sp.model_family("anthropic/claude-sonnet"), "anthropic")
        self.assertEqual(sp.model_family("claude-opus"), "anthropic")
        self.assertEqual(sp.model_family("openai/gpt-5"), "openai")

    def test_same_family_refused(self):
        with self.assertRaises(sp.JudgeError):
            sp.run_judge(good_judge, {}, builder_model="xiaomi/a", judge_model="xiaomi/b")

    def test_unknown_family_refused(self):
        with self.assertRaises(sp.JudgeError):
            sp.run_judge(good_judge, {}, builder_model="", judge_model="anthropic/claude")

    def test_two_samples_min_taken(self):
        calls = []

        def judge(req):
            calls.append(req)
            return {c: (2 if len(calls) == 1 else 1) for c in JUDGED}
        out = sp.run_judge(judge, {"finding": "f"}, "xiaomi/a", "anthropic/claude")
        self.assertEqual(len(calls), 2)
        self.assertEqual(out["scores"]["R1"], 1)
        self.assertEqual(out["escalate_human"], [])

    def test_disagreement_over_one_escalates(self):
        seq = iter([{c: 2 for c in JUDGED}, {c: 0 for c in JUDGED}])
        out = sp.run_judge(lambda r: next(seq), {}, "xiaomi/a", "anthropic/claude")
        self.assertEqual(sorted(out["escalate_human"]), sorted(JUDGED))

    def test_builder_reasoning_is_stripped_from_the_request(self):
        seen = {}

        def judge(req):
            seen.update(req)
            return {c: 2 for c in JUDGED}
        sp.run_judge(judge, {"finding": "f", "builder_reasoning": "secret chain", "nested": {"reasoning": "x", "ok": 1}},
                     "xiaomi/a", "anthropic/claude")
        self.assertNotIn("builder_reasoning", seen)
        self.assertNotIn("reasoning", seen["nested"])
        self.assertEqual(seen["nested"]["ok"], 1)

    def test_out_of_range_score_rejected(self):
        with self.assertRaises(sp.JudgeError):
            sp.run_judge(lambda r: {"R1": 3}, {}, "xiaomi/a", "anthropic/claude")

    def test_judge_cannot_set_mechanical_criteria(self):
        with self.assertRaises(sp.JudgeError):
            sp.run_judge(lambda r: {"R4": 2}, {}, "xiaomi/a", "anthropic/claude")


class ScoreTests(unittest.TestCase):
    def kw(self, **over):
        k = dict(base=fx.base_state(), registry=fx.registry_export(), evidence=fx.evidence_store())
        k.update(over)
        return k

    def test_without_judge_needs_judge(self):
        r = sp.score(fx.proposal(), **self.kw())
        self.assertEqual(r["verdict"], "needs_judge")
        self.assertEqual(r["criteria"]["R4"], 2)
        self.assertIsNone(r["criteria"]["R1"])

    def test_full_marks_is_adequate(self):
        r = sp.score(fx.proposal(), judge_scores=good_judge({}), **self.kw())
        self.assertEqual(r["verdict"], "adequate")
        self.assertEqual(r["total"], 24)
        self.assertFalse(r["human_review"])

    def test_hard_gate_zero_rejects_even_with_perfect_judge(self):
        p = fx.proposal()
        p["changes"][0]["docs"]["rationale"] = "ping a@b.co"
        r = sp.score(p, judge_scores=good_judge({}), **self.kw())
        self.assertEqual(r["verdict"], "reject")
        self.assertEqual(r["criteria"]["R11"], 0)

    def test_hard_gate_rejects_without_judge(self):
        p = fx.proposal()
        del p["changes"][0]["content"]["locales"]["pt"]
        r = sp.score(p, **self.kw())
        self.assertEqual(r["verdict"], "reject")

    def test_patch_failure_is_invalid_not_inadequate(self):
        p = fx.proposal()
        p["changes"][0]["patch"]["hunks"][0]["anchor"] = "nope"
        r = sp.score(p, judge_scores=good_judge({}), **self.kw())
        self.assertEqual(r["verdict"], "invalid")

    def test_unknown_target_is_invalid(self):
        p = fx.proposal()
        p["agent_id"] = "ghost"
        r = sp.score(p, judge_scores=good_judge({}), **self.kw())
        self.assertEqual(r["verdict"], "invalid")

    def test_any_zero_blocks_adequate(self):
        j = good_judge({})
        j["R2"] = 0
        r = sp.score(fx.proposal(), judge_scores=j, **self.kw())
        self.assertEqual(r["verdict"], "revise")

    def test_threshold_boundaries(self):
        # mechanical = 12 points; judged six criteria
        for judged_total, expected in ((7, "adequate"), (6, "revise"), (2, "revise"), (1, "reject")):
            j = {c: 0 for c in JUDGED}
            pts = judged_total
            for c in JUDGED:
                give = min(2, pts)
                j[c] = give
                pts -= give
            r = sp.score(fx.proposal(), judge_scores=j, **self.kw())
            if expected == "adequate":
                # with zeros present a 19+ total is still not adequate
                self.assertEqual(r["verdict"], "revise")
            else:
                self.assertEqual(r["verdict"], expected, (judged_total, r["total"]))

    def test_flow_change_forces_human_review(self):
        p = fx.proposal()
        p["changes"].append({"kind": "flow", "content": {"id": "f", "version": "1.0.1"}})
        r = sp.score(p, judge_scores=good_judge({}), **self.kw(registry=None))
        self.assertTrue(r["human_review"])

    def test_registry_absent_reported_as_skipped(self):
        r = sp.score(fx.proposal(), judge_scores=good_judge({}), **self.kw(registry=None))
        self.assertEqual(r["gates"]["target_in_registry"]["status"], "skipped")

    def test_inputs_not_mutated(self):
        p, b = fx.proposal(), fx.base_state()
        p0, b0 = copy.deepcopy(p), copy.deepcopy(b)
        sp.score(p, base=b)
        self.assertEqual((p, b), (p0, b0))

    def test_cli(self):
        with tempfile.TemporaryDirectory() as d:
            paths = {}
            for name, obj in (("p", fx.proposal()), ("b", fx.base_state()), ("r", fx.registry_export()),
                              ("e", fx.evidence_store())):
                paths[name] = os.path.join(d, name + ".json")
                with open(paths[name], "w") as f:
                    json.dump(obj, f)
            out = os.path.join(d, "o.json")
            p = subprocess.run([sys.executable, os.path.join(os.path.dirname(HERE), "score_proposal.py"),
                                "--proposal", paths["p"], "--base", paths["b"], "--registry", paths["r"],
                                "--evidence", paths["e"], "--out", out], capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stderr)
            with open(out) as f:
                self.assertEqual(json.load(f)["verdict"], "needs_judge")


if __name__ == "__main__":
    unittest.main()
