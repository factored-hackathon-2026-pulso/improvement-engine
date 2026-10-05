//! The value loop of `pulso run` (B3): cells sensor signals -> reasoning (Scout, independent Verifier, Builder, deterministic
//! recompute, compile) -> the registry writer delivering the proposal to agent-core as the `builder` principal.
//!
//! Every corroborated finding ends in one closed outcome kept in the engine job store (one record per finding, plus a summary):
//! `unlinked`, `blocked(<reason>)`, `no_change`, or `proposed` with the writer's delivery (`delivered` + proposal id, or `denied` +
//! closed reason). W11 (`ProofConfig`, on for the live path): a proposed finding is first PROVEN on agent-core (regression suite
//! fails on the base, passes on the candidate, `registry_writer::proof`) and only then announced; otherwise the record says
//! `outcome: not_announced:<verdict>` with the dossier and the verdict story, delivery null. Nothing here approves, publishes or
//! promotes (the writer's allow-list admits only `freeze` and `evaluate` of manual-origin evaluation drafts). Idempotent per
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
use reasoning::mapping::Caps;
use reasoning::pipeline::{Opts, Ports, candidate_plan, reason_candidate};
use registry_writer::eval::EvalOptions;
use registry_writer::proof::{FileProofStore, ProofInput, PythonScripts, Scripts, announce_submission, prove};
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

/// The `pulso.model_call/1` records of finding `step` live in the same job store under `CALLS_STEP_BASE + step` (a JSON array; kept apart from the
/// finding record, which carries no model free text). Works for every `Persist` (memory, file, postgres) without a table of its own.
pub const CALLS_STEP_BASE: u32 = 1_000_000;

pub struct NoPersist;
impl Persist for NoPersist {
    fn get(&self, _: u32) -> Option<String> {
        None
    }
    fn put(&self, _: u32, _: &str) -> Result<(), String> {
        Ok(())
    }
}

/// W11 "evaluate before announce". `None` in `ValueLoop::proof` = off (unit tests); `from_lookup` turns it ON by default for the
/// live path (`PULSO_EVAL_BEFORE_ANNOUNCE=off` disables it; `via=run` cannot be proven and keeps it off).
pub struct ProofConfig {
    /// build_suite.py and judge_story.py (subprocess, stdin/stdout JSON; PyYAML needed by build_suite).
    pub scripts: Arc<dyn Scripts + Send + Sync>,
    /// Same registry, long timeout: `evaluate` runs the real engine and the real JEV and takes minutes.
    pub eval_transport: Arc<dyn Transport + Send + Sync>,
    pub opts: EvalOptions,
    /// Conclusive proofs by (finding key, candidate digest): a replay makes no request.
    pub proofs: PathBuf,
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
    /// `PULSO_LOOP_MAX_EXPLORATORY` (default 2): how many `candidate_exploratory` findings (labelled, never corroborated) join the loop per
    /// run, after the corroborated ones. 0 = strict sensor only.
    pub max_exploratory: usize,
    pub proof: Option<ProofConfig>,
    /// ANN1: tells the support platform about an `announced` proposal AFTER agent-core accepted it (best effort, never fails the delivery).
    /// `PULSO_ANNOUNCE_TO_PLATFORM` / `PULSO_PLATFORM_URL` / `PULSO_PLATFORM_SERVICE_TOKEN`; default OFF.
    pub announcer: Option<registry_writer::announce::Announcer>,
    /// MAP1: what the engine can announce today (a NEW agent needs an admin credential for the release settings of its evaluation draft:
    /// `PULSO_NEW_AGENT_ADMIN=1`, default off). Decides the order in which a finding's candidates are tried.
    pub caps: Caps,
}

