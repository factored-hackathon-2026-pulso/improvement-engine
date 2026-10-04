"""CMPpy RED/GREEN: generic ChangeSpec -> DraftPlan compile over the seeded base world (stand-in, python)."""
import copy
import json
from pathlib import Path

import pytest
import yaml

from claude_standin import compile_step as C
from claude_standin.contracts import validate_out

ROOT = Path(__file__).resolve().parents[3]
WORLD_FILE = ROOT / "agent-core-assets" / "worlds" / "seeded-base.world.yaml"
SAMPLES = ROOT / "contracts" / "engine-steps" / "samples"
WORLD = C.load_world(WORLD_FILE)
PROMPT_REF, SUITE_REF = "prompt:resumen_radicado@1", "eval_suite:disputas-suite@1"


def spec(ops):
    return {"contract_version": "engine-steps/0", "step": "compile", "run_id": "run-cmp-0001", "data_class": "synthetic",
            "base_bundle_ref": "bundle:attention-demo@1",
            "change_spec": {"base_bundle_ref": "bundle:attention-demo@1", "opportunity_ref": "opportunity:o1@1",
                            "workflow_bridge_ref": "bridge:disputa-cargo@1", "operations": ops,
                            "expected_mechanism": "shorter closing reply", "affected_routes": ["disputa-cargo"],
                            "rollback_ref": "bundle:attention-demo@1"}}


def rep():
    return {"op": "replace", "target_kind": "prompt", "target_ref": PROMPT_REF, "new_ref": "prompt:resumen_radicado@2",
            "precondition_digest": C.asset_digest(WORLD, PROMPT_REF)}


def add():
    return {"op": "add", "target_kind": "eval_suite", "target_ref": SUITE_REF, "new_ref": "eval_suite:disputas-suite@2",
            "precondition_digest": C.asset_digest(WORLD, SUITE_REF)}


def test_two_operation_spec_compiles_and_validates():
    out = C.compile_change_spec(spec([rep(), add()]), WORLD)
    assert out["status"] == "compiled" and out["compiler_label"] == "claude-standin(python)"
    assert [o["op"] for o in out["draft_plan"]["operations"]] == ["replace", "add"]
    assert validate_out("compile", out) == []


def test_digest_is_deterministic_and_content_bound():
    a = C.compile_change_spec(spec([rep(), add()]), WORLD)
    assert a == C.compile_change_spec(copy.deepcopy(spec([rep(), add()])), WORLD)
    b = C.compile_change_spec(spec([rep()]), WORLD)
    assert a["draft_plan"]["digest"] != b["draft_plan"]["digest"]


def test_core_dry_run_digest_hook_wins():
    out = C.compile_change_spec(spec([rep()]), WORLD, dry_run=lambda ops: "sha256:" + "b" * 64)
    assert out["draft_plan"]["digest"] == "sha256:" + "b" * 64


def test_denied_kind_not_supported():
    o = rep(); o["op"] = "disable"
    assert C.compile_change_spec(spec([o]), WORLD)["denied_reason"] == "kind_not_supported"
    o = add(); o["target_kind"] = "prompt"; o["target_ref"] = PROMPT_REF
    assert C.compile_change_spec(spec([o]), WORLD)["denied_reason"] == "kind_not_supported"


def test_denied_missing_precondition():
    o = rep(); o["precondition_digest"] = "sha256:" + "0" * 64
    assert C.compile_change_spec(spec([o]), WORLD)["denied_reason"] == "missing_precondition"


def test_denied_outside_bridge():
    o = rep(); o["target_ref"] = "prompt:other@1"
    assert C.compile_change_spec(spec([o]), WORLD)["denied_reason"] == "outside_bridge"
    s = spec([rep()]); s["change_spec"]["affected_routes"] = ["not-a-route"]
    assert C.compile_change_spec(s, WORLD)["denied_reason"] == "outside_bridge"


def test_denied_mutable_reference():
    o = rep(); o["new_ref"] = PROMPT_REF  # would overwrite the published immutable version
    assert C.compile_change_spec(spec([o]), WORLD)["denied_reason"] == "mutable_reference"
    o = rep(); del o["new_ref"]
    assert C.compile_change_spec(spec([o]), WORLD)["denied_reason"] == "mutable_reference"


def test_denied_output_has_no_draft_plan_and_validates():
    o = rep(); o["op"] = "disable"
    out = C.compile_change_spec(spec([o]), WORLD)
    assert "draft_plan" not in out and validate_out("compile", out) == []


def test_schema_invalid_input_raises_not_denied():
    bad = json.loads((SAMPLES / "invalid-compile-unsupported-kind.in.json").read_text("utf-8"))
    with pytest.raises(ValueError):
        C.compile_change_spec(bad, WORLD)


def test_world_declaration_drives_targets():
    w = yaml.safe_load(WORLD_FILE.read_text("utf-8"))
    assert w["targetable_kinds"] == ["replace_prompt", "add_eval_suite"]
    assert WORLD["targetable_kinds"] == w["targetable_kinds"]
