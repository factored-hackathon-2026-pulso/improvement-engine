"""Regenerates cases/*.in.json and cases/*.expected.json from the Python reference (e2e-core compile_step.py).

Run from the repo root: python seams/crates/steps/tests/fixtures/cmp/gen_expected.py
Expected files are canonical JSON (sorted keys, compact, ASCII). The only normalisation applied to the reference
output is compiler_label: "claude-standin(python)" -> "claude-standin". Schema-invalid inputs are `*.error` markers.
"""
import copy
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[6]
sys.path.insert(0, str(ROOT / "e2e-core" / "src"))
from claude_standin import compile_step as C  # noqa: E402

OUT = Path(__file__).resolve().parent / "cases"
WORLD = C.load_world(ROOT / "agent-core-assets" / "worlds" / "seeded-base.world.yaml")
P, S = "prompt:resumen_radicado@1", "eval_suite:disputas-tarea-suite@1"
SAMPLES = ROOT / "contracts" / "engine-steps" / "samples"


def canon(v):
    return json.dumps(v, sort_keys=True, separators=(",", ":"))


def spec(ops, **over):
    d = {"contract_version": "engine-steps/0", "step": "compile", "run_id": "run-cmp-0001", "data_class": "synthetic",
         "base_bundle_ref": "bundle:attention-task@1",
         "change_spec": {"base_bundle_ref": "bundle:attention-task@1", "opportunity_ref": "opportunity:o1@1",
                         "workflow_bridge_ref": "bridge:disputa-tarea@1", "operations": ops,
                         "expected_mechanism": "shorter closing reply áñ", "affected_routes": ["disputa-tarea"],
                         "rollback_ref": "bundle:attention-task@1"}}
    for k, v in over.items():
        d["change_spec"][k] = v
    return d


def rep():
    return {"op": "replace", "target_kind": "prompt", "target_ref": P, "new_ref": "prompt:resumen_radicado@2",
            "precondition_digest": C.asset_digest(WORLD, P)}


def add():
    return {"op": "add", "target_kind": "eval_suite", "target_ref": S, "new_ref": "eval_suite:disputas-tarea-suite@2",
            "precondition_digest": C.asset_digest(WORLD, S)}


def mod(o, **kw):
    o = copy.deepcopy(o)
    for k, v in kw.items():
        if v is None:
            o.pop(k, None)
        else:
            o[k] = v
    return o


cases = {
    "two_ops_compiled": spec([rep(), add()]),
    "replace_only": spec([rep()]),
    "add_only": spec([add()]),
    "denied_kind_disable": spec([mod(rep(), op="disable")]),
    "denied_kind_add_prompt": spec([mod(add(), target_kind="prompt", target_ref=P)]),
    "denied_precondition": spec([mod(rep(), precondition_digest="sha256:" + "0" * 64)]),
    "denied_outside_target": spec([mod(rep(), target_ref="prompt:other@1")]),
    "denied_outside_major": spec([mod(rep(), target_ref="prompt:resumen_radicado@2")]),
    "denied_outside_route": spec([rep()], affected_routes=["not-a-route"]),
    "denied_outside_bridge_ref": spec([rep()], workflow_bridge_ref="bridge:otro-flujo@1"),
    "denied_outside_bundle": spec([rep()], base_bundle_ref="bundle:other@1"),
    "denied_mutable_same_ref": spec([mod(rep(), new_ref=P)]),
    "denied_mutable_no_new_ref": spec([mod(rep(), new_ref=None)]),
    "denied_mutable_double_publish": spec([rep(), rep()]),
    "first_failure_wins": spec([mod(rep(), op="disable"), mod(add(), new_ref=None)]),
    "frz0_valid_compile_sample": json.loads((SAMPLES / "valid-compile.in.json").read_text("utf-8")),
}
bad = json.loads((SAMPLES / "invalid-compile-unsupported-kind.in.json").read_text("utf-8"))
cases["frz0_invalid_unsupported_kind"] = bad
cases["invalid_extra_field"] = {**spec([rep()]), "extra": 1}

OUT.mkdir(parents=True, exist_ok=True)
for old in OUT.iterdir():
    old.unlink()
for name, doc in cases.items():
    (OUT / f"{name}.in.json").write_text(canon(doc), "utf-8")
    try:
        out = C.compile_change_spec(doc, WORLD)
    except ValueError:
        (OUT / f"{name}.error").write_text("schema-invalid input", "utf-8")
        continue
    assert out["compiler_label"] == C.LABEL
    out["compiler_label"] = "claude-standin"
    (OUT / f"{name}.expected.json").write_text(canon(out), "utf-8")
print(f"{len(cases)} cases written to {OUT}")
