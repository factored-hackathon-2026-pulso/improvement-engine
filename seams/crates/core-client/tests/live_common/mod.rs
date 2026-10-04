//! Shared plumbing of the live (ignored) tests: env contract, the e2e fixtures double (`/_e2e/config`), draft loading.
//! Env values (service seed, human seed, tokens) are never printed. Set them with `seams/scripts/live-env.ps1`.
#![allow(dead_code)]
use core_client::authoring::Change;
use core_client::evaluate::{BindingPreauthorizer, SuiteRef};
use core_client::writer::{ArtifactSealer, DraftPlan, WriterRun};
use core_client::{ClientConfig, CoreClient};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("{k} is not set: run seams/scripts/live-env.ps1 against a kept e2e stack"))
}

pub fn seed(var: &str) -> [u8; 32] {
    let hex = env(var);
    let mut s = [0u8; 32];
    for (i, b) in s.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex seed");
    }
    s
}

pub fn tenant() -> String {
    env("PULSO_LIVE_TENANT")
}

pub fn agent() -> String {
    std::env::var("PULSO_LIVE_AGENT").unwrap_or_else(|_| "atencion-tarea".into())
}

pub fn client(worker: &str) -> CoreClient {
    let mut cfg = ClientConfig::new(&env("PULSO_BRIDGE_ADDR"), &env("PULSO_SERVICE_KID"), seed("PULSO_SERVICE_SEED_HEX"), worker);
    cfg.timeout = Duration::from_secs(120);
    CoreClient::new(cfg)
}

/// The e2e fixtures double: the platform stand-in that holds sealed artifacts and issues bindings. Neither is an
/// `/internal/v1` route; this is the BRG1 gap candidate (see docs/reports/brg1-seal-and-bind-request.md).
pub struct Fx {
    pub addr: String,
    pub tenant: String,
}

impl Fx {
    pub fn from_env() -> Fx {
        Fx { addr: env("PULSO_E2E_FX_ADDR"), tenant: tenant() }
    }
    pub fn config(&self, body: &Value) -> Result<(), String> {
        let r = core_client::http::request(&self.addr, "POST", "/_e2e/config", &[], Some(&serde_json::to_vec(body).unwrap()), Duration::from_secs(10)).map_err(|e| format!("{e:?}"))?;
        if r.status == 200 { Ok(()) } else { Err(format!("fixtures server answered {}", r.status)) }
    }
}

impl Fx {
    /// The seeded task agent's only model call is its closing reply: one scripted answer for any prompt (no PII, no
    /// digits), exactly the stand-in's `RESPOND_RULE`. Without it the native evaluation ends `failed_infra`.
    pub fn script_closing_reply(&self) -> Result<(), String> {
        self.config(&json!({"llm_replace": true, "llm_rules": [{"id": "live-respond", "match": {}, "repeat_last": true,
            "responses": [{"text": "Recibimos tu disputa y la estamos revisando.", "citations": []}]}]}))
    }
}

impl ArtifactSealer for Fx {
    fn seal(&self, plan_ref: &str, content: &Value) -> Result<(), String> {
        self.config(&json!({"artifacts": [{"tenant": self.tenant, "id": plan_ref, "content": content}]}))
    }
}

impl BindingPreauthorizer for Fx {
    fn preauthorize(&self, tenant: &str, binding_ref: &str) -> Result<(), String> {
        self.config(&json!({"preauthorized_bindings": [{"tenant": tenant, "binding_ref": binding_ref}]}))
    }
}

pub struct Variant {
    pub plan_changes: Vec<Change>,
    pub suite: SuiteRef,
}

pub fn variant(name: &str) -> Variant {
    let fx: Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../core-client/tests/fixtures/live_attention_task.json")).unwrap()).unwrap();
    let v = &fx["variants"][name];
    let changes = v["changes"].as_array().expect(name).iter().map(|c| Change::new(c["kind"].as_str().unwrap(), c["content"].clone(), c["docs"].clone())).collect();
    Variant { plan_changes: changes, suite: SuiteRef { id: v["suite_id"].as_str().unwrap().into(), version: v["suite_version"].as_str().unwrap().into(), digest: v["suite_digest"].as_str().unwrap().into() } }
}

pub fn plan(v: &Variant, tag: &str) -> DraftPlan {
    DraftPlan::new(&agent(), &format!("pulso-key:{tag}"), v.plan_changes.clone())
}

pub fn writer_run(tag: &str) -> WriterRun {
    WriterRun {
        tenant_id: tenant(),
        job_id: format!("job-{tag}"),
        logical_key: format!("{tag}-writer"),
        pulso_run_ref: format!("pr-job-{tag}"),
        lab_grant_ref: "grant-contract".into(),
        writer_release_id: env("PULSO_LIVE_WRITER_RELEASE"),
        writer_agent_version: "1.0.0".into(),
    }
}

/// A unique tag per process run so a stack can host several windows.
pub fn tag(prefix: &str) -> String {
    let ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    format!("{prefix}-{}", ns % 1_000_000_000_000)
}

pub struct Timer(Vec<(String, f64)>);
impl Timer {
    pub fn new() -> Timer {
        Timer(vec![])
    }
    pub fn time<T>(&mut self, name: &str, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let r = f();
        self.0.push((name.into(), (t.elapsed().as_secs_f64() * 1000.0).round() / 1000.0));
        r
    }
    pub fn json(&self) -> Value {
        Value::Array(self.0.iter().map(|(n, s)| json!({"step": n, "seconds": s})).collect())
    }
}

pub fn evidence(name: &str, v: &Value) {
    eprintln!("EVIDENCE {name} {}", serde_json::to_string(v).unwrap());
    if let Ok(dir) = std::env::var("PULSO_LIVE_EVIDENCE_DIR") {
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.json")), serde_json::to_string_pretty(v).unwrap()).unwrap();
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`, `hours` from now.
pub fn deadline(hours: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    core_client::authorizer::iso_z(now + hours * 3600).replace(".000Z", "Z")
}
