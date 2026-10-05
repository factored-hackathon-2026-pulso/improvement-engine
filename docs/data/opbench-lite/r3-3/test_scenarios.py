import hashlib
import json
import os
import re
import unittest
from pathlib import Path

from generate_scenarios import (_aggregate_entry, _aggregate_hash, _load_catalog,
                                build_pack, verify_source_catalog)


class ScenarioPackTests(unittest.TestCase):
    def setUp(self):
        self.pack = build_pack()

    def test_three_agent_core_suites_have_six_scenarios_each(self):
        suites = self.pack["suites"]
        self.assertEqual({"disputas", "consultas", "copiloto-asesor"}, set(suites))
        self.assertEqual({6}, {len(s["scenarios"]) for s in suites.values()})
        self.assertEqual({3}, {s["repetitions"] for s in suites.values()})

    def test_contract_shape_and_references_are_exact_and_hash_only(self):
        allowed_root = {"id", "version", "agent_id", "repetitions", "scenarios"}
        allowed_scenario = {"id", "source", "principal", "steps", "seed", "expect", "assertions"}
        for suite in self.pack["suites"].values():
            self.assertEqual(allowed_root, set(suite))
            self.assertRegex(suite["version"], r"^\d+\.\d+\.\d+$")
            for scenario in suite["scenarios"]:
                self.assertTrue(set(scenario) <= allowed_scenario)
                self.assertEqual("scripted", scenario["source"])
                self.assertRegex(scenario["principal"]["id"], r"^syn-(customer|advisor)-")
                self.assertTrue(all(re.fullmatch(r"[a-z0-9][a-z0-9_-]*", s["id"])
                                    for s in [scenario]))
        provenance = self.pack["provenance"]
        self.assertEqual(18, len(provenance["scenario_map"]))
        self.assertEqual(provenance["catalog_excerpt_sha256"], provenance["scenario_map"]["probe-cargo-no-reconocido-es"]["catalog_sha256"])
        self.assertEqual(provenance["source_artifact_sha256"], self.pack["provenance"]["scenario_map"]["probe-cargo-no-reconocido-es"]["source_artifact_sha256"])
        self.assertEqual(64, len(provenance["catalog_excerpt_sha256"]))
        self.assertTrue(all(re.fullmatch(r"[0-9a-f]{64}", item["aggregate_entry_sha256"])
                            for item in provenance["scenario_map"].values()))
        self.assertEqual({"M1-01", "M1-13", "E1-01"},
                         {item["opbench_entry_id"] for item in provenance["scenario_map"].values()})
        self.assertTrue(all(item["expected_behavior_status"] == "contract_target_not_observed"
                            and item["execution_result"] == "not_run"
                            for item in provenance["scenario_map"].values()))
        self.assertFalse(any("customer_id" in str(x).lower() or "row_id" in str(x).lower()
                             for x in provenance.values()))

    def test_catalog_is_read_from_pinned_aggregate_excerpt_and_hash_binds_metrics(self):
        catalog, digest = _load_catalog()
        provenance = self.pack["provenance"]
        self.assertEqual(digest, provenance["catalog_excerpt_sha256"])
        self.assertEqual(catalog["source_sha256"], provenance["source_artifact_sha256"])
        self.assertTrue(catalog["privacy"]["aggregate_only"])
        self.assertFalse(catalog["privacy"]["row_data_included"])
        self.assertFalse(catalog["privacy"]["customer_identifiers_included"])
        self.assertGreaterEqual(catalog["privacy"]["minimum_count"], 10)
        self.assertEqual({"M1-01", "M1-13", "E1-01"}, set(catalog["entries"]))
        for entry_id, entry in catalog["entries"].items():
            self.assertEqual({"title", "type", "status", "actionability", "definition", "cell",
                              "discovery", "replication", "snapshot"}, set(entry))
            self.assertEqual({"numerator", "denominator", "effect"} |
                             ({"baseline_numerator", "baseline_denominator"} if entry_id.startswith("M1") else set()) |
                             ({"adjusted_q"} if entry_id == "E1-01" else set()), set(entry["discovery"]))
            self.assertEqual({"numerator", "denominator", "effect"} |
                             ({"baseline_numerator", "baseline_denominator"} if entry_id.startswith("M1") else set()) |
                             ({"adjusted_q", "p_value"} if entry_id == "E1-01" else set()), set(entry["replication"]))
            self.assertEqual({"numerator", "denominator", "rate", "suppressed", "suppression_reason"},
                             set(entry["snapshot"]))
            self.assertIs(entry["snapshot"]["suppressed"], False)
        for entry_id, entry in catalog["entries"].items():
            expected = hashlib.sha256(json.dumps(
                _aggregate_entry(entry_id, entry), ensure_ascii=False, sort_keys=True,
                separators=(",", ":")).encode("utf-8")).hexdigest()
            self.assertEqual(expected, _aggregate_hash(entry_id, entry))
        mapped = provenance["scenario_map"]["probe-cargo-no-reconocido-es"]
        self.assertEqual(_aggregate_hash("M1-01", catalog["entries"]["M1-01"]),
                         mapped["aggregate_entry_sha256"])

    def test_optional_original_catalog_hash_matches_pinned_source_digest(self):
        raw_path = os.environ.get("PULSO_OPBENCH_CATALOG")
        if not raw_path:
            self.skipTest("set PULSO_OPBENCH_CATALOG to verify the original local artifact")
        catalog, _ = _load_catalog()
        verify_source_catalog(raw_path, catalog)

    def test_inputs_are_synthetic_hypothesis_probes_not_copied_data(self):
        catalog, _ = _load_catalog()
        def string_leaves(value, path=()):
            if isinstance(value, str):
                yield path, value
            elif isinstance(value, dict):
                for key, item in value.items():
                    yield from string_leaves(item, (*path, str(key)))
            elif isinstance(value, list):
                for index, item in enumerate(value):
                    yield from string_leaves(item, (*path, str(index)))
        leaves = list(string_leaves({"pack": self.pack, "catalog": catalog}))
        digest_fields = {"catalog_excerpt_sha256", "source_artifact_sha256", "aggregate_entry_sha256",
                         "catalog_sha256", "pinned_eval_suite_schema_blob", "source_sha256"}
        checked = [value for path, value in leaves if not (path and path[-1] in digest_fields)]
        all_text = " ".join(checked)
        self.assertNotRegex(all_text, r"(?i)[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}")
        self.assertNotRegex(all_text, r"\d{6,}")
        def numeric_leaves(value, path=()):
            if isinstance(value, bool):
                return
            if isinstance(value, (int, float)):
                yield path
            elif isinstance(value, dict):
                for key, item in value.items():
                    yield from numeric_leaves(item, (*path, str(key)))
            elif isinstance(value, list):
                for index, item in enumerate(value):
                    yield from numeric_leaves(item, (*path, str(index)))
        for path in numeric_leaves({"pack": self.pack, "catalog": catalog}):
            allowed_source_metric = len(path) >= 3 and path[:2] == ("pack", "provenance") \
                and path[2] == "source_findings"
            allowed_repetition = len(path) == 4 and path[0] == "pack" \
                and path[1] == "suites" and path[3] == "repetitions"
            allowed_privacy_threshold = len(path) == 3 and path[:2] == ("catalog", "privacy")
            allowed_catalog_metric = len(path) >= 3 and path[:2] == ("catalog", "entries")
            self.assertTrue(allowed_source_metric or allowed_repetition or allowed_privacy_threshold
                            or allowed_catalog_metric, path)
        self.assertTrue(all("SYNTH" in p["probe_rationale"] for p in self.pack["provenance"]["scenario_map"].values()))
        self.assertTrue(all(p["execution_result"] == "not_run"
                            for p in self.pack["provenance"]["scenario_map"].values()))

    def test_every_case_has_one_start_then_no_post_terminal_steps(self):
        for suite in self.pack["suites"].values():
            for scenario in suite["scenarios"]:
                ops = [step["op"] for step in scenario["steps"]]
                self.assertEqual("start", ops[0])
                self.assertEqual(1, ops.count("start"))
                if "confirm" in ops:
                    self.assertEqual("confirm", ops[-1])
                self.assertLessEqual(len(ops), 5)

    def test_known_agent_tool_names_only_and_control_is_not_a_new_opportunity(self):
        expected_tools = {
            "disputas": {"buscar_transacciones", "seleccionar", "convertir_moneda", "radicar_pqr", "obtener_pqr"},
            "consultas": {"obtener_pqr"},
            "copiloto-asesor": {"leer_movimientos", "leer_productos", "leer_pqr_cliente", "obtener_handoff", "leer_transcript"},
        }
        for agent_id, suite in self.pack["suites"].items():
            for scenario in suite["scenarios"]:
                tools = set((scenario.get("seed", {}).get("tools") or {}).keys())
                self.assertTrue(tools <= expected_tools[agent_id])
        control = self.pack["provenance"]["suite_notes"]["copiloto-asesor"]
        self.assertIn("negative control", control.lower())
        self.assertIn("not test detector suppression", control.lower())
        self.assertTrue(all(s["principal"]["attrs"]["role"] == "advisor"
                            for s in self.pack["suites"]["copiloto-asesor"]["scenarios"]))
        self.assertEqual("not_evaluable", self.pack["provenance"]["scenario_map"]["probe-lookup-movement-01"]["evaluator_status"])

    def test_problem_cases_cover_success_cancel_ambiguous_write_failure_and_readback_failure(self):
        scenarios = {scenario["id"]: scenario for scenario in self.pack["suites"]["disputas"]["scenarios"]}
        self.assertEqual("resolved", scenarios["probe-cargo-no-reconocido-es"]["expect"]["outcome"])
        self.assertEqual(["radicar_pqr"], scenarios["probe-cargo-no-reconocido-es"]["expect"]["actions_verified"])
        self.assertEqual("cancelled", scenarios["probe-cobro-duplicado-cancelado-pt"]["expect"]["outcome"])
        self.assertEqual([], scenarios["probe-cobro-duplicado-cancelado-pt"]["assertions"])
        self.assertEqual("escalated", scenarios["probe-lookup-failure-pt"]["expect"]["outcome"])
        self.assertEqual("escalated", scenarios["probe-readback-failure-es"]["expect"]["outcome"])
        self.assertNotIn("actions_verified", scenarios["probe-readback-failure-es"]["expect"])
        high_amount = scenarios["probe-amount-boundary-600-es"]
        self.assertEqual("escalated", high_amount["expect"]["outcome"])
        self.assertEqual([], high_amount["assertions"])
        self.assertTrue(self.pack["provenance"]["scenario_map"][high_amount["id"]]["runtime_audit_required"])
        cancelled = scenarios["probe-cobro-duplicado-cancelado-pt"]
        self.assertEqual([], cancelled["assertions"])
        self.assertTrue(self.pack["provenance"]["scenario_map"][cancelled["id"]]["runtime_audit_required"])
        for scenario, required in ((scenarios["probe-cargo-no-reconocido-es"],
                                    {"buscar_transacciones", "seleccionar", "convertir_moneda", "radicar_pqr", "obtener_pqr"}),
                                   (high_amount, {"buscar_transacciones", "seleccionar", "convertir_moneda"}),
                                   (scenarios["probe-readback-failure-es"], {"buscar_transacciones", "seleccionar", "convertir_moneda", "radicar_pqr", "obtener_pqr"})):
            self.assertTrue(required <= set(scenario["seed"]["tools"]))

    def test_missing_pqr_identifier_escalates_without_tool_seed_or_call(self):
        scenario = next(s for s in self.pack["suites"]["consultas"]["scenarios"]
                        if s["id"] == "probe-missing-radicado")
        self.assertNotIn("seed", scenario)
        self.assertEqual("escalated", scenario["expect"]["outcome"])
        self.assertEqual([{"event": "engine.tool_called", "expect": "none"}], scenario["assertions"])

    def test_dispute_fake_transaction_references_match_seed_ids(self):
        for scenario in self.pack["suites"]["disputas"]["scenarios"]:
            seed = scenario.get("seed", {}).get("tools", {})
            transaction_ids = {
                row["transaction_id"]
                for response in seed.get("buscar_transacciones", [])
                for row in response.get("result", [])
            }
            if "seleccionar" in seed:
                transaction_ids.add(seed["seleccionar"][0]["result"]["transaction_id"])
            text = " ".join(step.get("text", "") for step in scenario["steps"])
            for transaction_id in transaction_ids:
                self.assertIn(transaction_id, text)

    def test_pqr_lookup_references_match_the_seeded_fake_pqr_ids(self):
        for scenario in self.pack["suites"]["consultas"]["scenarios"][:4]:
            expected_id = scenario["seed"]["tools"]["obtener_pqr"][0]["result"]["id"]
            text = " ".join(step.get("text", "") for step in scenario["steps"])
            self.assertIn(expected_id, text)

    def test_advisor_fake_movement_references_match_the_seeded_ids(self):
        for scenario in self.pack["suites"]["copiloto-asesor"]["scenarios"]:
            expected_id = scenario["seed"]["tools"]["leer_movimientos"][0]["result"][0]["transaction_id"]
            text = " ".join(step.get("text", "") for step in scenario["steps"])
            self.assertIn(expected_id, text)


if __name__ == "__main__":
    unittest.main()
