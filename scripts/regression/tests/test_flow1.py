"""FLOW1 offline tests: suites of the four flow edits (which fail on the base and which cannot), self-test of the validator presets."""
import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import build_suite as bs  # noqa: E402
import prove_fails_on_base as pf  # noqa: E402

REPO = HERE.parents[1]
TABLE = json.loads((REPO / "seams" / "crates" / "reasoning" / "fixtures" / "mapping_table.json").read_text(encoding="utf-8"))
FINDING = json.loads((HERE / "fixtures" / "finding_m4_pqr_status.json").read_text(encoding="utf-8"))


def preset(target: str, pid: str) -> dict:
    for row in TABLE["rows"]:
        for c in row["candidates"]:
            if c["target_ref"] == target:
                return next(p for p in c["params"]["presets"] if p["id"] == pid)
    raise KeyError(target)


VALIDATOR = {**FINDING, "art2": {"agent": "consultas", "flow": "consulta-pqr", "op": "add_validator", "position": "pedir_radicado", "from_tool": None,
                                 "discriminator": "tool_not_called", "chain": [{"id": "pedir_radicado", "type": "collect", "slot": "radicado", "tool": None}],
                                 "preset": preset("flow_edit:consulta-pqr/add_validator", "radicado_token")}}
ASK_DISPUTA = {**FINDING, "art2": {"agent": "disputas", "flow": "disputa-cargo", "op": "insert_ask", "position": "pedir_cargo.ok", "from_tool": None,
                                   "discriminator": "tool_not_called", "chain": [{"id": "pedir_cargo", "type": "collect", "slot": "descripcion_cargo", "tool": None}],
                                   "preset": preset("flow_edit:disputa-cargo/insert_ask", "ask_fecha")}}
ASK_AFTER_TOOL = {**FINDING, "art2": {"agent": "consultas", "flow": "consulta-pqr", "op": "insert_ask", "position": "consultar.ok", "from_tool": "obtener_pqr",
                                      "discriminator": "run_not_closed",
                                      "chain": [{"id": "pedir_radicado", "type": "collect", "slot": "radicado", "tool": None}, {"id": "consultar", "type": "tool", "slot": None, "tool": "obtener_pqr"}],
                                      "preset": preset("flow_edit:disputa-cargo/insert_ask", "ask_fecha")}}
NOTICE = {**FINDING, "art2": {"agent": "consultas", "flow": "consulta-pqr", "op": "insert_notice", "position": "consultar.error", "from_tool": "obtener_pqr",
                              "discriminator": None, "chain": None, "preset": preset("flow_edit:consulta-pqr/insert_notice", "notice_handoff")}}
ACK = {**FINDING, "art2": {**NOTICE["art2"], "op": "insert_ack", "position": "pedir_radicado.ok", "from_tool": None,
                           "preset": preset("flow_edit:consulta-pqr/insert_ack", "ack_recibido")}}


class Validator(unittest.TestCase):
    def test_every_validator_preset_of_the_table_passes_its_own_self_test(self):
        n = 0
        for row in TABLE["rows"]:
            for c in row["candidates"]:
                if c["kind"] != "flow_edit" or c["params"]["op"] != "add_validator":
                    continue
                for p in c["params"]["presets"]:
                    n += 1
                    bs._selftest({"preset": p})
        self.assertGreaterEqual(n, 2)

    def test_selftest_refuses_a_validator_that_accepts_its_reject_example(self):
        bad = json.loads(json.dumps(VALIDATOR["art2"]))
        bad["preset"]["validator"]["value"] = "(?i)(.+)"
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs._selftest(bad)
        self.assertEqual(cm.exception.code, "validator_selftest")

    def test_cases_reject_a_text_without_the_id_and_fail_on_the_base_by_construction(self):
        b = bs.build_suite(VALIDATOR, "flow_edit:consulta-pqr/add_validator")
        self.assertEqual((b["mechanism"], b["agent"]), ("flow_validator", "consultas"))
        by = {s["id"]: s for s in b["suite"]["scenarios"]}
        self.assertEqual(len(b["finding_case_ids"]), 6)
        for cid in b["finding_case_ids"]:
            c = by[cid]
            self.assertEqual(len(c["steps"]), 2, "one turn only: a second turn would hit a closed run on the base")
            self.assertIn({"event": "engine.tool_called", "expect": "none"}, c["assertions"])
            self.assertIn({"event": "engine.run_closed", "expect": "none"}, c["assertions"])
            self.assertIn("obtener_pqr", c["seed"]["tools"], "the tool is seeded, so the base resolves")
        # the three pulso-min guards stay (their utterances carry a real id) plus an accepted-id guard per locale
        self.assertEqual(len(b["guard_case_ids"]), 5)
        self.assertEqual(b["dropped_guards"], [])
        for gid in ("guard-valida-acepta-es", "guard-valida-acepta-pt"):
            self.assertEqual(by[gid]["expect"], {"outcome": "resolved", "escalated": False})

    def test_the_pulso_min_guards_utterances_still_pass_the_validator(self):
        v = VALIDATOR["art2"]["preset"]["validator"]
        for s in bs.load_guards("consultas"):
            if s["id"] in ("guard-es-falla-herramienta-escala",):
                for st in s["steps"]:
                    if st["op"] == "turn":
                        self.assertTrue(bs.validator_accepts(v, st["text"]), st["text"])

    def test_coverage_names_what_is_native_and_what_is_not(self):
        b = bs.build_suite(VALIDATOR, "flow_edit:consulta-pqr/add_validator")
        c = pf.coverage(b, {"state": "not_applicable"})
        self.assertEqual(c["native"][0], "flow_validator_rejects_input")
        self.assertIn("flow_input_format", c["not_measured"])


