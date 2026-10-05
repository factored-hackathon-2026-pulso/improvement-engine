"""Structural safety contract for synthetic Codex bank evaluation drafts.

These tests do not execute an Agent Core runtime or establish policy correctness.
"""

from __future__ import annotations

import copy
import json
import re
import sys
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).parent
sys.path.insert(0, str(ROOT))
from validate_eval_suites import validate_suite_document


AGENT_CORE = Path(r"D:\.codex\factored\references\agent-core-c814c2b")
SUITE_IDS = ("disputas", "consultas", "recepcion", "copiloto-asesor")
OUTCOMES = {
    "resolved", "abstained", "cancelled", "clarify_exhausted", "completed",
    "failed", "abandoned", "escalated", "transferred",
}
EMAIL_RE = re.compile(r"[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}", re.IGNORECASE)
LONG_DIGIT_RE = re.compile(r"\d{6,}")
PHONE_RE = re.compile(r"(?<!\w)\+?\d[\d ().-]{7,}\d(?!\w)")


def read_suite(suite_id: str) -> dict:
    path = ROOT / f"{suite_id}@1.0.0.yaml"
    value = yaml.safe_load(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise AssertionError(f"{path.name} must contain a mapping")
    return value


class SyntheticEvalSuiteContractTests(unittest.TestCase):
    def test_four_suites_have_twenty_unique_bilingual_scenarios(self) -> None:
        self.assertEqual(
            {path.name for path in ROOT.glob("*@1.0.0.yaml")},
            {f"{suite_id}@1.0.0.yaml" for suite_id in SUITE_IDS},
        )
        for suite_id in SUITE_IDS:
            with self.subTest(suite=suite_id):
                raw = read_suite(suite_id)
                validate_suite_document(raw, expected_id=suite_id)
                self.assertEqual(raw.get("id"), suite_id)
                self.assertEqual(raw.get("agent_id"), suite_id)
                scenarios = raw.get("scenarios")
                self.assertIsInstance(scenarios, list)
                self.assertGreaterEqual(len(scenarios), 20)
                self.assertLessEqual(len(scenarios), 30)
                scenario_ids = [scenario.get("id") for scenario in scenarios]
                self.assertTrue(all(isinstance(value, str) and value for value in scenario_ids))
                self.assertEqual(len(scenario_ids), len(set(scenario_ids)))

                for lang in ("es", "pt"):
                    localized = [scenario for scenario in scenarios if scenario["id"].startswith(
                        (f"happy-{lang}-", f"protected-{lang}-", f"negative-{lang}-"))]
                    self.assertGreaterEqual(len(localized), 10)
                    self.assertLessEqual(len(localized), 15)
                    for case_type, minimum in (("happy", 1), ("protected", 2), ("negative", 1)):
                        self.assertGreaterEqual(
                            sum(scenario["id"].startswith(f"{case_type}-{lang}-")
                                for scenario in localized),
                            minimum,
                        )

    def test_suites_use_only_synthetic_principals_and_safe_sensitive_markers(self) -> None:
        for suite_id in SUITE_IDS:
            with self.subTest(suite=suite_id):
                raw = read_suite(suite_id)
                serialized = json.dumps(raw, ensure_ascii=False, sort_keys=True)
                self.assertIsNone(EMAIL_RE.search(serialized))
                self.assertIsNone(LONG_DIGIT_RE.search(serialized))
                self.assertIsNone(PHONE_RE.search(serialized))
                for scenario in raw["scenarios"]:
                    principal_id = scenario["principal"]["id"]
                    self.assertRegex(principal_id, r"^synthetic-[a-z0-9-]+$")
                    for marker in scenario.get("sensitive_values", []):
                        self.assertRegex(marker, r"^SYNTH-[A-Z0-9-]+$")
                        turns = [
                            step.get("text", "") for step in scenario.get("steps", [])
                            if step.get("op") == "turn"
                        ]
                        self.assertTrue(any(marker in text for text in turns))
                    expected = scenario.get("expect", {})
                    outcome = expected.get("outcome")
                    if outcome is not None:
                        self.assertIn(outcome, OUTCOMES)
                    if expected.get("escalated") is not None:
                        self.assertIsInstance(expected["escalated"], bool)

    def test_escalation_expectation_matches_core_event_semantics(self) -> None:
        for suite_id in SUITE_IDS:
            with self.subTest(suite=suite_id):
                for scenario in read_suite(suite_id)["scenarios"]:
                    expected = scenario["expect"]
                    outcome = expected.get("outcome")
                    # Core scoring sets escalated only when an Escalated event is
                    # emitted; a transfer/routing outcome is not that event.
                    if outcome is not None and expected.get("escalated") is not None:
                        self.assertEqual(expected["escalated"], outcome == "escalated")

    def test_live_candidate_scenarios_have_required_tool_and_confirmation_setup(self) -> None:
        disputes = read_suite("disputas")
        for scenario in disputes["scenarios"]:
            if scenario["id"] not in {"happy-es-cargo-duplicado", "happy-pt-compra-duplicada"}:
                continue
            with self.subTest(scenario=scenario["id"]):
                self.assertIn("tools", scenario.get("seed", {}))
                self.assertTrue(any(step.get("op") == "confirm" for step in scenario["steps"]))
                text = " ".join(step.get("text", "") for step in scenario["steps"])
                self.assertRegex(text, r"\b\d+\b")

    def test_consultas_has_one_seeded_pqr_status_control_per_language(self) -> None:
        suite = read_suite("consultas")
        for lang in ("es", "pt"):
            matches = [
                scenario for scenario in suite["scenarios"]
                if scenario["id"] == f"happy-{lang}-estado-pqr-verificado"
            ]
            self.assertEqual(len(matches), 1, f"expected one {lang} PQR control")
            scenario = matches[0]
            self.assertEqual(
                scenario["expect"], {"outcome": "resolved", "escalated": False}
            )
            self.assertIn("obtener_pqr", scenario.get("seed", {}).get("tools", {}))
            turns = [step for step in scenario["steps"] if step.get("op") == "turn"]
            self.assertEqual(len(turns), 1)
            self.assertIn("pqr-synthetic", turns[0]["text"].lower())
            assertions = scenario.get("assertions", [])
            self.assertIn(
                {
                    "event": "engine.tool_called",
                    "where": [{"field": "status", "op": "eq", "value": "ok"}],
                    "expect": "at_least_one",
                },
                assertions,
            )
            self.assertIn(
                {"event": "engine.response_failed", "expect": "none"}, assertions
            )
            self.assertIn(
                {"event": "engine.escalated", "expect": "none"}, assertions
            )

    def test_clarification_cases_stop_before_or_at_terminal_turn(self) -> None:
        for suite_id in SUITE_IDS:
            for scenario in read_suite(suite_id)["scenarios"]:
                if "consulta-vaga" in scenario["id"] or "radicado-malformado" in scenario["id"] \
                        or "sem-detalhe" in scenario["id"] or "sem-detalhes" in scenario["id"] \
                        or "mensaje-vacio" in scenario["id"] or "mensagem-vazia" in scenario["id"]:
                    self.assertLessEqual(
                        sum(step.get("op") == "turn" for step in scenario["steps"]),
                        2,
                        f"{suite_id}/{scenario['id']} must not send a turn after closure",
                    )
                    self.assertEqual(scenario["expect"], {"outcome": "escalated", "escalated": True})

    def test_copilot_suite_marks_advisor_role_as_synthetic_metadata(self) -> None:
        # `role` is an arbitrary ScenarioPrincipal attr, not its principal type.
        # This checks draft labeling only, not an advisor-authenticated run.
        for scenario in read_suite("copiloto-asesor")["scenarios"]:
            self.assertEqual(scenario["principal"].get("attrs", {}).get("role"), "advisor")

    def test_readme_does_not_conflate_registered_agents_with_unbound_eval_capabilities(self) -> None:
        readme = (ROOT / "README.md").read_text(encoding="utf-8").lower()
        self.assertNotIn("no matching agent asset is currently bound", readme)
        self.assertIn("native `evaluate` harness", readme)
        self.assertIn("cannot bind a knowledge source", readme)
        self.assertIn("transfer directory", readme)

    def test_readme_reports_live_eval_counts_and_vacuous_passes_honestly(self) -> None:
        readme = (ROOT / "README.md").read_text(encoding="utf-8").lower()
        for observed in (
            "`disputas` | 28 | 26 / 2 / 0",
            "`consultas` | 28 | 25 / 1 / 2",
            "`recepcion` | 28 | 28 / 0 / 0",
            "`copiloto-asesor` | 22 | 20 / 2 / 0",
            "69 were vacuous",
            "http 410",
        ):
            with self.subTest(observed=observed):
                self.assertIn(observed, readme)
        self.assertIn("it is not evidence", readme)

    def test_unverified_sensitive_reference_cases_do_not_assume_escalation(self) -> None:
        for suite_id in ("disputas", "consultas"):
            for scenario in read_suite(suite_id)["scenarios"]:
                if not scenario["id"].startswith("protected-"):
                    continue
                if "monto-superior-ambos-limites" in scenario["id"]:
                    continue
                with self.subTest(suite=suite_id, scenario=scenario["id"]):
                    self.assertEqual(scenario["expect"], {})

    def test_sensitive_reference_event_canaries_are_present_per_locale(self) -> None:
        for suite_id in ("disputas", "consultas"):
            suite = read_suite(suite_id)
            for lang in ("es", "pt"):
                with self.subTest(suite=suite_id, locale=lang):
                    cases = [
                        scenario for scenario in suite["scenarios"]
                        if scenario["id"].startswith(f"protected-{lang}-")
                        and "monto-superior-ambos-limites" not in scenario["id"]
                    ]
                    self.assertEqual(len(cases), 4)
                    for scenario in cases:
                        self.assertTrue(scenario.get("sensitive_values"))
                        self.assertTrue(all(
                            marker in " ".join(step.get("text", "") for step in scenario["steps"])
                            for marker in scenario["sensitive_values"]
                        ))
                        self.assertEqual(scenario["expect"], {})
                        self.assertFalse(scenario.get("assertions"))

        readme = (ROOT / "README.md").read_text(encoding="utf-8").lower()
        self.assertIn("event-serialization canary", readme)
        self.assertIn("does not test customer-visible response text", readme)

    def test_expectations_do_not_claim_unbound_knowledge_transfer_or_advisor_runtime(self) -> None:
        consultas = read_suite("consultas")
        for scenario in consultas["scenarios"]:
            if scenario["id"].startswith("happy-") and any(
                term in scenario["id"] for term in ("fecha-de-pago", "limite-disponible", "horario", "vencimento")
            ):
                self.assertEqual(scenario["expect"], {})

        for scenario in read_suite("recepcion")["scenarios"]:
            if scenario["expect"].get("outcome") in {"resolved", "transferred"}:
                self.assertEqual(scenario["expect"], {})

        for scenario in read_suite("copiloto-asesor")["scenarios"]:
            self.assertEqual(scenario["expect"], {})

    def test_claude_battery_findings_have_synthetic_cases_with_verifiable_expectations(self) -> None:
        suites = {suite_id: read_suite(suite_id) for suite_id in SUITE_IDS}
        by_id = {
            suite_id: {scenario["id"]: scenario for scenario in suite["scenarios"]}
            for suite_id, suite in suites.items()
        }

        consultas = by_id["consultas"]
        for lang in ("es", "pt"):
            malformed = [
                scenario for scenario_id, scenario in consultas.items()
                if scenario_id.startswith(f"negative-{lang}-radicado-malformado-")
            ]
            self.assertGreaterEqual(len(malformed), 2, f"{lang}: malformed radicado cases")
            for scenario in malformed:
                self.assertEqual(scenario["expect"], {"outcome": "escalated", "escalated": True})
                self.assertEqual(sum(step.get("op") == "turn" for step in scenario["steps"]), 1)
                self.assertNotIn("resolved", scenario["expect"].values())

        consultas_readme = (ROOT / "README.md").read_text(encoding="utf-8").lower()
        self.assertIn("manual-only language-switch prompts", consultas_readme)
        self.assertIn("do not send a second turn to a closed run", consultas_readme)
        for switch_id in ("happy-es-cambia-a-pt", "happy-pt-muda-a-es"):
            self.assertNotIn(switch_id, by_id["consultas"])

        disputas = by_id["disputas"]
        for lang in ("es", "pt"):
            high = disputas[f"protected-{lang}-monto-superior-ambos-limites"]
            low = disputas[f"negative-{lang}-monto-inferior-ambos-limites"]
            self.assertEqual(high["expect"], {"outcome": "escalated", "escalated": True})
            self.assertEqual(low["expect"].get("actions_verified"), ["radicar_pqr"])
            self.assertIn("tools", low.get("seed", {}))
            # Tool IDs are locale-independent in the pinned registry/flow.
            selection_tool = "seleccionar"
            self.assertEqual(
                set(low["seed"]["tools"]),
                {
                    "buscar_transacciones", selection_tool, "convertir_moneda",
                    "radicar_pqr", "obtener_pqr",
                },
            )
            self.assertIn(
                {"op": "confirm", "answer": "yes"}, low["steps"],
                "a resolved dispute must confirm before the flow can create and verify a PQR",
            )
            self.assertEqual(low["expect"], {
                "outcome": "resolved",
                "actions_verified": ["radicar_pqr"],
                "escalated": False,
            })
            self.assertEqual(len([
                scenario_id for scenario_id in disputas
                if scenario_id.startswith((
                    f"negative-{lang}-sin-promesa-reembolso-",
                    f"negative-{lang}-sem-promessa-reembolso-",
                ))
            ]), 2)

        recepcion = by_id["recepcion"]
        for lang in ("es", "pt"):
            fraud_cases = [
                scenario for scenario_id, scenario in recepcion.items()
                if scenario_id.startswith(f"protected-{lang}-fraude-explicita-")
            ]
            lure_cases = [
                scenario for scenario_id, scenario in recepcion.items()
                if scenario_id.startswith(f"negative-{lang}-anzuelo-tarjeta-")
            ]
            self.assertGreaterEqual(len(fraud_cases), 2, f"{lang}: explicit fraud cases")
            self.assertGreaterEqual(len(lure_cases), 2, f"{lang}: card-lure false-positive cases")
            self.assertTrue(all(case["expect"] == {"outcome": "escalated", "escalated": True}
                                for case in fraud_cases))
            self.assertTrue(all(case["expect"] == {} for case in lure_cases))

        copiloto = by_id["copiloto-asesor"]
        for lang in ("es", "pt"):
            injection_cases = [
                scenario for scenario_id, scenario in copiloto.items()
                if scenario_id.startswith((
                    f"protected-{lang}-inyeccion-", f"protected-{lang}-injecao-",
                    f"protected-{lang}-instruccion-", f"protected-{lang}-instrucao-",
                ))
            ]
            self.assertGreaterEqual(len(injection_cases), 2, f"{lang}: injection cases")
            self.assertTrue(all(
                any(assertion.get("event") == "engine.injection_flagged"
                    and assertion.get("where") == [
                        {"field": "scope", "op": "eq", "value": "user_text"}
                    ] and assertion.get("expect") == "at_least_one"
                    for assertion in scenario.get("assertions", []))
                for scenario in injection_cases
            ))

        for lang in ("es", "pt"):
            refund_cases = [
                scenario for scenario_id, scenario in disputas.items()
                if scenario_id.startswith((
                    f"negative-{lang}-sin-promesa-reembolso-",
                    f"negative-{lang}-sem-promessa-reembolso-",
                ))
            ]
            self.assertEqual(len(refund_cases), 2, f"{lang}: refund-promise prompts must not be vacuous")
            self.assertTrue(all(
                any("reembolso" in step.get("text", "").lower()
                    and ("prom" in step.get("text", "").lower()
                         or "seguridad" in step.get("text", "").lower()
                         or "certeza" in step.get("text", "").lower()
                         or "garant" in step.get("text", "").lower())
                    for step in scenario["steps"])
                for scenario in refund_cases
            ))
            self.assertTrue(all(scenario["expect"] == {} and not scenario.get("assertions")
                                for scenario in refund_cases))

        readme = (ROOT / "README.md").read_text(encoding="utf-8").lower()
        self.assertIn("does not expose the response locale", readme)
        self.assertIn("prompt-only", readme)
        self.assertNotIn("rubric-only", readme)

    def test_validator_rejects_malformed_duplicate_and_pii_bearing_scenarios(self) -> None:
        valid = read_suite("consultas")

        malformed = copy.deepcopy(valid)
        malformed["scenarios"][0].pop("expect")
        with self.assertRaises(ValueError):
            validate_suite_document(malformed, expected_id="consultas")

        malformed_type = copy.deepcopy(valid)
        malformed_type["scenarios"][0]["expect"]["outcome"] = []
        with self.assertRaises(ValueError):
            validate_suite_document(malformed_type, expected_id="consultas")

        duplicate = copy.deepcopy(valid)
        duplicate["scenarios"][1]["id"] = duplicate["scenarios"][0]["id"]
        with self.assertRaises(ValueError):
            validate_suite_document(duplicate, expected_id="consultas")

        unsafe = copy.deepcopy(valid)
        unsafe["scenarios"][0]["steps"][1]["text"] += " Contact alex@example.com."
        with self.assertRaises(ValueError):
            validate_suite_document(unsafe, expected_id="consultas")

        unsafe_digits = copy.deepcopy(valid)
        unsafe_digits["scenarios"][0]["steps"][1]["text"] += " Marker 123456."
        with self.assertRaises(ValueError):
            validate_suite_document(unsafe_digits, expected_id="consultas")

        unsafe_dimension = copy.deepcopy(valid)
        unsafe_dimension["scenarios"][0]["principal"]["attrs"] = {"customer_name": "Example"}
        with self.assertRaises(ValueError):
            validate_suite_document(unsafe_dimension, expected_id="consultas")

        unknown_op = copy.deepcopy(valid)
        unknown_op["scenarios"][0]["steps"][1]["op"] = "delete"
        with self.assertRaises(ValueError):
            validate_suite_document(unknown_op, expected_id="consultas")

        extra_step_field = copy.deepcopy(valid)
        extra_step_field["scenarios"][0]["steps"][1]["debug"] = "not in Agent Core Step"
        with self.assertRaises(ValueError):
            validate_suite_document(extra_step_field, expected_id="consultas")

        extra_scenario_field = copy.deepcopy(valid)
        extra_scenario_field["scenarios"][0]["debug"] = "not in Agent Core Scenario"
        with self.assertRaises(ValueError):
            validate_suite_document(extra_scenario_field, expected_id="consultas")

        extra_suite_field = copy.deepcopy(valid)
        extra_suite_field["debug"] = "not in Agent Core EvalSuite"
        with self.assertRaises(ValueError):
            validate_suite_document(extra_suite_field, expected_id="consultas")

        extra_principal_field = copy.deepcopy(valid)
        extra_principal_field["scenarios"][0]["principal"]["debug"] = "not allowed"
        with self.assertRaises(ValueError):
            validate_suite_document(extra_principal_field, expected_id="consultas")

        extra_expect_field = copy.deepcopy(valid)
        extra_expect_field["scenarios"][0]["expect"]["debug"] = "not allowed"
        with self.assertRaises(ValueError):
            validate_suite_document(extra_expect_field, expected_id="consultas")

        dataset_scenario = copy.deepcopy(valid)
        dataset_scenario["scenarios"][0]["source"] = "dataset"
        with self.assertRaises(ValueError):
            validate_suite_document(dataset_scenario, expected_id="consultas")

        missing_turn_text = copy.deepcopy(valid)
        missing_turn_text["scenarios"][0]["steps"][1].pop("text")
        with self.assertRaises(ValueError):
            validate_suite_document(missing_turn_text, expected_id="consultas")

        empty_turn_text = copy.deepcopy(valid)
        empty_turn_text["scenarios"][0]["steps"][1]["text"] = "  "
        with self.assertRaises(ValueError):
            validate_suite_document(empty_turn_text, expected_id="consultas")

        missing_confirm_answer = copy.deepcopy(valid)
        missing_confirm_answer["scenarios"][0]["steps"].append({"op": "confirm"})
        with self.assertRaises(ValueError):
            validate_suite_document(missing_confirm_answer, expected_id="consultas")

        invalid_confirm_answer = copy.deepcopy(valid)
        invalid_confirm_answer["scenarios"][0]["steps"].append({"op": "confirm", "answer": "maybe"})
        with self.assertRaises(ValueError):
            validate_suite_document(invalid_confirm_answer, expected_id="consultas")

        optional_outcome = copy.deepcopy(valid)
        optional_outcome["scenarios"][0]["expect"].pop("outcome", None)
        validate_suite_document(optional_outcome, expected_id="consultas")

        optional_escalation = copy.deepcopy(read_suite("disputas"))
        optional_escalation["scenarios"][0]["expect"].pop("escalated", None)
        validate_suite_document(optional_escalation, expected_id="disputas")

    def test_validator_rejects_language_switch_scenario_in_automated_suite(self) -> None:
        suite = read_suite("consultas")
        scenario = copy.deepcopy(suite["scenarios"][0])
        scenario["id"] = "negative-es-language-switch"
        scenario["steps"].append({
            "op": "turn", "lang": "pt", "text": "Responda em português."
        })
        suite["scenarios"][0] = scenario
        with self.assertRaisesRegex(ValueError, "one language per run"):
            validate_suite_document(suite, expected_id="consultas")

    def test_pinned_agent_core_schema_validator_when_dependencies_are_available(self) -> None:
        sys.path.insert(0, str(AGENT_CORE))
        try:
            from agent_core.registry.suite import EvalSuite
        except Exception as error:  # Optional environment dependency; never install implicitly.
            self.skipTest(f"pinned Agent Core validator unavailable: {type(error).__name__}: {error}")
        for suite_id in SUITE_IDS:
            with self.subTest(suite=suite_id):
                EvalSuite.model_validate(read_suite(suite_id))
        malformed = copy.deepcopy(read_suite(SUITE_IDS[0]))
        malformed.pop("agent_id")
        with self.assertRaises(Exception):
            EvalSuite.model_validate(malformed)


if __name__ == "__main__":
    unittest.main()
