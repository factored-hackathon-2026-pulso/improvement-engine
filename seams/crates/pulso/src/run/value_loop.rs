//! The value loop of `pulso run` (B3): cells sensor signals -> reasoning (Scout, independent Verifier, Builder, deterministic
//! recompute, compile) -> the registry writer delivering the proposal to agent-core as the `builder` principal.
//!
//! Every corroborated finding ends in one closed outcome kept in the engine job store (one record per finding, plus a summary):
//! `unlinked`, `blocked(<reason>)`, `no_change`, or `proposed` with the writer's delivery (`delivered` + proposal id, or `denied` +
//! closed reason). Nothing here approves, publishes, promotes, freezes or evaluates (the writer's allow-list). Idempotent per
//! finding twice over: a replayed job reuses the per-finding records it already committed (no second model call), and the writer's
//! receipt store (key = evidence + target + kind) never opens a second proposal for the same finding across jobs.
//!
//! Honesty: the sensor is the `claude-standin` cells sensor (real code, labelled stand-in); the baseline label says whether the
//! texts came from the live registry; the models are whatever the ports are (the live ports are the real gateway; there is no
//! scripted fallback in this module); derived aggregates reach a hosted model only with the explicit opt-in. Records hold reason
//! codes, ids and numbers only: never model free text, never a token.
use crate::config::Secret;
use core_client::authorizer::Jws;
use reasoning::catalog::Catalog;
use reasoning::finding::{Finding, Source};
use reasoning::pipeline::{Opts, Ports, reason};
use registry_writer::{Config, Environment, FileStore, HttpTransport, Submission, Transport, Via, Writer};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub type PortsFactory = Arc<dyn Fn() -> Result<Ports, String> + Send + Sync>;
pub type Lookup = dyn Fn(&str) -> Option<String>;

/// Where per-finding records live (the engine job store in `pulso run`; nothing in a one-shot).
pub trait Persist {
    fn get(&self, step: u32) -> Option<String>;
    fn put(&self, step: u32, record: &str) -> Result<(), String>;
}

pub struct NoPersist;
impl Persist for NoPersist {
    fn get(&self, _: u32) -> Option<String> {
        None
    }
    fn put(&self, _: u32, _: &str) -> Result<(), String> {
        Ok(())
    }
}

pub struct ValueLoop {
    pub cells: PathBuf,
    pub source: Source,
    pub allow_derived: bool,
    pub receipts: PathBuf,
    pub via: Via,
    pub environment: Environment,
    /// Label of the credential the registry accepted. LOCAL STACK ONLY: trusting the engine kid through the staff-keys file (or any
    /// stand-in credential) is a property of our own instance; it must never be reused against a shared Core, which has to mint the
    /// `builder` principal itself. A stand-in is declared (`PULSO_REGISTRY_CREDENTIAL=standin`) and travels in every delivery label.
    pub credential: &'static str,
    pub registry_token: Jws,
    pub run_token: Option<Jws>,
    pub transport: Arc<dyn Transport + Send + Sync>,
    pub ports: PortsFactory,
    pub model_label: String,
    /// `PULSO_LOOP_MAX_FINDINGS`: cost bound per run (the first N corroborated findings in sensor order); the rest are counted, not silently dropped.
    pub max_findings: Option<usize>,
}

fn truthy(v: Option<String>) -> bool {
    v.is_some_and(|s| matches!(s.trim(), "1" | "true" | "yes" | "enabled"))
}