class Ask(unittest.TestCase):
    def test_before_a_tool_the_candidate_does_not_call_it_and_the_slow_guards_are_dropped(self):
        b = bs.build_suite(ASK_DISPUTA, "flow_edit:disputa-cargo/insert_ask")
        self.assertEqual((b["mechanism"], b["agent"]), ("flow_ask", "disputas"))
        by = {s["id"]: s for s in b["suite"]["scenarios"]}
        for cid in b["finding_case_ids"]:
            self.assertEqual(len(by[cid]["steps"]), 2)
            self.assertIn({"event": "engine.tool_called", "expect": "none"}, by[cid]["assertions"])
        self.assertEqual(sorted(b["dropped_guards"]), ["guard-es-cancela-confirmacion", "guard-es-monto-alto-escala"])
        self.assertIn("guard-es-fraude-interrumpe", b["guard_case_ids"])
        self.assertTrue(all(g in by for g in b["guard_case_ids"]))
        c = pf.coverage(b, {"state": "not_applicable"})
        self.assertIn("tool_failure_exit_after_extra_turn", c["not_measured"])

    def test_after_a_tool_the_discriminator_is_the_run_not_closing(self):
        b = bs.build_suite(ASK_AFTER_TOOL, "flow_edit:consulta-pqr/insert_ask")
        by = {s["id"]: s for s in b["suite"]["scenarios"]}
        for cid in b["finding_case_ids"]:
            self.assertEqual(by[cid]["assertions"][0], {"event": "engine.run_closed", "expect": "none"})
            self.assertEqual(len(by[cid]["steps"]), 2)

    def test_a_position_without_a_discriminator_is_refused(self):
        bad = json.loads(json.dumps(ASK_DISPUTA))
        bad["art2"]["discriminator"] = None
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(bad, "flow_edit:disputa-cargo/insert_ask")
        self.assertEqual(cm.exception.code, "no_native_evidence")


class PathOnly(unittest.TestCase):
    def test_notice_cases_seed_the_failing_tool_and_expect_the_same_escalation_as_the_base(self):
        b = bs.build_suite(NOTICE, "flow_edit:consulta-pqr/insert_notice")
        by = {s["id"]: s for s in b["suite"]["scenarios"]}
        self.assertEqual(len(b["finding_case_ids"]), 6)
        for cid in b["finding_case_ids"]:
            self.assertEqual(by[cid]["expect"], {"outcome": "escalated", "escalated": True})
            self.assertIn(by[cid]["seed"]["tools"]["obtener_pqr"][0]["status"] if "status" in by[cid]["seed"]["tools"]["obtener_pqr"][0] else "", ("error", "timeout", "denied"))
        c = pf.coverage(b, {"state": "not_applicable"})
        self.assertEqual(c["native"][0], "flow_path_exercised")

    def test_ack_cases_resolve_and_a_notice_on_a_collect_exit_has_no_scripted_run(self):
        b = bs.build_suite(ACK, "flow_edit:consulta-pqr/insert_ack")
        by = {s["id"]: s for s in b["suite"]["scenarios"]}
        for cid in b["finding_case_ids"]:
            self.assertEqual(by[cid]["expect"]["outcome"], "resolved")
        collect_exit = json.loads(json.dumps(NOTICE))
        collect_exit["art2"].update(position="pedir_radicado.max_attempts", from_tool=None)
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(collect_exit, "flow_edit:consulta-pqr/insert_notice")
        self.assertEqual(cm.exception.code, "no_scenario_for_position")

    def test_a_suite_is_deterministic_and_free_of_pii_shapes(self):
        for f, t in ((VALIDATOR, "flow_edit:consulta-pqr/add_validator"), (ASK_DISPUTA, "flow_edit:disputa-cargo/insert_ask"), (NOTICE, "flow_edit:consulta-pqr/insert_notice")):
            a, b = bs.build_suite(f, t), bs.build_suite(f, t)
            self.assertEqual(bs.dumps(a), bs.dumps(b))
            self.assertEqual(bs.pii_problems(json.dumps([[s["steps"], s.get("seed")] for s in a["suite"]["scenarios"]])), [])

    def test_missing_params_are_refused(self):
        with self.assertRaises(bs.SuiteRefused) as cm:
            bs.build_suite(FINDING, "flow_edit:consulta-pqr/add_validator")
        self.assertEqual(cm.exception.code, "params_missing")


if __name__ == "__main__":
    unittest.main()
