//! The treated requests the thread sends through the `ModelPort`, one per role. Every payload is a TPS-shaped agent input dict:
//! opaque ids, enums and one k-anonymous aggregate row, never a free-text field. The ids that are not system-issued shapes
//! (`sig-0001`) travel in the request `registry`, as the scanner requires.
use crate::SignalSeed;
use engine::models::{DataClass, ModelRequest, Role};
use serde_json::{Value, json};

pub const SCOUT_SYSTEM: &str = "You are the scout. Claim the rate of the one signal from its aggregate row. Answer with JSON {\"hypotheses\":[{\"id\":\"h_1\",\"signal_id\":<id>,\"claimed_rate\":<number 0..1>}]}. No quality claims.";
pub const VERIFIER_SYSTEM: &str = "You are the independent verifier. Compare the claimed rate with the aggregate row. Answer with JSON {\"verdict\":\"agree\"|\"disagree\"}. No quality claims.";
pub const BUILDER_SYSTEM: &str = "You are the builder. Propose one change to a prompt of the catalogue for the signal. Answer with JSON {\"proposal\":{\"kind\":\"prompt\",\"op\":\"replace\",\"target_ref\":<ref>,\"new_ref\":<ref>,\"mechanism\":<short text>}}. No quality claims.";

const TOOL: &str = "pulso/lab_query@1.0.0";
pub const TARGET_TOKEN: &str = "prompt.resumen_radicado.v1";

fn observation(seed: &SignalSeed) -> Value {
    json!([{"tool": TOOL, "args": {"metric_id": "synthetic_metric"}, "status": "ok", "error": null,
        "result": {"rows": [{"metric_id": "synthetic_metric", "window_id": "w1", "count": seed.count, "rate": seed.rate(), "evidence_ref": seed.model_evidence_ref()}]}}])
}

fn tools() -> Value {
    json!([{"tool": TOOL, "description": "aggregate lab rows, k-anonymous", "args_schema": {"type": "object"}}])
}

fn request(role: Role, system: &str, goal: &str, seed: &SignalSeed, inputs: Value, observations: Value) -> ModelRequest {
    ModelRequest {
        role,
        system: system.into(),
        payload: json!({"goal": goal, "inputs": inputs, "step": 0, "tools": tools(), "observations": observations, "output_schema": {"type": "object"}}),
        registry: vec![seed.signal_id.clone(), TARGET_TOKEN.into()],
        data_class: DataClass::Synthetic,
    }
}

pub fn scout_request(seed: &SignalSeed) -> ModelRequest {
    request(Role::Scout, SCOUT_SYSTEM, "claim the rate of the signal", seed, json!({"signal_id": seed.signal_id, "metric_id": "synthetic_metric"}), observation(seed))
}

pub fn verifier_request(seed: &SignalSeed, claimed_rate: f64) -> ModelRequest {
    request(Role::Verifier, VERIFIER_SYSTEM, "verify the claimed rate", seed, json!({"signal_id": seed.signal_id, "hypothesis_id": "h_1", "claimed_rate": claimed_rate}), observation(seed))
}

pub fn builder_request(seed: &SignalSeed) -> ModelRequest {
    request(Role::Builder, BUILDER_SYSTEM, "propose one prompt change", seed, json!({"signal_id": seed.signal_id, "hypothesis_id": "h_1", "target_catalogue": [TARGET_TOKEN]}), json!([]))
}