/// `scripts/regression` of the working directory, else of the checkout the binary was built from.
fn default_script_dir() -> PathBuf {
    let local = PathBuf::from("scripts/regression");
    if local.join("judge_story.py").exists() { local } else { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/regression") }
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
        let work_dir = work.map(PathBuf::from).or_else(|| receipts.parent().map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("."));
        let proof = match get("PULSO_EVAL_BEFORE_ANNOUNCE").as_deref().unwrap_or("") {
            "off" | "0" | "false" => None,
            "on" | "1" | "true" if via == Via::BuilderRun => {
                return Err("PULSO_EVAL_BEFORE_ANNOUNCE=on needs PULSO_REGISTRY_VIA=api: a builder-run draft is authored by the agent and cannot be proven here".into());
            }
            "" if via == Via::BuilderRun => None,
            "" | "on" | "1" | "true" => {
                let script_dir = get("PULSO_REGRESSION_SCRIPTS").map(PathBuf::from).unwrap_or_else(default_script_dir);
                let python: Vec<String> = get("PULSO_REGRESSION_PYTHON").unwrap_or_else(|| "python".into()).split_whitespace().map(str::to_string).collect();
                if python.is_empty() {
                    return Err("PULSO_REGRESSION_PYTHON is blank".into());
                }
                let secs = get("PULSO_EVAL_TIMEOUT_SECS").and_then(|v| v.parse().ok()).unwrap_or(900);
                let w11 = work_dir.join("w11");
                std::fs::create_dir_all(&w11).map_err(|e| format!("work dir for the proof: {e}"))?;
                Some(ProofConfig {
                    scripts: Arc::new(PythonScripts { python, script_dir, work: w11, env: vec![] }),
                    eval_transport: Arc::new(HttpTransport::new(&addr, Duration::from_secs(secs))),
                    opts: EvalOptions::default(),
                    proofs: work_dir.join("w11-proofs.json"),
                })
            }
            _ => return Err("PULSO_EVAL_BEFORE_ANNOUNCE is not on|off".into()),
        };
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
            max_exploratory: get("PULSO_LOOP_MAX_EXPLORATORY").and_then(|v| v.parse().ok()).unwrap_or(2),
            proof,
            announcer: registry_writer::announce::Announcer::from_lookup(get)?,
            caps: Caps { new_agent_admin: truthy(get("PULSO_NEW_AGENT_ADMIN")) },
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
        let sensor = if self.max_exploratory > 0 { steps::cells::run_exploratory(&ndjson) } else { steps::cells::run(&ndjson) };
        let report: Value = serde_json::from_str(&sensor.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let (mut findings, skipped) = Finding::from_report_with(&report, self.source, self.max_exploratory)?;
        let total_corroborated = findings.iter().filter(|f| !f.is_exploratory()).count();
        if let Some(n) = self.max_findings {
            // the cap bounds the corroborated findings; the exploratory ones follow within their own cap
            let mut kept = 0usize;
            findings.retain(|f| f.is_exploratory() || { kept += 1; kept <= n });
        }
        let mut ports: Option<Ports> = None; // built on the first finding that needs a model: a full replay calls none
        let store = FileStore::new(&self.receipts);
        let w = self.writer(&store);
        let ew = self.proof.as_ref().map(|p| {
            let mut c = Config::new(self.via, self.environment, self.registry_token.clone());
            c.credential = self.credential;
            Writer::new(c, &*p.eval_transport, &store)
        });
        let proofs = self.proof.as_ref().map(|p| FileProofStore::new(&p.proofs));
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
            let mut call_records: Vec<Value> = vec![];
            let plan = candidate_plan(f, self.caps);
            let row = reasoning::pipeline::row_of(f);
            // MAP1: candidates are tried in rank order (at most MAX_CANDIDATES) and the loop stops at the first PROVEN one. A finding with no
            // candidate (unlinked, human owned, direction not up) still gets one attempt so that its typed outcome is recorded.
            let tries: Vec<Option<String>> = if plan.is_empty() { vec![None] } else { plan.iter().cloned().map(Some).collect() };
            let (mut recs, mut attempts, mut tried, mut cost) = (vec![], vec![], vec![], 0.0f64);
            for cand in &tries {
                let (r_rec, r) = self.attempt(&w, ew.as_ref(), proofs.as_ref(), &refreshed, f, ports.as_ref().expect("just built"), &opts, cand.as_deref());
                append_calls(&mut call_records, &r.call_records);
                cost += r_rec["metering"]["cost_usd"].as_f64().unwrap_or(0.0);
                if let Some(t) = cand {
                    tried.push(t.clone());
                }
                attempts.push(json!({"rank": r.candidate.as_ref().map(|c| c["rank"].clone()), "target_ref": cand, "status": r_rec["status"], "reason": r_rec["reason"], "outcome": r_rec["outcome"],
                                     "proof": r_rec["evaluation"]["verdict"].clone(), "delivery": r_rec["delivery"]["status"].clone(), "cost_usd": r_rec["metering"]["cost_usd"]}));
                let proven = r_rec["outcome"] == "announced";
                let ended = !matches!(r_rec["status"].as_str(), Some("proposed" | "blocked"));
                recs.push(r_rec);
                if proven || ended {
                    break;
                }
            }
            // The record of the finding is the proven attempt; else the first attempt that produced a proposal (the most informative
            // non-proven one); else the first. `attempts` lists every one.
            let pick = recs.iter().position(|r| r["outcome"] == "announced").or_else(|| recs.iter().position(|r| r["status"] == "proposed" || r["status"] == "needs_owner_ack")).unwrap_or(0);
            let mut rec = recs.swap_remove(pick);
            rec["candidates"] = row.as_ref().map_or(Value::Null, |r| json!(r.candidate_list(self.caps, &tried)));
            rec["attempts"] = json!(attempts);
            rec["mapping_claim"] = json!("hypothesis_of_where_to_intervene_not_a_cause");
            rec["metering"]["cost_usd"] = json!((cost * 1e6).round() / 1e6);
            if !call_records.is_empty() {
                // before the finding record: a replayed finding either has both or is reasoned again
                persist.put(CALLS_STEP_BASE.saturating_add(step), &Value::Array(call_records).to_string())?;
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
        let announced = records.iter().filter(|r| r["outcome"] == "announced").count();
        let not_announced = records.iter().filter(|r| r["outcome"].as_str().is_some_and(|o| o.starts_with("not_announced:"))).count();
        Ok(json!({
            "contract": "value-loop/b3-0", "sensor": "claude-standin (steps::cells, real code, labelled stand-in)", "data_source": self.source.as_str(),
            "baseline": {"label": refreshed.catalog.label, "live": refreshed.live.len(), "fixture": refreshed.fixture.len()},
            "opt_in_derived_aggregates": self.allow_derived, "models": self.model_label, "quality_claims": "forbidden",
            "summary": {"corroborated": total_corroborated, "reasoned": findings.len(), "skipped_not_corroborated": skipped.len(), "proposed": n("proposed"), "no_change": n("no_change"), "unlinked": n("unlinked"), "human_owned": n("human_owned"), "policy_hypothesis": n("policy_hypothesis"), "needs_owner_ack": n("needs_owner_ack"), "blocked": n("blocked"),
                        "delivered": delivered, "denied": denied, "announced": announced, "not_announced": not_announced,
                        // unlinked findings are descriptive with an explicit reason, never a failure; only `blocked` counts against the roles
                        "unlinked_by_reason": by_reason, "failed": n("blocked"),
                        "cost_usd": (records.iter().map(|r| r["metering"]["cost_usd"].as_f64().unwrap_or(0.0)).sum::<f64>() * 1e6).round() / 1e6,
                        "builder_tiers": tiers},
            "evaluate_before_announce": if self.proof.is_some() { "on" } else { "off" },
            "findings": records,
            "engine_never_approves_publishes_or_promotes": true,
        }))
    }

    /// One attempt of one finding on one candidate (`None` = the whole row, only for findings with no candidate): the roles, then the
    /// proof and, when proven, the delivery.
    #[allow(clippy::too_many_arguments)]
    fn attempt(
        &self, w: &Writer<'_>, ew: Option<&Writer<'_>>, proofs: Option<&FileProofStore>, refreshed: &registry_writer::baseline::Refreshed, f: &Finding, ports: &Ports, opts: &Opts, only: Option<&str>,
    ) -> (Value, reasoning::pipeline::Reasoned) {
            let r = reason_candidate(&refreshed.catalog, f, ports, opts, only);
            let mut rec = json!({
                "finding_id": f.id, "evidence_ref": f.evidence_ref(), "metric": f.metric, "dims": f.dims, "status": r.status, "reason": r.reason, "stage": r.stage, "mapping_row": r.mapping_row,
                "rubric": r.rubric.as_ref().map(|x| json!({"total": x["total"], "band": x["band"]})), "independence": r.independence, "metering": r.metering, "builder_tier": r.metering["builder"]["tier"],
                "models": r.calls.iter().map(|c| c["model_id"].clone()).collect::<Vec<_>>(), "doubles": r.doubles.len(), "delivery": null,
                "candidate": r.candidate, "human_owned": r.human_owned,
            });
            if let Some(c) = &r.compiled_raw {
                rec["target_ref"] = json!(c.target_ref);
                rec["proposal_kind"] = json!(c.kind);
                if r.status == "proposed" || r.status == "needs_owner_ack" {
                    engine::trace::set_stage("deliver", 1);
                    match (&self.proof, ew, proofs) {
                        (Some(pc), Some(ew), Some(ps)) => {
                            let inp = ProofInput {
                                finding: f,
                                compiled: c,
                                attempts: vec![],
                                base_artifact: refreshed.catalog.get(&c.target_ref),
                                labels: reasoning::dossier::Labels { runtime: reasoning::dossier::Runtime::Real, ..Default::default() },
                                doubles: json!(r.doubles),
                                rubric: r.rubric.clone().unwrap_or(Value::Null),
                            };
                            let proof = prove(ew, &*pc.scripts, ps, &pc.opts, &inp);
                            rec["evaluation"] = proof.record();
                            rec["outcome"] = json!(proof.outcome);
                            if proof.announce {
                                let o = w.deliver(&announce_submission(f, c, &proof));
                                if !o.delivered() {
                                    rec["outcome"] = json!(format!("proven_not_delivered:{}", o.reason.map_or("unknown", registry_writer::Reason::code)));
                                }
                                rec["delivery"] = o.to_json();
                                // ANN1: only an announced proposal that agent-core accepted; the outcome is a record, never a failure of the delivery.
                                if let (true, Some(a), Some(id)) = (o.delivered(), &self.announcer, o.proposal_id.as_deref()) {
                                    let outcome = a.announce(f, id, &proof.dossier);
                                    rec["platform_announce"] = json!(outcome.record());
                                    if let Some(e) = &outcome.evidence {
                                        rec["platform_evidence"] = json!(e);
                                    }
                                }
                            }
                        }
                        // ART2: a draft that waits for its owner is never delivered without a proof
                        _ if r.status == "proposed" => {
                            let o = w.deliver(&Submission::new(f, c));
                            rec["delivery"] = o.to_json();
                        }
                        _ => {}
                    }
                }
            }
        (rec, r)
    }

}

/// Appends the call records of one candidate attempt to the finding's. Each attempt numbers its calls from 1 per role; the store keys a call by
/// (evidence_ref, role, n), so the numbers (and the generation span ids derived from them) continue across the attempts of one finding.
fn append_calls(all: &mut Vec<Value>, fresh: &[Value]) {
    let mut next: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for c in all.iter() {
        let e = next.entry(c["role"].as_str().unwrap_or("").to_string()).or_insert(0);
        *e = (*e).max(c["n"].as_u64().unwrap_or(0));
    }
    for c in fresh {
        let mut c = c.clone();
        let role = c["role"].as_str().unwrap_or("").to_string();
        let n = next.get(&role).copied().unwrap_or(0) + 1;
        next.insert(role.clone(), n);
        c["n"] = json!(n);
        if let Some(t) = c["trace_id"].as_str().map(str::to_string) {
            c["span_id"] = json!(core_client::trace::generation_span_id(&t, &role, u32::try_from(n).unwrap_or(u32::MAX)));
        }
        all.push(c);
    }
}

/// Every model call recorded for the first `findings` findings of a job, in finding order (what `run_as` persisted, also by an earlier attempt).
pub fn recorded_calls(persist: &dyn Persist, findings: usize) -> Vec<Value> {
    (1..=u32::try_from(findings).unwrap_or(0))
        .filter_map(|step| persist.get(CALLS_STEP_BASE.saturating_add(step)))
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .flat_map(|v| v.as_array().cloned().unwrap_or_default())
        .collect()
}
