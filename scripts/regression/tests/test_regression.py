"""Offline tests of build_suite / prove_fails_on_base (no stack). From the repo root:
    uv run --with pyyaml python -m unittest discover -s scripts/regression/tests -t . -v
"""
import copy
import json
import re
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(ROOT))
import build_suite as bs  # noqa: E402
import prove_fails_on_base as pf  # noqa: E402

FIXT = ROOT / "fixtures"
FINDING = json.loads((FIXT / "finding_m4_pqr_status.json").read_text(encoding="utf-8"))
BASE_ART = json.loads(pf.BASE_ARTIFACTS.read_text(encoding="utf-8"))
TARGETS = ("template:t/estado_pqr", "prompt:p/resumen_radicado", "new_agent:consultas")


class Build(unittest.TestCase):
    def test_deterministic_byte_identical(self):
        for t in TARGETS[:2]:
            self.assertEqual(bs.dumps(bs.build_suite(FINDING, t)), bs.dumps(bs.build_suite(copy.deepcopy(FINDING), t)))

    def test_hash_depends_on_finding(self):
        other = copy.deepcopy(FINDING)
        other["dims"]["channel"] = "Chat"
        a, b = bs.build_suite(FINDING, TARGETS[0]), bs.build_suite(other, TARGETS[0])
        self.assertNotEqual(a["suite"]["id"], b["suite"]["id"])

    def test_counts_languages_and_guards(self):
        for t in TARGETS[:2]:  # new_agent/recepcion has no pulso-min guards: refused (test_no_guards_refused_not_faked)
            b = bs.build_suite(FINDING, t)
            self.assertTrue(6 <= len(b["finding_case_ids"]) <= 10, t)
            self.assertEqual(len(b["guard_case_ids"]), 3)
            ids = [s["id"] for s in b["suite"]["scenarios"]]
            self.assertEqual(len(ids), len(set(ids)))
            self.assertTrue(any("-es-" in i for i in b["finding_case_ids"]))
            self.assertTrue(any("-pt-" in i for i in b["finding_case_ids"]))
            self.assertEqual(b["suite"]["thresholds"], {})

    def test_every_case_tagged(self):
        b = bs.build_suite(FINDING, TARGETS[0])
        for s in b["suite"]["scenarios"]:
            m = b["meta"][s["id"]]
            self.assertEqual(m["finding_key"], b["finding_key"])
            self.assertTrue(m["behaviour"])
            self.assertIn(m["kind"], ("finding", "guard"))

    def test_guards_are_verbatim_pulso_min(self):
        import yaml
        for t, agent in ((TARGETS[0], "consultas"), (TARGETS[1], "disputas")):
            b = bs.build_suite(FINDING, t)
            src = {s["id"]: s for s in yaml.safe_load(bs.GUARD_FILES[agent].read_text(encoding="utf-8"))["scenarios"]}
            for g in (s for s in b["suite"]["scenarios"] if s["id"].startswith("guard-")):
                orig = copy.deepcopy(src[g["id"][len("guard-"):]])
                orig["id"] = g["id"]
                self.assertEqual(g, orig)

    def test_no_pii_shaped_text(self):
        for t in TARGETS[:2]:
            b = bs.build_suite(FINDING, t)
            text = bs.dumps(b)
            self.assertIsNone(re.search(r"\d{6,}", text), t)
            self.assertIsNone(re.search(r"[\w.]+@[\w.]+\.\w+", text), t)
            cases = [s for s in b["suite"]["scenarios"] if not s["id"].startswith("guard-")]
            self.assertEqual(bs.pii_problems(json.dumps(cases)), [])

    def test_pii_lint_catches(self):
        self.assertTrue(bs.pii_problems("mi cuenta 1234567"))
        self.assertTrue(bs.pii_problems("ana@example.com"))

    def test_k_floor_refuses(self):
        thin = copy.deepcopy(FINDING)
        thin["holdout"]["denominator"] = bs.K_MIN - 1
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(thin, TARGETS[0])
        self.assertEqual(cm.exception.code, "k_below_minimum")

    def test_unknown_target_refused(self):
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(FINDING, "flow:consulta-pqr")
        self.assertEqual(cm.exception.code, "no_mechanism")

    def test_no_guards_refused_not_faked(self):
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(FINDING, "new_agent:consultas")
        self.assertEqual(cm.exception.code, "no_guards_for_agent")

    def test_truncation_keeps_both_locales(self):
        b = bs.build_suite(FINDING, TARGETS[0], max_finding_cases=6)
        self.assertEqual(len(b["finding_case_ids"]), 6)
        self.assertTrue(any("-pt-" in i for i in b["finding_case_ids"]) and any("-es-" in i for i in b["finding_case_ids"]))
        self.assertEqual({p["case_id"] for p in b["probes"]}, set(b["finding_case_ids"]))