impl ValueLoop {
    /// `Ok(None)` when `PULSO_CELLS_NDJSON` is not set (the loop is off). A half-configured loop is refused with the variable named.
    /// The model ports are the REAL gateway (`PULSO_LLM_GATEWAY*`): the loop has no scripted configuration.
    pub fn from_lookup(get: &Lookup, work: Option<&std::path::Path>) -> Result<Option<ValueLoop>, String> {
        let Some(cells) = get("PULSO_CELLS_NDJSON").filter(|v| !v.is_empty()) else { return Ok(None) };
        let source = get("PULSO_CELLS_SOURCE").ok_or("PULSO_CELLS_SOURCE (synthetic|bank|e0) is required with PULSO_CELLS_NDJSON")?;
        let source = Source::parse(&source).ok_or("PULSO_CELLS_SOURCE is not synthetic|bank|e0")?;
        let addr = get("PULSO_REGISTRY_ADDR").filter(|v| !v.is_empty()).ok_or("PULSO_REGISTRY_ADDR (host:port of the agent-core registry) is required with PULSO_CELLS_NDJSON")?;
        let token = Secret::new(get("PULSO_REGISTRY_TOKEN").filter(|v| !v.is_empty()).ok_or("PULSO_REGISTRY_TOKEN (the builder principal) is required with PULSO_CELLS_NDJSON")?);
        let via = match get("PULSO_REGISTRY_VIA").as_deref().unwrap_or("api") {
            "api" => Via::RegistryApi,
            "run" => Via::BuilderRun,
            _ => return Err("PULSO_REGISTRY_VIA is not api|run".into()),
        };
        let environment = match get("PULSO_REGISTRY_ENV").as_deref().unwrap_or("local") {
            "local" => Environment::LocalStack,
            "shared" => Environment::SharedCore,
            _ => return Err("PULSO_REGISTRY_ENV is not local|shared".into()),
        };
        let credential = if get("PULSO_REGISTRY_CREDENTIAL").as_deref() == Some("standin") { "operator-declared stand-in credential (not the engine builder principal)" } else { "engine builder principal" };
        let receipts = get("PULSO_RECEIPTS")
            .map(PathBuf::from)
            .or_else(|| work.map(|w| w.join("registry-receipts.json")))
            .ok_or("PULSO_RECEIPTS or PULSO_WORK_DIR is required (the idempotency receipts of the writer)")?;
        let model_label = get("PULSO_LLM_GATEWAY_MODEL").unwrap_or_else(|| engine::models::llm_gateway::DEFAULT_MODEL.to_string());
        let envs: std::collections::HashMap<String, String> =
            [
                "PULSO_LLM_GATEWAY", "PULSO_LLM_GATEWAY_ADDR", "PULSO_LLM_GATEWAY_KEY", "PULSO_LLM_GATEWAY_MODEL", "PULSO_LLM_GATEWAY_VERIFIER_MODEL", "PULSO_LLM_GATEWAY_ALIAS",
                "PULSO_LLM_GATEWAY_MAX_TOKENS", "PULSO_LLM_GATEWAY_TIMEOUT_S", "PULSO_LLM_GATEWAY_STRUCTURED", "PULSO_LLM_GATEWAY_BUILDER_MODEL", "PULSO_LLM_GATEWAY_BUILDER_ESCALATION_MODEL",
            ]
                .iter()
                .filter_map(|k| get(k).map(|v| (k.to_string(), v)))
                .collect();
        let ports: PortsFactory = Arc::new(move || reasoning::live::ports_from_env(&|k| envs.get(k).cloned()));
        // Fail at start, not at the first job: the gateway setup must be complete.
        ports().map(drop)?;
        Ok(Some(ValueLoop {
            cells: PathBuf::from(cells),
            source,
            allow_derived: truthy(get("PULSO_ALLOW_DERIVED_AGGREGATES")),
            receipts,
            via,
            environment,
            credential,
            registry_token: Jws::new(token.expose().to_string()),
            run_token: get("PULSO_RUN_TOKEN").filter(|v| !v.is_empty()).map(Jws::new),
            transport: Arc::new(HttpTransport::new(&addr, Duration::from_secs(90))),
            ports,
            model_label,
            max_findings: get("PULSO_LOOP_MAX_FINDINGS").and_then(|v| v.parse().ok()),
        }))
    }

    fn writer<'a>(&'a self, store: &'a FileStore) -> Writer<'a> {
        let mut c = Config::new(self.via, self.environment, self.registry_token.clone());
        c.run_token = self.run_token.clone();
        c.credential = self.credential;
        Writer::new(c, &*self.transport, store)
    }

    /// Runs the loop once over the configured cells package. `Err` only for infrastructure that makes the whole job unrunnable
    /// (unreadable cells, sensor failure, ports): the job is then retried. A finding that is blocked or denied is an outcome.
    pub fn run(&self, persist: &dyn Persist) -> Result<Value, String> {
        self.run_as(persist, "value-loop-local")
    }

