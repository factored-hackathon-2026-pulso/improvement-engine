"""W13 offline tests: native generated-response assertion, native binding control (PR 50 present/absent), probe-only labelling,
no-op prompt, new-agent suite (6 cases es/pt + 3 adapted guards), absent base, sample_probes keys. No stack, no network.
    cd scripts/regression && python -m unittest discover -s tests -v
"""
import copy
import json
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import build_suite as bs  # noqa: E402
import judge_story  # noqa: E402
import prove_fails_on_base as pf  # noqa: E402
import sample_probes  # noqa: E402

FIXT = HERE / "fixtures"
FINDING = json.loads((FIXT / "finding_m4_pqr_status.json").read_text(encoding="utf-8"))
BASE_ART = json.loads(pf.BASE_ARTIFACTS.read_text(encoding="utf-8"))
PROMPT = bs.build_suite(FINDING, "prompt:p/resumen_radicado")
CAND = lambda n: json.loads((FIXT / "candidates" / f"{n}.json").read_text(encoding="utf-8"))["changes"]  # noqa: E731
GOOD = {"es": "Un especialista le dara seguimiento a su caso.", "pt": "Um especialista fara o acompanhamento do seu caso."}
BAD = {"es": "Su disputa fue radicada.", "pt": "Sua contestacao foi registrada."}
FOLLOWUP = CAND("resumen_radicado_attempt2")
NOOP = CAND("resumen_radicado_noop")


def raw(failing=(), verdict=None, bundle=PROMPT):
    ids = bundle["finding_case_ids"] + bundle["guard_case_ids"]
    return {"label": "x", "proposal_id": "p1", "verdict": verdict or ("fail" if failing else "pass"), "gate_items": [],
            "per_case_native": {c: {"passed": c not in failing, "reason": "fallback" if c in failing else ""} for c in ids},
            "detail": None, "problem": None, "infra_retries": []}


def samples_for(changes, cand_samples):
    """What sample_probes would collect: `cand_samples` for the candidate texts, BAD for the base texts."""
    inputs = PROMPT["probes"][0]["inputs"]
    gen = {}
    for c in changes:
        for loc, text in c["content"]["locales"].items():
            gen[pf.probe_key(text, loc, inputs, 3)] = {"samples": [cand_samples[loc]] * 3}
    base = {a["id"]: a["locales"] for a in BASE_ART["artifacts"]}["p/resumen_radicado"]
    for loc, text in base.items():
        gen[pf.probe_key(text, loc, inputs, 3)] = {"samples": [BAD[loc]] * 3}
    return gen


class NativeAssertion(unittest.TestCase):
    def test_prompt_cases_assert_the_model_path_natively(self):
        by_id = {s["id"]: s for s in PROMPT["suite"]["scenarios"]}
        for cid in PROMPT["finding_case_ids"]:
            self.assertIn(bs.GENERATED_RESPONSE, by_id[cid]["assertions"])
            w = {x["field"]: x for x in bs.GENERATED_RESPONSE["where"]}
            self.assertIs(w["fallback_used"]["value"], False)
            self.assertEqual(w["kind"]["value"], "generated")
            self.assertEqual(PROMPT["meta"][cid]["check"], "native_generated+generated_probe")

    def test_guards_are_not_asked_for_generation(self):
        by_id = {s["id"]: s for s in PROMPT["suite"]["scenarios"]}
        for gid in PROMPT["guard_case_ids"]:
            self.assertNotIn(bs.GENERATED_RESPONSE, by_id[gid]["assertions"])


