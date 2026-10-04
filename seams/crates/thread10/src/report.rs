//! Ten report steps derived from what the job COMMITTED (never from what was configured). Vocabulary of C-2: a step is
//! never `real` here (the Core is a double, the sensor is the fixed-output `synth_runner`, the judge is the GSIpy
//! stand-in, the human is simulated); a step that did not run says `not_exercised`, a refusal says `blocked(<reason>)`.
use serde_json::{Value, json};

pub const CONTRACT_REVISION: &str = "engine-run/c2-1";
pub const JUDGE: &str = "claude-gsipy";
pub const SUITE_SEALED_AT: &str = "2026-10-04T12:00:00Z";
pub const CANDIDATE_CREATED_AT: &str = "2026-10-04T12:00:10Z";

pub struct Input<'a> {
    pub sha: &'a str,
    pub payload: Option<&'a Value>,
    pub events: &'a [String],
    pub error: Option<&'a str>,
}

fn step(i: &Input, n: u64, id: &str, status: &str, provider: &str, detail: Value) -> Value {
    json!({"n": n, "id": id, "status": status, "data_class": "generated_sample", "target": "local", "sha": i.sha,
           "contract_revision": CONTRACT_REVISION, "host": "rust", "receipt": {"provider": provider}, "detail": detail})
}

fn blocked(reason: &str) -> String {
    format!("blocked({reason})")
}

pub fn build(i: &Input) -> Value {
    let out = i.payload.and_then(|p| p.get("out"));
    let get = |k: &str| out.and_then(|o| o.get(k));
    let err = i.error.unwrap_or("");
    let ran = |k: &str| get(k).is_some();
    let ex = |k: &str, status: &str| if ran(k) { status.to_string() } else { "not_exercised".to_string() };
    let compile = get("compile");
    let compiled = compile.and_then(|c| c.get("status")).and_then(Value::as_str) == Some("compiled");
    let gate_v = get("gate").and_then(|g| g.get("verdict")).and_then(Value::as_str).map(str::to_string);
    let validation_v = get("validation").and_then(|v| v.get("verdict")).and_then(Value::as_str).map(str::to_string);

    let compile_status = if compiled {
        "stand-in".to_string()
    } else if let Some(r) = compile.and_then(|c| c.get("denied_reason")).and_then(Value::as_str) {
        blocked(r)
    } else if err.contains("blocked: validation") {
        blocked("validation")
    } else {
        "not_exercised".to_string()
    };
    let gate_blocked = err.contains("blocked(gate)");
    let auth = get("authority");
    let approval_status = if auth.is_some() {
        "simulated".to_string()
    } else if gate_blocked {
        blocked("gate")
    } else {
        "not_exercised".to_string()
    };
    let publish_status = if ran("publish") {
        "stand-in".to_string()
    } else if gate_blocked {
        blocked("gate")
    } else {
        "not_exercised".to_string()
    };
    let publish = get("publish");
    let override_ = auth.and_then(|a| a.get("override")).filter(|o| !o.is_null());

    let steps = vec![
        step(i, 1, "trigger", "stand-in", "manual-command", json!({"why": "the run was started by hand (the thread10 binary); a scheduled ingest is DEMO-2"})),
        step(i, 2, "signals", &ex("sensors", "stand-in"), "claude-standin", json!({"runner": "synth_runner: fixed aggregate output, reads no data"})),
        step(i, 3, "scout", &ex("recompute", "stand-in"), "scripted", json!({"why": "the scout claim is a scripted value of the synthetic job spec; no model ran"})),
        step(i, 3, "recompute", &ex("recompute", "real-narrow"), "claude-standin", json!({"semantics": "claude-standin", "recompute": "Rust recompute step over the synthetic lab row"})),
        step(i, 4, "opportunity", &ex("validation", "stand-in"), "scripted", json!({"why": "the change spec is a fixed value of the synthetic job spec; no builder ran"})),
        step(i, 4, "validation", &ex("validation", "real-narrow"), "claude-standin", json!({"verdict": validation_v})),
        step(i, 5, "compile", &compile_status, "claude-standin", json!({"dry_run": "offline double digest, not a Core dry-run"})),
        step(i, 6, "gate", &ex("gate", "stand-in"), "claude-standin", json!({"verdict": gate_v, "arms": "offline double: both sides complete the same cases", "judge": JUDGE})),
        step(i, 7, "revision", "not_exercised", "claude-standin", json!({"why": "V3r bounded revision is a library hook, not wired into this job"})),
        step(i, 8, "approval", &approval_status, "simulated-issuer", json!({"authority": auth})),
        step(i, 9, "publish", &publish_status, "claude-standin", json!({"registry": "offline double", "publish": publish})),
        step(i, 10, "observation", &ex("publish", "simulated"), "platform-sim", json!({"window": "simulated"})),
    ];
    let mut overrides = vec![];
    if let (Some(o), Some(v)) = (override_, gate_v.as_deref()) {
        overrides.push(json!({"step": "approval", "of": "gate", "verdict": v, "by": o["by"], "label": o["label"], "reason": o["reason"], "actor": o["actor"], "simulated": true}));
    }
    let mut r = json!({
        "contract_revision": CONTRACT_REVISION, "target": "local", "sha": i.sha, "host": "rust", "label": "DEMO-0", "quality_claims": "forbidden",
        "gate": {"verdict": gate_v, "judge": JUDGE}, "steps": steps,
        "ports": [
            {"port": "core", "provenance": "offline-double(thread10::DoublePort)", "price_source": "n/a"},
            {"port": "llm_gateway", "provenance": "not-exercised(no model in the offline Rust thread)", "price_source": "n/a"},
            {"port": "registry", "provenance": "in-process-double", "price_source": "n/a"},
            {"port": "human_issuer", "provenance": "simulated-local-issuer", "price_source": "n/a"},
            {"port": "platform", "provenance": "platform-sim", "price_source": "n/a"}],
        "authors": {"world": "claude-wrld0", "suite": "agent-core-registry-demo@c814c2b", "effect": "claude-p2py-effects", "judge": JUDGE,
                    "suite_sealed_at": SUITE_SEALED_AT, "candidate_created_at": if compiled { json!(CANDIDATE_CREATED_AT) } else { Value::Null }},
        "events": i.events,
    });
    if !overrides.is_empty() {
        r["overrides"] = json!(overrides);
    }
    r
}