    /// As `run`; `run_id` is the debug-api run id of this job (`value-loop-<job>`): the session and the trace key of every story.
    pub fn run_as(&self, persist: &dyn Persist, run_id: &str) -> Result<Value, String> {
        let ndjson = std::fs::read_to_string(&self.cells).map_err(|e| format!("cells package: {e}"))?;
        let report: Value = serde_json::from_str(&steps::cells::run(&ndjson).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let (mut findings, skipped) = Finding::from_report(&report, self.source)?;
        let total_corroborated = findings.len();
        if let Some(n) = self.max_findings {
            findings.truncate(n);
        }
        let mut ports: Option<Ports> = None; // built on the first finding that needs a model: a full replay calls none
        let store = FileStore::new(&self.receipts);
        let w = self.writer(&store);
        let refreshed = w.refresh_catalog(&Catalog::bundled());
        let opts = Opts { allow_derived_aggregates: self.allow_derived };
        let mut records: Vec<Value> = vec![];
        for (i, f) in findings.iter().enumerate() {
            let step = u32::try_from(i + 1).unwrap_or(u32::MAX);
            if let Some(prev) = persist.get(step).and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(|r| r["evidence_ref"] == f.evidence_ref().as_str()) {
                let mut prev = prev;
                prev["resumed_from_job_store"] = json!(true);
                records.push(prev);
                continue;
            }
            if ports.is_none() {
                ports = Some((self.ports)()?);
            }
            // one story per finding: every gateway and agent-core call below carries its `traceparent` and `baggage`
            let _story = engine::trace::enter(engine::trace::TraceCtx {
                finding_key: f.evidence_ref(),
                run_id: run_id.to_string(),
                release: std::env::var("PULSO_RELEASE").unwrap_or_default(),
                case_type: f.metric.clone(),
                ..Default::default()
            });
            let r = reason(&refreshed.catalog, f, ports.as_ref().expect("just built"), &opts);
            let mut rec = json!({
                "finding_id": f.id, "evidence_ref": f.evidence_ref(), "metric": f.metric, "status": r.status, "reason": r.reason, "stage": r.stage, "mapping_row": r.mapping_row,
                "rubric": r.rubric.as_ref().map(|x| json!({"total": x["total"], "band": x["band"]})), "independence": r.independence, "metering": r.metering, "builder_tier": r.metering["builder"]["tier"],
                "models": r.calls.iter().map(|c| c["model_id"].clone()).collect::<Vec<_>>(), "doubles": r.doubles.len(), "delivery": null,
            });
            if let Some(c) = &r.compiled_raw {
                rec["target_ref"] = json!(c.target_ref);
                rec["proposal_kind"] = json!(c.kind);
                if r.status == "proposed" {
                    engine::trace::set_stage("deliver", 1);
                    let o = w.deliver(&Submission::new(f, c));
                    rec["delivery"] = o.to_json();
                }
            }
            persist.put(step, &rec.to_string())?;
            records.push(rec);
        }
        let n = |s: &str| records.iter().filter(|r| r["status"] == s).count();
        let mut by_reason = serde_json::Map::new();
        let mut tiers = serde_json::Map::new();
        for r in &records {
            if r["status"] == "unlinked" {
                let k = r["reason"].as_str().unwrap_or("no_mapping").to_string();
                let c = by_reason.get(&k).and_then(Value::as_u64).unwrap_or(0) + 1;
                by_reason.insert(k, json!(c));
            }
            if let Some(t) = r["builder_tier"].as_str().filter(|_| r["status"] == "proposed") {
                let c = tiers.get(t).and_then(Value::as_u64).unwrap_or(0) + 1;
                tiers.insert(t.to_string(), json!(c));
            }
        }
        let delivered = records.iter().filter(|r| r["delivery"]["status"] == "delivered").count();
        let denied = records.iter().filter(|r| r["delivery"]["status"] == "denied").count();
        Ok(json!({
            "contract": "value-loop/b3-0", "sensor": "claude-standin (steps::cells, real code, labelled stand-in)", "data_source": self.source.as_str(),
            "baseline": {"label": refreshed.catalog.label, "live": refreshed.live.len(), "fixture": refreshed.fixture.len()},
            "opt_in_derived_aggregates": self.allow_derived, "models": self.model_label, "quality_claims": "forbidden",
            "summary": {"corroborated": total_corroborated, "reasoned": findings.len(), "skipped_not_corroborated": skipped.len(), "proposed": n("proposed"), "no_change": n("no_change"), "unlinked": n("unlinked"), "blocked": n("blocked"),
                        "delivered": delivered, "denied": denied,
                        // unlinked findings are descriptive with an explicit reason, never a failure; only `blocked` counts against the roles
                        "unlinked_by_reason": by_reason, "failed": n("blocked"),
                        "cost_usd": (records.iter().map(|r| r["metering"]["cost_usd"].as_f64().unwrap_or(0.0)).sum::<f64>() * 1e6).round() / 1e6,
                        "builder_tiers": tiers},
            "findings": records,
            "engine_never_approves_publishes_or_promotes": true,
        }))
    }
}