class Probes(unittest.TestCase):
    def setUp(self):
        self.b = bs.build_suite(FINDING, "template:t/estado_pqr")

    def native_ok(self):
        return {i: {"passed": True} for i in self.b["finding_case_ids"] + self.b["guard_case_ids"]}

    def test_static_base_sentence_fails_every_probe(self):
        r = pf.case_results(self.b, self.native_ok(), [], BASE_ART)
        self.assertEqual(set(pf.failed(r, self.b["finding_case_ids"])), set(self.b["finding_case_ids"]))
        self.assertEqual(pf.failed(r, self.b["guard_case_ids"]), [])

    def candidate(self, es, pt):
        return [{"kind": "template", "content": {"id": "t/estado_pqr", "version": "1.0.1", "locales": {"es": es, "pt": pt}}}]

    def test_state_placeholder_passes_probe(self):
        c = self.candidate("Tu PQR esta en estado {{ facts.pqr.value.status }}.",
                           "Sua solicitacao esta no estado {{ facts.pqr.value.status }}.")
        r = pf.case_results(self.b, self.native_ok(), c, BASE_ART)
        self.assertEqual(pf.failed(r, self.b["finding_case_ids"]), [])

    def test_unknown_placeholder_is_a_render_error(self):
        c = self.candidate("Tu PQR: {{ facts.pqr.value.estado }}.", "Sua solicitacao: {{ facts.pqr.value.estado }}.")
        r = pf.case_results(self.b, self.native_ok(), c, BASE_ART)
        self.assertEqual(len(pf.failed(r, self.b["finding_case_ids"])), len(self.b["finding_case_ids"]))
        self.assertIn("render_error", r[self.b["finding_case_ids"][0]]["reason"])

    def test_native_failure_alone_fails_case(self):
        n = self.native_ok()
        n[self.b["finding_case_ids"][0]] = {"passed": False, "reason": "outcome"}
        c = self.candidate("Estado: {{ facts.pqr.value.status }}", "Estado: {{ facts.pqr.value.status }}")
        r = pf.case_results(self.b, n, c, BASE_ART)
        self.assertEqual(pf.failed(r, self.b["finding_case_ids"]), [self.b["finding_case_ids"][0]])


class GeneratedProbes(unittest.TestCase):
    def setUp(self):
        self.b = bs.build_suite(FINDING, "prompt:p/resumen_radicado")
        self.native = {i: {"passed": True} for i in self.b["finding_case_ids"] + self.b["guard_case_ids"]}

    def test_probe_shape(self):
        self.assertEqual({p["case_id"] for p in self.b["probes"]}, set(self.b["finding_case_ids"]))
        self.assertTrue(all(p["kind"] == "generated_contains" and p["samples"] >= 3 for p in self.b["probes"]))

    def test_generator_that_never_follows_up_fails_base(self):
        r = pf.case_results(self.b, self.native, [], BASE_ART, generate=lambda t, i, l, n: ["Tu disputa fue radicada."] * n)
        self.assertEqual(len(pf.failed(r, self.b["finding_case_ids"])), len(self.b["finding_case_ids"]))
        self.assertEqual(r[self.b["finding_case_ids"][0]]["source"], "generated_probe")

    def test_one_bad_sample_fails_the_case(self):
        def gen(t, i, loc, n):
            return ["Un especialista le dara seguimiento."] * (n - 1) + ["Nada mas."]
        r = pf.case_results(self.b, self.native, [], BASE_ART, generate=gen)
        self.assertEqual(len(pf.failed(r, self.b["finding_case_ids"])), len(self.b["finding_case_ids"]))

    def test_digit_in_sample_fails(self):
        r = pf.case_results(self.b, self.native, [], BASE_ART,
                            generate=lambda t, i, l, n: ["Un especialista le dara seguimiento en 3 dias; acompanhamento."] * n)
        self.assertIn("digit", r[self.b["finding_case_ids"][0]]["reason"])

    def test_no_generator_is_not_a_pass(self):
        r = pf.case_results(self.b, self.native, [], BASE_ART)
        self.assertEqual(len(pf.failed(r, self.b["finding_case_ids"])), len(self.b["finding_case_ids"]))
        self.assertIn("not measured", r[self.b["finding_case_ids"][0]]["reason"])

    def test_candidate_prompt_text_is_what_the_generator_sees(self):
        seen = []

        def gen(t, i, loc, n):
            seen.append(t)
            return ["Un especialista le dara seguimiento; acompanhamento do especialista."] * n
        cand = [{"kind": "prompt", "content": {"id": "p/resumen_radicado", "version": "1.0.1",
                                               "locales": {"es": "ES NUEVO", "pt": "PT NOVO"}}}]
        r = pf.case_results(self.b, self.native, cand, BASE_ART, generate=gen)
        self.assertEqual(pf.failed(r, self.b["finding_case_ids"]), [])
        self.assertEqual(set(seen), {"ES NUEVO", "PT NOVO"})  # sampled once per locale (cache), with the candidate text