class Binding(unittest.TestCase):
    base_native = raw()["per_case_native"]

    def test_not_applicable_for_a_template_bundle(self):
        b = bs.build_suite(FINDING, "template:t/estado_pqr")
        self.assertEqual(pf.native_binding(b, {}, None)["state"], "not_applicable")

    def test_control_passing_means_candidate_bound(self):
        self.assertEqual(pf.native_binding(PROMPT, self.base_native, raw())["state"], "candidate_bound")

    def test_control_failing_the_native_assertion_means_not_bound(self):
        r = pf.native_binding(PROMPT, self.base_native, raw(failing=PROMPT["finding_case_ids"]))
        self.assertEqual(r["state"], "native_not_candidate_bound")
        self.assertEqual(len(r["control_failed_native"]), len(PROMPT["finding_case_ids"]))

    def test_a_base_that_fails_natively_proves_nothing(self):
        r = pf.native_binding(PROMPT, raw(failing=PROMPT["finding_case_ids"][:1])["per_case_native"], raw())
        self.assertEqual(r["state"], "unknown")

    def test_missing_or_infra_control_is_unknown_never_assumed(self):
        self.assertEqual(pf.native_binding(PROMPT, self.base_native, None)["state"], "unknown")
        self.assertEqual(pf.native_binding(PROMPT, self.base_native, {"verdict": "failed_infra"})["state"], "unknown")

    def test_control_changes_are_a_text_identical_bump(self):
        ctl = pf.control_changes(FOLLOWUP, BASE_ART)
        self.assertEqual(ctl[0]["content"]["version"], FOLLOWUP[0]["content"]["version"])
        base = {a["id"]: a["locales"] for a in BASE_ART["artifacts"]}["p/resumen_radicado"]
        self.assertEqual(ctl[0]["content"]["locales"], base)


class PromptVerdicts(unittest.TestCase):
    def doc(self, changes, cand_run, control_run, cand_samples=GOOD):
        d = {"bundle": PROMPT, "base": raw(), "attempts": [{"attempt": 1, "changes": changes, "run": cand_run}]}
        if control_run is not None:
            d["control"] = control_run
        d["generated"] = samples_for(changes, cand_samples)
        return d

    def test_with_pr50_a_followup_prompt_is_proven_and_announced(self):
        s = judge_story.judge(self.doc(FOLLOWUP, raw(), raw()))
        self.assertEqual((s["outcome"], s["announce"], s["suite_is_regression_suite"]), ("regression_suite_proven", True, True))
        self.assertEqual(s["native_binding"]["state"], "candidate_bound")
        self.assertIn("response_from_model_path", s["coverage"]["native"])
        self.assertIn("generated_followup", s["coverage"]["harness_probe"])
        self.assertNotIn("candidate_prompt_native", s["coverage"]["not_measured"])
        self.assertEqual(len(s["base"]["failed_cases"]), len(PROMPT["finding_case_ids"]))

    def test_with_pr50_a_noop_prompt_is_not_fixed_and_not_announced(self):
        s = judge_story.judge(self.doc(NOOP, raw(), raw(), BAD))
        self.assertEqual((s["outcome"], s["announce"]), ("not_fixed", False))

    def test_with_pr50_a_prompt_that_makes_the_model_path_fail_natively_is_not_fixed(self):
        s = judge_story.judge(self.doc(FOLLOWUP, raw(failing=PROMPT["finding_case_ids"][:2]), raw()))
        self.assertEqual((s["outcome"], s["announce"]), ("not_fixed", False))
        self.assertIn(s["attempts"][0]["per_case"][PROMPT["finding_case_ids"][0]]["source"], ("native", "native+generated_probe"))

    def test_without_pr50_the_engine_detects_it_labels_and_does_not_announce(self):
        failing = PROMPT["finding_case_ids"]  # the candidate prompt was never exercised: template fallback
        s = judge_story.judge(self.doc(FOLLOWUP, raw(failing=failing), raw(failing=failing)))
        self.assertEqual(s["native_binding"]["state"], "native_not_candidate_bound")
        self.assertEqual((s["outcome"], s["announce"], s["suite_is_regression_suite"]), ("native_not_candidate_bound", False, False))
        self.assertTrue(s["probe_only_proven"])
        self.assertIn("candidate_prompt_native", s["coverage"]["not_measured"])
        self.assertNotIn("response_from_model_path", s["coverage"]["native"])
        att = s["attempts"][0]
        self.assertEqual(att["verdict"], "probe_only_pass")
        e = att["per_case"][PROMPT["finding_case_ids"][0]]
        self.assertEqual(e["source"], "native_ignored")
        self.assertEqual(e["native_ignored"], {"passed": False, "why": "native_not_candidate_bound"})

    def test_without_pr50_a_noop_prompt_is_still_not_fixed_by_the_probe(self):
        failing = PROMPT["finding_case_ids"]
        s = judge_story.judge(self.doc(NOOP, raw(failing=failing), raw(failing=failing), BAD))
        self.assertEqual((s["outcome"], s["announce"]), ("not_fixed", False))
        self.assertFalse(s["probe_only_proven"])

    def test_unbound_guard_failures_still_count(self):
        cand_run = raw(failing=PROMPT["finding_case_ids"] + PROMPT["guard_case_ids"][:1])
        s = judge_story.judge(self.doc(FOLLOWUP, cand_run, raw(failing=PROMPT["finding_case_ids"])))
        self.assertEqual(s["outcome"], "guard_regressed")

    def test_without_a_control_the_binding_is_unknown_and_stated(self):
        s = judge_story.judge(self.doc(FOLLOWUP, raw(), None))
        self.assertEqual(s["native_binding"]["state"], "unknown")
        self.assertIn("candidate_prompt_native", s["coverage"]["not_measured"])

    def test_no_samples_collected_is_not_measured_not_a_pass(self):
        d = self.doc(FOLLOWUP, raw(), raw())
        d["generated"] = {}
        s = judge_story.judge(d)
        self.assertFalse(s["announce"])
        self.assertIn("not measured", s["base"]["per_case"][PROMPT["finding_case_ids"][0]]["reason"])

    def test_gateway_error_is_recorded_as_not_measured(self):
        d = self.doc(FOLLOWUP, raw(), raw())
        d["generated"] = {k: {"error": "http 503"} for k in d["generated"]}
        s = judge_story.judge(d)
        self.assertFalse(s["announce"])
        self.assertIn("http 503", s["base"]["per_case"][PROMPT["finding_case_ids"][0]]["reason"])


