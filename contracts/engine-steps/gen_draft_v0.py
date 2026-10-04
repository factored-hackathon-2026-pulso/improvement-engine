"""One-shot generator of the draft v0 schemas, samples and port list.

Run from this directory: python gen_draft_v0.py && python digest.py --write
The generated JSON files are the contract; this script only avoids hand-typing.
"""
import copy
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def dump(rel, obj):
    p = HERE / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8", newline="\n")


SHA = "^sha256:[0-9a-f]{64}$"
DC = ["synthetic", "treated", "e0", "original"]
ID = {"type": "string", "pattern": "^[a-z0-9][a-z0-9._-]{2,63}$"}
DG = {"type": "string", "pattern": SHA}
REF = {"type": "string", "pattern": "^[a-z_]+:[A-Za-z0-9._-]+@[0-9]+$"}
RATE = {"type": "number", "minimum": 0, "maximum": 1}


def env(step, side, props, req, extra=None):
    p = {
        "contract_version": {"const": "engine-steps/0"},
        "step": {"const": step},
        "run_id": ID,
        "data_class": {"enum": DC},
    }
    p.update(props)
    s = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": f"engine-steps/{step}.{side}",
        "title": f"{step} step {side}put",
        "type": "object",
        "additionalProperties": False,
        "required": ["contract_version", "step", "run_id", "data_class"] + req,
        "properties": p,
    }
    s.update(extra or {})
    return s


def obj(req, props):
    return {"type": "object", "additionalProperties": False, "required": req, "properties": props}


def arr(items, n=0):
    return {"type": "array", "minItems": n, "items": items}


CHANGE_SPEC = obj(
    ["base_bundle_ref", "opportunity_ref", "workflow_bridge_ref", "operations",
     "expected_mechanism", "affected_routes", "rollback_ref"],
    {
        "base_bundle_ref": REF, "opportunity_ref": REF, "workflow_bridge_ref": REF,
        "operations": arr(obj(
            ["op", "target_kind", "target_ref", "precondition_digest"],
            {"op": {"enum": ["add", "replace", "disable"]},
             "target_kind": {"enum": ["prompt", "eval_suite"]},
             "target_ref": REF, "new_ref": REF, "precondition_digest": DG}), 1),
        "expected_mechanism": {"type": "string", "minLength": 1},
        "affected_routes": arr({"type": "string", "minLength": 1}, 1),
        "rollback_ref": REF,
    },
)

compile_out = env("compile", "out", {
    "status": {"enum": ["compiled", "denied"]},
    "draft_plan": obj(["operations", "digest"],
                      {"operations": arr({"type": "object"}, 1), "digest": DG}),
    "denied_reason": {"enum": ["kind_not_supported", "missing_precondition",
                               "outside_bridge", "mutable_reference"]},
    "compiler_label": {"type": "string"},
}, ["status", "compiler_label"], {"oneOf": [
    {"required": ["draft_plan"], "properties": {"status": {"const": "compiled"}}},
    {"required": ["denied_reason"], "properties": {"status": {"const": "denied"}}},
]})