def run(bundle, failing=(), attempt=None):
    ids = bundle["finding_case_ids"] + bundle["guard_case_ids"]
    per = {i: {"passed": i not in failing, "source": "native", "reason": ""} for i in ids}
    out = {"verdict": "pass" if not failing else "fail", "per_case": per,
           "failed_cases": [i for i in bundle["finding_case_ids"] if i in failing],
           "guards_failed": [i for i in bundle["guard_case_ids"] if i in failing], "gate_items": []}
    if attempt:
        out["attempt"] = attempt
    return out


class Decide(unittest.TestCase):
    def setUp(self):
        self.b = bs.build_suite(FINDING, "prompt:p/resumen_radicado")
        self.f = self.b["finding_case_ids"]
        self.g = self.b["guard_case_ids"]

    def test_non_discriminating(self):
        s = pf.verdict_story(self.b, run(self.b), [run(self.b, attempt=1)])
        self.assertEqual(s["outcome"], "non_discriminating")
        self.assertFalse(s["suite_is_regression_suite"])
        self.assertFalse(s["announce"])

    def test_candidate_always_clean_stays_non_discriminating(self):
        s = pf.verdict_story(self.b, run(self.b), [run(self.b, attempt=1)])
        self.assertIn("--candidate-always", s["reason"])
        self.assertFalse(s["announce"] or s["suite_is_regression_suite"])

    def test_candidate_always_catches_candidate_regression(self):
        s = pf.verdict_story(self.b, run(self.b), [run(self.b, {self.f[0]}, 1)])
        self.assertEqual(s["outcome"], "guard_regressed")
        self.assertFalse(s["announce"])

    def test_candidate_flag_wired(self):
        import inspect
        src = inspect.getsource(pf.main)
        self.assertIn("--candidate-always", src)
        self.assertIn("a.candidate_always", src)

    def test_proven(self):
        s = pf.verdict_story(self.b, run(self.b, {self.f[0]}), [run(self.b, attempt=1)])
        self.assertEqual(s["outcome"], "regression_suite_proven")
        self.assertTrue(s["suite_is_regression_suite"] and s["announce"])
        self.assertEqual(set(s["story_text"]), {"es", "pt"})

    def test_attempt1_fails_attempt2_passes_story(self):
        s = pf.verdict_story(self.b, run(self.b, {self.f[0], self.f[1]}),
                             [run(self.b, {self.f[1]}, 1), run(self.b, attempt=2)])
        self.assertEqual(s["outcome"], "regression_suite_proven")
        self.assertIn("intento 1 fallo", s["story_text"]["es"])
        self.assertIn("intento 2 paso", s["story_text"]["es"])
        self.assertIn("tentativa 2 passou", s["story_text"]["pt"])
        self.assertTrue(s["announce"])

    def test_not_fixed_not_announced(self):
        s = pf.verdict_story(self.b, run(self.b, {self.f[0]}), [run(self.b, {self.f[0]}, 1)])
        self.assertEqual(s["outcome"], "not_fixed")
        self.assertFalse(s["announce"])

    def test_guard_regression_on_candidate(self):
        s = pf.verdict_story(self.b, run(self.b, {self.f[0]}), [run(self.b, {self.g[0]}, 1)])
        self.assertEqual(s["outcome"], "guard_regressed")
        self.assertFalse(s["announce"])

    def test_guard_failing_on_base(self):
        s = pf.verdict_story(self.b, run(self.b, {self.f[0], self.g[1]}), [run(self.b, attempt=1)])
        self.assertEqual(s["outcome"], "guard_regressed")

    def test_infra_failure_claims_nothing(self):
        bad = {"verdict": "failed_infra", "per_case": {}, "failed_cases": [], "guards_failed": [], "gate_items": []}
        s = pf.verdict_story(self.b, bad, [])
        self.assertEqual(s["outcome"], "infra_failed")
        self.assertFalse(s["announce"])

    def test_native_report_parse(self):
        rep = {"runs": {"cand_on_new": {"scenarios": {"a": True, "b": False}}},
               "results": [{"scenario_id": "b", "run": "cand_on_new",
                            "score": {"passed": False, "failures": ["outcome != resolved"]}}]}
        n = pf.native_from_report(rep, ["a", "b", "c"])
        self.assertEqual(n["a"]["passed"], True)
        self.assertEqual(n["b"], {"passed": False, "reason": "outcome != resolved"})
        self.assertNotIn("c", n)  # not measured -> case_results marks it failed
        r = pf.case_results(self.b, {}, [], BASE_ART)
        self.assertTrue(all(not v["passed"] for v in r.values()))


if __name__ == "__main__":
    unittest.main()
