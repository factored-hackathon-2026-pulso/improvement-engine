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

/// What the model port contributes to a report: one record per call (`CallRecord::to_json`, in call order) and, when a model
/// answer stopped the job before it ran, `(role, code)` (`code` is `model_refused`, `model_unavailable`, `model_invalid`,
/// `verifier_refuted` or `evidence_insufficient`). `Extras::default()` = no model was consulted (the partial reports of a
/// demo that predates the port).
#[derive(Default, Clone)]
pub struct Extras {
    pub models: Vec<Value>,
    pub stop: Option<(String, String)>,
    /// The Core port is the real one (`CorePort::provenance`): steps 5, 6 and 9 are then `real-narrow` instead of `stand-in`.
    pub core_real: bool,
}

fn model_step(extras: &Extras, role: &str, fallback: String) -> (String, String) {
    let Some(m) = extras.models.iter().find(|m| m["role"] == role) else {
        return (fallback, "scripted".into());
    };
    let blocked = extras.stop.as_ref().filter(|(r, c)| r == role && c == "model_invalid").map(|(_, c)| format!("blocked({c})"));
    (blocked.unwrap_or_else(|| m["status"].as_str().unwrap_or("not_exercised").to_string()), m["provider"].as_str().unwrap_or("scripted").to_string())
}

fn gateway_provenance(models: &[Value]) -> String {
    let real: Vec<&str> = models.iter().filter(|m| m["real"] == true).filter_map(|m| m["model_id"].as_str()).collect();
    let has = |l: &str| models.iter().any(|m| m["label"] == l && m["outcome"] == "answered");
    if !real.is_empty() {
        format!("real-gateway({})", real.join(","))
    } else if has("roleplay") {
        "agent-roleplay(replay of the roleplay-llm queue, not a real model)".into()
    } else if has("local-model") {
        "local-model(an endpoint declared local, not a hosted real model)".into()
    } else if has("scripted") {
        "scripted(no model ran: fixed answers)".into()
    } else if models.is_empty() {
        "not-exercised(no model in the offline Rust thread)".into()
    } else {
        "refused-or-unavailable(no model answered)".into()
    }
}

pub fn build(i: &Input) -> Value {
    build_with(i, &Extras::default())
}

pub fn build_with(i: &Input, x: &Extras) -> Value {
    let out = i.payload.and_then(|p| p.get("out"));
    let get = |k: &str| out.and_then(|o| o.get(k));
    let err = i.error.unwrap_or("");
    let ran = |k: &str| get(k).is_some();
    let ex = |k: &str, status: &str| if ran(k) { status.to_string() } else { "not_exercised".to_string() };
    let compile = get("compile");
    let compiled = compile.and_then(|c| c.get("status")).and_then(Value::as_str) == Some("compiled");
    let gate_v = get("gate").and_then(|g| g.get("verdict")).and_then(Value::as_str).map(str::to_string);
    let validation_v = get("validation").and_then(|v| v.get("verdict")).and_then(Value::as_str).map(str::to_string);

    let narrow = if x.core_real { "real-narrow" } else { "stand-in" };
    let compile_status = if compiled {
        narrow.to_string()
    } else if let Some(r) = compile.and_then(|c| c.get("denied_reason")).and_then(Value::as_str) {
        blocked(r)
    } else if let Some((_, code)) = x.stop.as_ref().filter(|(_, c)| c == "kind_not_supported" || c == "release_settings_not_allowed") {
        blocked(code)
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
        narrow.to_string()
    } else if gate_blocked {
        blocked("gate")
    } else {
        "not_exercised".to_string()
    };
    let publish = get("publish");
    let override_ = auth.and_then(|a| a.get("override")).filter(|o| !o.is_null());

    let scout = model_step(x, "scout", ex("recompute", "stand-in"));
    let opportunity = model_step(x, "builder", ex("validation", "stand-in"));
    let steps = vec![
        step(i, 1, "trigger", "stand-in", "manual-command", json!({"why": "the run was started by hand (the thread10 binary); a scheduled ingest is DEMO-2"})),
        step(i, 2, "signals", &ex("sensors", "stand-in"), "claude-standin", json!({"runner": "synth_runner: fixed aggregate output, reads no data"})),
        step(i, 3, "scout", &scout.0, &scout.1, json!({"model": x.models.iter().find(|m| m["role"] == "scout"), "why": "the claim comes from the model port; see models[] for what answered"})),
        step(i, 3, "recompute", &ex("recompute", "real-narrow"), "claude-standin", json!({"semantics": "claude-standin", "recompute": "Rust recompute step over the synthetic lab row"})),
        step(i, 4, "opportunity", &opportunity.0, &opportunity.1, json!({"model": x.models.iter().find(|m| m["role"] == "builder"), "why": "the change spec comes from the model port; see models[] for what answered"})),
        step(i, 4, "validation", &ex("validation", "real-narrow"), "claude-standin", json!({"verdict": validation_v, "model_verifier": x.models.iter().find(|m| m["role"] == "verifier")})),
        step(i, 5, "compile", &compile_status, if x.core_real { "core-bridge+claude-standin" } else { "claude-standin" }, json!({"dry_run": if x.core_real { "Core dry-run digest via the bridge; the compile step itself is the claude-standin" } else { "offline double digest, not a Core dry-run" }})),
        step(i, 6, "gate", &ex("gate", narrow), if x.core_real { "core-bridge+claude-gsipy" } else { "claude-standin" }, json!({"verdict": gate_v, "arms": if x.core_real { "real Core arm runs (oracle: the suite's own expect blocks)" } else { "offline double: both sides complete the same cases" }, "judge": JUDGE})),
        step(i, 7, "revision", "not_exercised", "claude-standin", json!({"why": "V3r bounded revision is a library hook, not wired into this job"})),
        step(i, 8, "approval", &approval_status, "simulated-issuer", json!({"authority": auth})),
        step(i, 9, "publish", &publish_status, if x.core_real { "core-registry+simulated-issuer" } else { "claude-standin" }, json!({"registry": if x.core_real { "Core registry, staging alias readback" } else { "offline double" }, "publish": publish})),
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
            {"port": "core", "provenance": if x.core_real { "real-core-live(engine::live_core::LiveCore over core-client; the platform sealer and the human issuer are stand-ins)" } else { "offline-double(thread10::DoublePort)" }, "price_source": "n/a"},
            {"port": "llm_gateway", "provenance": gateway_provenance(&x.models), "price_source": "n/a"},
            {"port": "registry", "provenance": "in-process-double", "price_source": "n/a"},
            {"port": "human_issuer", "provenance": "simulated-local-issuer", "price_source": "n/a"},
            {"port": "platform", "provenance": "platform-sim", "price_source": "n/a"}],
        "authors": {"world": "claude-wrld0", "suite": "agent-core-registry-demo@c814c2b", "effect": "claude-p2py-effects", "judge": JUDGE,
                    "suite_sealed_at": SUITE_SEALED_AT, "candidate_created_at": if compiled { json!(CANDIDATE_CREATED_AT) } else { Value::Null }},
        "events": i.events,
        "models": x.models,
    });
    if !overrides.is_empty() {
        r["overrides"] = json!(overrides);
    }
    r
}