SCHEMAS = {
    "sensors.in": env("sensors", "in", {
        "source_snapshot_ref": REF, "discovery_config_ref": REF,
        "window": obj(["start", "end"], {"start": {"type": "string"}, "end": {"type": "string"}}),
        "metric_spec_refs": arr(REF, 1)},
        ["source_snapshot_ref", "discovery_config_ref", "window", "metric_spec_refs"]),
    "sensors.out": env("sensors", "out", {
        "signals": arr(obj(
            ["signal_id", "metric_id", "population", "numerator", "denominator",
             "holdout_checked", "evidence_ref"],
            {"signal_id": ID, "metric_id": ID, "population": {"type": "string", "minLength": 1},
             "numerator": {"type": "integer", "minimum": 0},
             "denominator": {"type": "integer", "minimum": 1},
             "holdout_checked": {"type": "boolean"}, "evidence_ref": ID})),
        "discards": arr(obj(["metric_id", "reason"], {
            "metric_id": ID,
            "reason": {"enum": ["below_k", "failed_holdout", "low_coverage", "duplicate", "other"]}}))},
        ["signals", "discards"]),
    "recompute.in": env("recompute", "in", {
        "lab_ref": REF, "signal_ids": arr(ID, 1),
        "scout_claims": arr(obj(["signal_id", "claimed_rate"],
                                {"signal_id": ID, "claimed_rate": RATE}), 1)},
        ["lab_ref", "signal_ids", "scout_claims"]),
    "recompute.out": env("recompute", "out", {
        "recomputes": arr(obj(
            ["signal_id", "claimed_rate", "recomputed_rate", "match", "evidence_ref"],
            {"signal_id": ID, "claimed_rate": RATE, "recomputed_rate": RATE,
             "match": {"type": "boolean"}, "evidence_ref": ID}), 1)},
        ["recomputes"]),
    "validation.in": env("validation", "in", {
        "signal_id": ID, "recompute_ref": REF,
        "checks": arr({"enum": ["schema_coverage", "denominator", "temporality", "mix",
                                "duplicates", "multiple_testing", "window_sensitivity",
                                "counterexamples"]}, 1),
        "scout_actor": ID, "verifier_actor": ID},
        ["signal_id", "recompute_ref", "checks", "scout_actor", "verifier_actor"]),
    "validation.out": env("validation", "out", {
        "signal_id": ID, "verdict": {"enum": ["corroborated", "refuted", "inconclusive"]},
        "check_results": arr(obj(["check", "status"], {
            "check": {"type": "string"}, "status": {"enum": ["pass", "fail", "not_applicable"]}}), 1),
        "verifier_actor": ID, "evidence_refs": arr(ID)},
        ["signal_id", "verdict", "check_results", "verifier_actor"]),
    "compile.in": env("compile", "in", {
        "change_spec": {"$ref": "#/$defs/ChangeSpec"}, "base_bundle_ref": REF},
        ["change_spec", "base_bundle_ref"], {"$defs": {"ChangeSpec": CHANGE_SPEC}}),
    "compile.out": compile_out,
    "gate.in": env("gate", "in", {
        "base_arm_report_ref": REF, "candidate_arm_report_ref": REF, "suite_ref": REF,
        "judge_actor": ID, "author_actors": arr(ID, 1)},
        ["base_arm_report_ref", "candidate_arm_report_ref", "suite_ref", "judge_actor",
         "author_actors"]),
    "gate.out": env("gate", "out", {
        "verdict": {"enum": ["pass", "fail", "not_evaluable"]},
        "gates": arr(obj(["gate", "status"], {
            "gate": {"enum": ["safety", "improvement"]},
            "status": {"enum": ["pass", "fail", "not_evaluable"]},
            "reason": {"type": "string"}}), 2),
        "judge_actor": ID, "quality_claims": {"enum": ["forbidden", "allowed"]}},
        ["verdict", "gates", "judge_actor", "quality_claims"]),
}
for k, v in SCHEMAS.items():
    dump(f"schemas/{k.split('.')[0]}.{k.split('.')[1]}.schema.json", v)


def base(step, **kw):
    return {"contract_version": "engine-steps/0", "step": step, "run_id": "run-demo-0001",
            "data_class": "synthetic", **kw}


def R(n, i=1):
    return f"{n}:sample-{i}@1"