class SampleProbes(unittest.TestCase):
    def test_needed_covers_base_and_each_candidate_text_per_locale(self):
        doc = {"bundle": PROMPT, "attempts": [{"changes": FOLLOWUP}], "base_artifacts": BASE_ART}
        texts = {t for t, _p in sample_probes.needed(doc)}
        self.assertEqual(len(texts), 4)  # base es/pt + candidate es/pt
        self.assertIn(FOLLOWUP[0]["content"]["locales"]["es"], texts)

    def test_collect_requests_each_text_once_and_keeps_existing(self):
        calls = []

        def gen(text, inputs, locale, n):
            calls.append((locale, n))
            return ["x"] * n
        gen.last_error = ""
        doc = {"bundle": PROMPT, "attempts": [{"changes": FOLLOWUP}], "base_artifacts": BASE_ART}
        out = sample_probes.collect(doc, gen)
        self.assertEqual(len(calls), 4)
        again = sample_probes.collect({**doc, "generated": out}, gen)
        self.assertEqual(len(calls), 4)  # nothing re-requested
        self.assertEqual(out, again)

    def test_unreachable_gateway_is_an_error_entry(self):
        def gen(text, inputs, locale, n):
            gen.last_error = "URLError"
            return []
        out = sample_probes.collect({"bundle": PROMPT, "attempts": [], "base_artifacts": BASE_ART}, gen)
        self.assertTrue(out and all(v == {"error": "URLError"} for v in out.values()))

    def test_cli_rejects_garbage(self):
        bad = subprocess.run([sys.executable, str(HERE / "sample_probes.py")], input=b"{no", capture_output=True)
        self.assertEqual(bad.returncode, 2)


TEC = {"metric": "M1", "dims": {"reason_category": "Tecnico", "channel": "Phone", "locale": "es"},
       "discovery": {"numerator": 96, "denominator": 240}, "holdout": {"numerator": 90, "denominator": 236}, "finding_id": "f-tec"}


