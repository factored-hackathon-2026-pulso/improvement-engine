"""EVT2 offline tests: the regression-suite mechanism for copilot findings (`draft_next_step`, target `prompt:p/sugerir`, agent
`copiloto-sugerencias`). No stack, no network.
    python -m unittest discover -s scripts/regression/tests -t . -v      (PyYAML needed by build_suite for the guards of other agents)

Which agent emits what (agent-core `tests/fixtures`): `copiloto-sugerencias` (mode task, flow `sugerir`, node `suggest`, prompt
`p/sugerir`) produces the typed suggestions (reply drafts, tool, escalate) the platform records as `copilot.suggestion_*`;
`copiloto-asesor` (mode conversational, prompt `p/copiloto`) only answers the advisor's questions. A draft-acceptance finding
therefore targets `p/sugerir`; the suite for it must NOT be built on `p/copiloto`.
"""
import json
import re
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import build_suite as bs  # noqa: E402
import prove_fails_on_base as pf  # noqa: E402

FIXT = HERE / "fixtures"
REJECT = json.loads((FIXT / "finding_p_draft_reject_service_quality.json").read_text(encoding="utf-8"))
TARGET = "prompt:p/sugerir"

# `Step`, `Scenario`, `ScenarioPrincipal`, `Expect`, `SuggestionExpect` of agent-core are `extra=forbid`: a key outside these is a 422.
SCENARIO_KEYS = {"id", "principal", "steps", "seed", "expect", "assertions", "sensitive_values"}
PRINCIPAL_KEYS = {"id", "attrs", "type", "subject"}
STEP_KEYS = {"op", "text", "answer", "lang", "auth", "input"}
EXPECT_KEYS = {"outcome", "actions_verified", "escalated", "suggestion_count", "suggestions"}
SUGG_KEYS = {"type", "expect", "tool", "reason_code", "language", "citations_min", "text_contains", "text_excludes"}
INPUT_REQUIRED = {"turnos", "idioma", "canal", "prioridad", "sla_estado", "espera_del_cliente_segundos"}  # copiloto-sugerencias input_schema
INPUT_ALLOWED = INPUT_REQUIRED | {"sla_minutos_restantes", "motivo_llegada", "sugerencia_borrador", "sugerencia_escalacion_aceptada",
                                  "modo_copiloto", "assistant_session_id"}


def build(finding=REJECT):
    return bs.build_suite(finding, TARGET)


class Routing(unittest.TestCase):
    def test_draft_metrics_target_the_agent_that_emits_suggestions(self):
        b = build()
        self.assertEqual(b["agent"], "copiloto-sugerencias")
        self.assertEqual(b["suite"]["agent_id"], "copiloto-sugerencias")
        self.assertEqual(b["mechanism"], "draft_next_step")
        self.assertEqual(b["target"], TARGET)

    def test_the_qa_prompt_is_refused_for_draft_metrics_with_the_reason(self):
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(REJECT, "prompt:p/copiloto")
        self.assertEqual(cm.exception.code, "metric_target_mismatch")
        self.assertIn("copiloto-sugerencias", cm.exception.why)

    def test_other_metrics_on_the_qa_prompt_stay_unsupported(self):
        f = {**REJECT, "metric": "P_TOOL_USE"}
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(f, "prompt:p/copiloto")
        self.assertEqual(cm.exception.code, "no_mechanism")

    def test_a_non_draft_metric_on_p_sugerir_is_refused(self):
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite({**REJECT, "metric": "P_TYPE_REASSIGN"}, TARGET)
        self.assertEqual(cm.exception.code, "metric_target_mismatch")

    def test_k_floor_still_refuses(self):
        f = json.loads(json.dumps(REJECT))
        f["holdout"]["denominator"] = 9
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(f, TARGET)
        self.assertEqual(cm.exception.code, "k_below_minimum")