SD = "sha256:" + "a" * 64
VALID = {
    "sensors.in": base("sensors", source_snapshot_ref=R("source_snapshot"),
                       discovery_config_ref=R("discovery_config"),
                       window={"start": "2026-04-18", "end": "2026-06-17"},
                       metric_spec_refs=[R("metric_spec")]),
    "sensors.out": base("sensors", signals=[{
        "signal_id": "sig-0001", "metric_id": "contact_unresolved",
        "population": "phone/complaint", "numerator": 120, "denominator": 400,
        "holdout_checked": True, "evidence_ref": "ev-0001"}],
        discards=[{"metric_id": "txn_rejects", "reason": "below_k"}]),
    "recompute.in": base("recompute", lab_ref=R("lab"), signal_ids=["sig-0001"],
                         scout_claims=[{"signal_id": "sig-0001", "claimed_rate": 0.3}]),
    "recompute.out": base("recompute", recomputes=[{
        "signal_id": "sig-0001", "claimed_rate": 0.3, "recomputed_rate": 0.3,
        "match": True, "evidence_ref": "ev-0001"}]),
    "validation.in": base("validation", signal_id="sig-0001", recompute_ref=R("recompute"),
                          checks=["denominator", "temporality"],
                          scout_actor="actor-scout", verifier_actor="actor-verifier"),
    "validation.out": base("validation", signal_id="sig-0001", verdict="corroborated",
                           check_results=[{"check": "denominator", "status": "pass"}],
                           verifier_actor="actor-verifier", evidence_refs=["ev-0001"]),
    "compile.in": base("compile", base_bundle_ref=R("bundle"), change_spec={
        "base_bundle_ref": R("bundle"), "opportunity_ref": R("opportunity"),
        "workflow_bridge_ref": R("bridge"),
        "operations": [{"op": "replace", "target_kind": "prompt", "target_ref": R("prompt"),
                        "new_ref": R("prompt", 2), "precondition_digest": SD}],
        "expected_mechanism": "clearer follow-up wording",
        "affected_routes": ["pqr_status_lookup"], "rollback_ref": R("bundle", 0)}),
    "compile.out": base("compile", status="compiled", compiler_label="claude-standin(python)",
                        draft_plan={"operations": [{"op": "replace"}], "digest": SD}),
    "gate.in": base("gate", base_arm_report_ref=R("arm_report"),
                    candidate_arm_report_ref=R("arm_report", 2), suite_ref=R("eval_suite"),
                    judge_actor="actor-judge", author_actors=["actor-builder"]),
    "gate.out": base("gate", verdict="pass",
                     gates=[{"gate": "safety", "status": "pass"},
                            {"gate": "improvement", "status": "pass"}],
                     judge_actor="actor-judge", quality_claims="forbidden"),
}
for k, v in VALID.items():
    step, side = k.split(".")
    dump(f"samples/valid-{step}.{side}.json", v)


def bad(name, key, fn):
    v = copy.deepcopy(VALID[key])
    fn(v)
    step, side = key.split(".")
    dump(f"samples/invalid-{step}-{name}.{side}.json", v)


bad("unknown-version", "sensors.in", lambda v: v.update(contract_version="engine-steps/9"))
bad("zero-denominator", "sensors.out", lambda v: v["signals"][0].update(denominator=0))
bad("claim-out-of-range", "recompute.in", lambda v: v["scout_claims"][0].update(claimed_rate=1.5))
bad("bad-verdict", "validation.out", lambda v: v.update(verdict="maybe"))
bad("unsupported-kind", "compile.in",
    lambda v: v["change_spec"]["operations"][0].update(target_kind="flow"))
bad("compiled-without-plan", "compile.out", lambda v: v.pop("draft_plan"))
bad("one-gate", "gate.out", lambda v: v.update(gates=v["gates"][:1]))


def port(name, direction, steps, lane, desc):
    return {"name": name, "direction": direction, "steps": steps, "owner_lane": lane,
            "description": desc}


dump("ports.json", {"contract_version": "engine-steps/0", "status": "draft", "ports": [
    port("SourceSnapshotPort", "driven", ["sensors"], "X-SRC", "read sealed snapshot partitions"),
    port("SensorPort", "driving", ["sensors"], "X-SENS", "run metric specs, emit signals and discards"),
    port("LabQueryPort", "driven", ["recompute"], "X-SENS", "treated k-anonymous aggregate lab"),
    port("RecomputePort", "driving", ["recompute"], "L-E2E", "deterministic recompute of scout figures"),
    port("ModelPort", "driven", ["validation", "compile"], "L-MODEL", "LLM calls via the gateway; treated payloads only"),
    port("VerifierPort", "driving", ["validation"], "X-GATE", "independent verifier checks"),
    port("CompilerPort", "driving", ["compile"], "X-COMPILE", "ChangeSpec to DraftPlan"),
    port("CoreClientPort", "driven", ["compile", "gate"], "L-CLIENT", "dry-run, write and arm runs on Agent Core"),
    port("GatePort", "driving", ["gate"], "X-GATE", "two-gate verdict from arm reports"),
]})