class NewAgentSuite(unittest.TestCase):
    def setUp(self):
        self.b = bs.build_suite(TEC, "new_agent:consultas", new_agent="soporte-tecnico")

    def test_suite_runs_on_the_new_agent_with_six_cases_es_pt_and_three_guards(self):
        b = self.b
        self.assertEqual((b["agent"], b["suite"]["agent_id"], b["mechanism"]), ("soporte-tecnico", "soporte-tecnico", "uncovered_topic"))
        self.assertEqual(len(b["finding_case_ids"]), 6)
        self.assertEqual(len(b["guard_case_ids"]), 3)
        self.assertTrue(any("-es-" in i for i in b["finding_case_ids"]) and any("-pt-" in i for i in b["finding_case_ids"]))
        self.assertEqual(b["new_agent"], {"agent_id": "soporte-tecnico", "base": "absent", "routing_measured": False})

    def test_deterministic_and_slug_checked(self):
        self.assertEqual(bs.dumps(self.b), bs.dumps(bs.build_suite(copy.deepcopy(TEC), "new_agent:consultas", new_agent="soporte-tecnico")))
        for bad in (None, "", "Soporte", "a/b", "x"):
            with self.assertRaises(bs.SuiteRefused) as cm:
                bs.build_suite(TEC, "new_agent:consultas", new_agent=bad)
            self.assertEqual(cm.exception.code, "new_agent_missing")

    def test_finding_cases_are_native_deterministic_and_toolless(self):
        by_id = {s["id"]: s for s in self.b["suite"]["scenarios"]}
        for cid in self.b["finding_case_ids"]:
            sc = by_id[cid]
            self.assertEqual(sc["expect"], {"outcome": "escalated", "escalated": True})
            self.assertIn({"event": "engine.tool_called", "expect": "none"}, sc["assertions"])
            self.assertNotIn("seed", sc)
            self.assertEqual(self.b["meta"][cid]["check"], "native")
        self.assertEqual(self.b["probes"], [])
        self.assertTrue(any("Phone" in self.b["meta"][c]["behaviour"] for c in self.b["finding_case_ids"]))

    def test_utterances_are_templated_es_pt_and_pii_free(self):
        texts = [s["steps"][1]["text"] for s in self.b["suite"]["scenarios"] if s["id"] in self.b["finding_case_ids"]]
        self.assertEqual(len(texts), len(set(texts)))
        self.assertTrue(any("aplicacion" in t for t in texts) and any("aplicativo" in t for t in texts))
        self.assertEqual(bs.pii_problems(json.dumps(texts)), [])

    def test_guards_are_the_pulso_min_consultas_guards_with_only_the_documented_adaptation(self):
        import yaml
        src = {s["id"]: s for s in yaml.safe_load(bs.GUARD_FILES["consultas"].read_text(encoding="utf-8"))["scenarios"]}
        pairs = {"guard-es-fraude-interrumpe-nuevo": "es-fraude-interrumpe", "guard-es-inyeccion-sin-herramientas": "es-inyeccion-ignora-instrucciones",
                 "guard-pt-fraude-interrompe-nuevo": "pt-fraude-interrompe"}
        by_id = {s["id"]: s for s in self.b["suite"]["scenarios"]}
        for gid, orig_id in pairs.items():
            g, o = copy.deepcopy(by_id[gid]), copy.deepcopy(src[orig_id])
            o.pop("seed", None)  # the clone has no tools: the seed is dropped
            o["id"] = gid
            if "inyeccion" in gid:  # the adaptation also asserts that no tool is called
                o["assertions"].append({"event": "engine.tool_called", "expect": "none"})
            self.assertEqual(g, o, gid)
            self.assertEqual(self.b["meta"][gid]["source"], "pulso-w13 (adapted from pulso-min)")

    def test_pulso_min_is_untouched_and_gets_no_recepcion_guards(self):
        self.assertNotIn("recepcion", bs.GUARDS)

    def test_a_k_below_minimum_finding_is_refused_for_a_new_agent_too(self):
        thin = copy.deepcopy(TEC)
        thin["holdout"]["denominator"] = 3
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(thin, "new_agent:consultas", new_agent="soporte-tecnico")
        self.assertEqual(cm.exception.code, "k_below_minimum")

    def test_other_uncovered_topics_also_build(self):
        for reason, slug in (("Queja", "intake-quejas"), ("Comercial", "soporte-comercial"), ("Retencion", "soporte-retencion")):
            f = copy.deepcopy(TEC)
            f["dims"]["reason_category"] = reason
            b = bs.build_suite(f, "new_agent:consultas", new_agent=slug)
            self.assertEqual((len(b["finding_case_ids"]), b["agent"]), (6, slug))