class Shape(unittest.TestCase):
    b = build()
    by_id = {s["id"]: s for s in b["suite"]["scenarios"]}

    def test_deterministic_byte_identical(self):
        self.assertEqual(bs.dumps(build()), bs.dumps(build()))

    def test_counts_languages_and_guards(self):
        b = self.b
        self.assertTrue(bs.MIN_CASES <= len(b["finding_case_ids"]) <= bs.MAX_CASES)
        self.assertEqual(len(b["guard_case_ids"]), 3)
        ids = b["finding_case_ids"]
        self.assertTrue(any("-es-" in i for i in ids) and any("-pt-" in i for i in ids))
        self.assertEqual(len(b["suite"]["scenarios"]), len(ids) + 3)
        self.assertEqual(b["suite"]["repetitions"], 3)

    def test_every_scenario_runs_as_an_advisor_acting_on_a_customer(self):
        for s in self.b["suite"]["scenarios"]:
            p = s["principal"]
            self.assertEqual(p.get("type"), "advisor", s["id"])
            self.assertEqual(set(p["subject"]), {"kind", "ref"}, s["id"])
            self.assertEqual(p["subject"]["kind"], "customer")
            self.assertTrue(p["subject"]["ref"].startswith("cust-reg-"))
            self.assertNotEqual(p["id"], p["subject"]["ref"], "the advisor is not the customer")

    def test_keys_are_the_ones_agent_core_accepts(self):
        for s in self.b["suite"]["scenarios"]:
            self.assertLessEqual(set(s), SCENARIO_KEYS, s["id"])
            self.assertLessEqual(set(s["principal"]), PRINCIPAL_KEYS)
            self.assertTrue(re.match(r"^[a-z0-9][a-z0-9_-]*$", s["id"]), s["id"])
            for st in s["steps"]:
                self.assertLessEqual(set(st), STEP_KEYS)
            self.assertLessEqual(set(s["expect"]), EXPECT_KEYS)
            for e in s["expect"].get("suggestions", []):
                self.assertLessEqual(set(e), SUGG_KEYS, s["id"])
                self.assertIn(e["type"], ("reply", "tool", "action", "escalate"))

    def test_the_run_input_follows_the_agent_input_schema_flat(self):
        for s in self.b["suite"]["scenarios"]:
            steps = s["steps"]
            self.assertEqual(len(steps), 1, "a task agent: one start step with the input")
            self.assertEqual(steps[0]["op"], "start")
            inp = steps[0]["input"]
            self.assertTrue(INPUT_REQUIRED <= set(inp) <= INPUT_ALLOWED, (s["id"], set(inp) ^ INPUT_REQUIRED))
            self.assertIsInstance(inp["turnos"], list)
            for t in inp["turnos"]:
                self.assertEqual(set(t), {"rol", "texto", "hora"})
                self.assertIn(t["rol"], ("cliente", "analista", "asistente"))
                self.assertTrue(re.match(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d$", t["hora"]))
            self.assertIn(inp["idioma"], ("es", "pt"))
            for k, v in inp.items():  # the core accepts no object or null slot (flattened by the platform)
                if k != "turnos":
                    self.assertIsInstance(v, (str, int, bool), (s["id"], k))

    def test_every_expectation_says_something_and_has_an_outcome(self):
        for s in self.b["suite"]["scenarios"]:
            ex = s["expect"]
            self.assertEqual(ex.get("outcome"), "completed", s["id"])  # a suggestion expectation needs an outcome
            self.assertTrue(ex.get("suggestions") or ex.get("suggestion_count") is not None, s["id"])

    def test_tools_are_seeded_for_the_read_node_of_the_flow(self):
        for cid in self.b["finding_case_ids"]:
            seed = self.by_id[cid]["seed"]["tools"]
            self.assertIn("leer_productos", seed)  # the flow reads the products before it suggests
            self.assertTrue(seed["leer_productos"][0]["result"])

    def test_no_pii_shaped_text_and_no_free_ids(self):
        blob = json.dumps([[s["steps"], s.get("seed")] for cid, s in self.by_id.items() if cid in self.b["finding_case_ids"]], ensure_ascii=False)
        self.assertEqual(bs.pii_problems(blob), [])
        self.assertNotIn("http", blob)

    def test_meta_probes_and_tags(self):
        b = self.b
        self.assertEqual(b["probes"], [], "native only: the scorer reads the suggestion texts")
        for cid in b["finding_case_ids"]:
            m = b["meta"][cid]
            self.assertEqual((m["kind"], m["mechanism"], m["check"]), ("finding", "draft_next_step", "native"))
        for gid in b["guard_case_ids"]:
            self.assertEqual(b["meta"][gid]["kind"], "guard")
            self.assertTrue(gid.startswith("guard-"))
        self.assertFalse(pf.has_generated_probes(b))


class Behaviour(unittest.TestCase):
    """The suite encodes ONE pre-registered behaviour hypothesis: a draft closes with the concrete next step and who follows up."""

    b = build()
    by_id = {s["id"]: s for s in b["suite"]["scenarios"]}

    def test_finding_cases_expect_a_reply_that_names_the_follow_up(self):
        stems = {"es": "seguimiento", "pt": "acompanhamento"}
        for cid in self.b["finding_case_ids"]:
            s = self.by_id[cid]
            lang = s["steps"][0]["input"]["idioma"]
            replies = [e for e in s["expect"]["suggestions"] if e["type"] == "reply" and e.get("expect", "at_least_one") == "at_least_one"]
            self.assertEqual(len(replies), 1, cid)
            self.assertEqual(replies[0]["language"], lang)
            self.assertIn(stems[lang], replies[0]["text_contains"], cid)
            self.assertEqual(s["steps"][0]["input"].get("modo_copiloto", "drafts"), "drafts")

    def test_the_finding_cases_do_not_ask_for_escalation(self):
        for cid in self.b["finding_case_ids"]:
            esc = [e for e in self.by_id[cid]["expect"]["suggestions"] if e["type"] == "escalate"]
            self.assertTrue(esc and all(e.get("expect") == "none" for e in esc), cid)

    def test_cell_dimensions_shape_the_cases(self):
        f = json.loads(json.dumps(REJECT))
        f["dims"] = {"case_type": "unrecognized_charge", "channel": "phone_inbound"}
        b = bs.build_suite(f, TARGET)
        self.assertNotEqual(b["suite"]["id"], self.b["suite"]["id"])
        inputs = [s["steps"][0]["input"] for s in b["suite"]["scenarios"] if s["id"] in b["finding_case_ids"]]
        self.assertTrue(all(i["canal"] == "phone_inbound" for i in inputs))
        text = " ".join(t["texto"] for i in inputs for t in i["turnos"]).lower()
        self.assertTrue("cargo" in text or "cobranca" in text)

    def test_release_agent_findings_build_a_suite_too(self):
        f = json.loads(json.dumps(REJECT))
        f["dims"] = {"release": "rel-b", "agent": "copiloto-sugerencias@1.0.0"}
        b = bs.build_suite(f, TARGET)
        self.assertEqual(b["agent"], "copiloto-sugerencias")
        self.assertEqual(len(b["guard_case_ids"]), 3)

    def test_heavy_edit_findings_use_the_same_mechanism(self):
        f = {**REJECT, "metric": "P_DRAFT_HEAVY_EDIT"}
        self.assertEqual(bs.build_suite(f, TARGET)["mechanism"], "draft_next_step")

    def test_guards_protect_other_behaviours_of_the_prompt(self):
        g = {gid: self.by_id[gid] for gid in self.b["guard_case_ids"]}
        greet = [s for k, s in g.items() if "saludo" in k]
        self.assertEqual(len(greet), 1)
        self.assertEqual(greet[0]["expect"]["suggestion_count"], 0)
        tools = [s for k, s in g.items() if "modo-tools" in k]
        self.assertEqual(len(tools), 1)
        self.assertEqual(tools[0]["steps"][0]["input"]["modo_copiloto"], "tools")
        self.assertTrue(any(e["type"] == "reply" and e.get("expect") == "none" for e in tools[0]["expect"]["suggestions"]))
        pii = [s for k, s in g.items() if "pii" in k]
        self.assertEqual(len(pii), 1)
        self.assertTrue(pii[0].get("sensitive_values"))
        self.assertTrue(any(e.get("text_excludes") for e in pii[0]["expect"]["suggestions"]))
        self.assertIn(pii[0]["sensitive_values"][0], pii[0]["steps"][0]["input"]["turnos"][0]["texto"])
        for s in g.values():  # the guards must hold on the base: they never ask for the follow-up stem
            for e in s["expect"].get("suggestions", []):
                self.assertNotIn("seguimiento", e.get("text_contains", []))


def run(bundle, failing=(), attempt=None):
    ids = bundle["finding_case_ids"] + bundle["guard_case_ids"]
    per = {i: {"passed": i not in failing, "source": "native", "reason": ""} for i in ids}
    out = {"verdict": "pass" if not failing else "fail", "per_case": per,
           "failed_cases": [i for i in bundle["finding_case_ids"] if i in failing],
           "guards_failed": [i for i in bundle["guard_case_ids"] if i in failing], "gate_items": []}
    if attempt:
        out["attempt"] = attempt
    return out


class Judging(unittest.TestCase):
    """A native-only bundle goes through the same judge: the base must fail a finding case, the candidate must pass them all."""

    b = build()

    def test_coverage_names_what_is_and_is_not_measured(self):
        cov = pf.coverage(self.b, {"state": "not_applicable"})
        self.assertIn("advisor_suggestions_native", cov["native"])
        self.assertIn("real_customer_effect", cov["not_measured"])
        self.assertIn("draft_acceptance_effect", cov["not_measured"])
        self.assertIn("native_wording", cov["not_measured"])
        self.assertEqual(cov["harness_probe"], [])
        self.assertEqual(cov["assumptions"], [])

    def test_proven_when_the_base_fails_and_the_candidate_passes(self):
        f = self.b["finding_case_ids"]
        s = pf.verdict_story(self.b, run(self.b, set(f[:3])), [run(self.b, attempt=1)])
        self.assertEqual(s["outcome"], "regression_suite_proven")
        self.assertTrue(s["announce"] and s["suite_is_regression_suite"])
        self.assertEqual(s["mechanism"], "draft_next_step")
        self.assertEqual(s["agent"], "copiloto-sugerencias")

    def test_a_base_that_already_passes_is_not_a_regression_suite(self):
        s = pf.verdict_story(self.b, run(self.b), [run(self.b, attempt=1)])
        self.assertEqual(s["outcome"], "non_discriminating")
        self.assertFalse(s["announce"])

    def test_a_candidate_that_breaks_a_guard_is_not_announced(self):
        f, g = self.b["finding_case_ids"], self.b["guard_case_ids"]
        s = pf.verdict_story(self.b, run(self.b, set(f[:2])), [run(self.b, {g[1]}, 1)])
        self.assertEqual(s["outcome"], "guard_regressed")
        self.assertFalse(s["announce"])


if __name__ == "__main__":
    unittest.main()
