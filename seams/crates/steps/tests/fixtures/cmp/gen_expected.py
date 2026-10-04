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

# The FRZ0 valid-compile sample names `prompt:sample-1@1`, outside the seeded world, so the case above is denied
# outside_bridge by design. This case keeps the sample's shape retargeted onto the seeded world and compiles.
_f = json.loads((SAMPLES / "valid-compile.in.json").read_text("utf-8"))
_f["base_bundle_ref"] = _f["change_spec"]["base_bundle_ref"] = "bundle:attention-task@1"
_f["change_spec"]["workflow_bridge_ref"] = "bridge:disputa-tarea@1"
_f["change_spec"]["affected_routes"] = ["disputa-tarea"]
_f["change_spec"]["operations"][0].update(
    target_ref=P, new_ref="prompt:resumen_radicado@2", precondition_digest=C.asset_digest(WORLD, P))
cases["frz0_valid_compile_sample_in_world"] = _f

# Raw (non-canonical) inputs: written as is, the reference parses them with json.loads.
BS = chr(92)
_g = canon(spec([rep()]))
_u = lambda h: BS + "u" + h  # noqa: E731
RAW = {
    "raw_pretty_whitespace": json.dumps(spec([rep(), add()]), indent=3).replace("\n", "\r\n") + "\r\n\t ",
    "raw_escaped_keys": _g.replace('"run_id"', '"' + _u("0072") + 'un_id"').replace('"step"', '"st' + _u("0065") + 'p"'),
    "raw_slash_escape": _g.replace("shorter closing", "shorter" + BS + "/closing"),
    "raw_escapes_in_text": _g.replace("shorter closing reply " + _u("00e1") + _u("00f1"),
                                      "caf" + _u("00e9") + " " + _u("d83d") + _u("de00") + " " + _u("007f") + " "
                                      + _u("0001") + " " + BS + "n" + BS + "t" + BS + BS + BS + '" raw ü \U0001F600'),
    "raw_duplicate_key_last_wins": _g.replace('{"base_bundle_ref"', '{"run_id":"bad run id","base_bundle_ref"', 1),
    "raw_leading_zero_major": _g.replace("prompt:resumen_radicado@1", "prompt:resumen_radicado@0000000000000000000000001"),
    "raw_huge_new_major": _g.replace("resumen_radicado@2", "resumen_radicado@99999999999999999999999999999"),
    "raw_trailing_newline_ref": _g.replace('"opportunity:o1@1"', '"opportunity:o1@1' + BS + 'n"'),
    "raw_trailing_comma": _g.replace('"step":"compile"}', '"step":"compile",}'),
    "raw_bad_number_dup_key": _g.replace('{"base_bundle_ref"', '{"run_id":-,"base_bundle_ref"', 1),
    "raw_bad_hex_plus": _g.replace("shorter", "sh" + _u("+041") + "rter"),
    "raw_lone_surrogate": _g.replace("shorter", "sh" + _u("d800") + "rter"),
    "raw_control_char": _g.replace("shorter", "sh\torter"),
    "raw_top_array": "[1,2]",
    "raw_empty": "",
}

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
for name, text in RAW.items():
    (OUT / f"{name}.in.json").write_bytes(text.encode("utf-8"))
    try:
        out = C.compile_change_spec(json.loads(text), WORLD)
    except ValueError:  # json.JSONDecodeError and schema-invalid input
        (OUT / f"{name}.error").write_text("schema-invalid or malformed input", "utf-8")
        continue
    out["compiler_label"] = "claude-standin"
    (OUT / f"{name}.expected.json").write_text(canon(out), "utf-8")
print(f"{len(cases) + len(RAW)} cases written to {OUT}")