class PiiLintIgnoresHashIds(unittest.TestCase):
    def test_a_finding_hash_with_a_digit_run_does_not_refuse_the_suite(self):
        import re
        for n in range(96, 400):  # find a finding whose 8-hex hash holds 6 consecutive digits (about 7% of them)
            f = copy.deepcopy(TEC)
            f["discovery"]["numerator"] = n
            if re.search(r"\d{6}", bs.finding_hash(f)):
                b = bs.build_suite(f, "new_agent:consultas", new_agent="soporte-tecnico")
                self.assertEqual(len(b["finding_case_ids"]), 6)
                return
        self.fail("no such hash found in range")

    def test_utterances_are_still_linted(self):
        self.assertTrue(bs.pii_problems(json.dumps([[{"op": "turn", "text": "mi tarjeta 1234567890"}]])))


class NewAgentVerdicts(unittest.TestCase):
    def setUp(self):
        self.b = bs.build_suite(TEC, "new_agent:consultas", new_agent="soporte-tecnico")
        self.cand = [{"kind": "agent", "content": {"id": "soporte-tecnico", "version": "1.0.0"}}]
        self.base = {"label": "base", "verdict": "absent", "proposal_id": None}

    def run_(self, failing=()):
        return raw(failing, bundle=self.b)

    def judge(self, run):
        return judge_story.judge({"bundle": self.b, "base": self.base, "attempts": [{"attempt": 1, "changes": self.cand, "run": run}]})

    def test_absent_base_fails_by_absence_and_a_passing_candidate_is_proven(self):
        s = self.judge(self.run_())
        self.assertEqual((s["outcome"], s["announce"]), ("regression_suite_proven", True))
        e = s["base"]["per_case"][self.b["finding_case_ids"][0]]
        self.assertEqual((e["passed"], e["source"]), (False, "absent_on_base"))
        self.assertTrue(s["base"]["per_case"][self.b["guard_case_ids"][0]]["passed"])
        self.assertIn("absence", s["reason"])
        self.assertEqual(s["new_agent"]["agent_id"], "soporte-tecnico")
        self.assertEqual(s["native_binding"]["state"], "not_applicable")
        cov = s["coverage"]
        self.assertIn("routing_recepcion_to_new_agent", cov["not_measured"])
        self.assertIn("release_settings_assumed", cov["assumptions"], "no settings mode given: the fallback label")
        self.assertIn("new_agent_intake_and_handoff", cov["native"])

    def test_a_clone_that_inherits_the_donor_settings_drops_the_assumed_label(self):
        run = self.run_()
        s = judge_story.judge({"bundle": self.b, "base": self.base, "settings": "inherit_from",
                               "attempts": [{"attempt": 1, "changes": self.cand, "run": run}]})
        self.assertEqual(s["coverage"]["assumptions"], ["settings_inherited"])
        s = judge_story.judge({"bundle": self.b, "base": self.base, "settings": "assumed",
                               "attempts": [{"attempt": 1, "changes": self.cand, "run": run}]})
        self.assertEqual(s["coverage"]["assumptions"], ["release_settings_assumed"])

    def test_a_failing_finding_case_on_the_new_agent_is_not_fixed(self):
        s = self.judge(self.run_([self.b["finding_case_ids"][0]]))
        self.assertEqual((s["outcome"], s["announce"]), ("not_fixed", False))

    def test_a_failing_guard_on_the_new_agent_is_guard_regressed(self):
        s = self.judge(self.run_([self.b["guard_case_ids"][0]]))
        self.assertEqual((s["outcome"], s["announce"]), ("guard_regressed", False))

    def test_the_candidate_native_run_failing_infra_claims_nothing(self):
        s = self.judge({"label": "c", "verdict": "failed_infra", "per_case_native": {}})
        self.assertEqual((s["outcome"], s["announce"]), ("infra_failed", False))


if __name__ == "__main__":
    unittest.main()
