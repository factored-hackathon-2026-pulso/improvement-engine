//! The worker's `JobRunner` of `pulso run`: the engine job of one monitor tick.
//!
//! A job keyed `monitor:<run_id>` points at the run record `monitor::tick` wrote under `<work>/runs`. Every signal the sensor
//! admitted becomes one proposal through `thread10::pipeline::run_signals` (scout, verifier, builder through the configured
//! `ModelPort`; the Core through the `CorePort`), every proposal ends in a verdict recorded in the ledger (an engine job store
//! under `<work>/pipeline/<run_id>`), and the run, its profile, doubles, panels and `proposal_verdict` events go into the
//! debug-api store in process (`RunEventSink`), where the console reads them.
//!
//! Honesty: the sensor is whatever `semantics` the record says (`rust-events` is real code over the event package; `claude-standin`
//! is a fixed-output stand-in); the models are labelled by the port that answered (scripted by default, never real unless the
//! gateway answered); the Core is the offline double unless `PULSO_CORE_PORT=live`; the human, the release and the observation
//! are simulated; a source the operator declares simulated is declared as a double. A tick with no admitted signal is a visible
//! run with no proposal, and nothing is ever promoted to `viable` here (the verdict comes from the pipeline, derived from what the
//! job committed).
use crate::config::{DataMode, ModelPortKind, Provenance, RunConfig};
use crate::doubles::generate;
use crate::run::models::{MODEL_ID, ObservingScripted};
use crate::run::source::JOB_KEY_PREFIX;

/// Key prefix of the jobs the automation trigger endpoint admits (`trigger:<trigger_key>`).
pub const TRIGGER_KEY_PREFIX: &str = "trigger:";
use crate::run::outcome::{OutcomeStep, find_trigger};
use crate::run::tasks::{JobCtx, JobRunner};
use crate::run::value_loop::{Persist, ValueLoop};
use debug_api::ingest::double_item;
use debug_api::panels::project;
use debug_api::store::now_iso;
use debug_api::{NewEvent, RunEventSink, Store};
use engine::live::CorePort;
use engine::models::gateway::Gateway;
use engine::models::roleplay::Roleplay;
use engine::models::{DataClass, ModelPort};
use engine::real_core::core_from_env;
use pg::repo::{Claimed, RepoError};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use thread10::SignalSeed;
use thread10::pipeline::{PipelineOpts, run_signals};
use thread10::report::{Extras, Input, build_with};

type Env = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub struct EngineRunner {
    work: PathBuf,
    runner_exe: PathBuf,
    store: Arc<Store>,
    data_mode: DataMode,
    provenance: Provenance,
    model_port: ModelPortKind,
    roleplay_queue: Option<PathBuf>,
    core_live: bool,
    env: Env,
    value_loop: Option<Arc<ValueLoop>>,
    outcome: Option<Arc<OutcomeStep>>,
}

fn opaque(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

impl EngineRunner {
    pub fn new(c: &RunConfig, work: &Path, runner_exe: &Path, store: Arc<Store>) -> Result<EngineRunner, String> {
        EngineRunner::with_env(c, work, runner_exe, store, Arc::new(|k| std::env::var(k).ok()))
    }

    /// As `new`, with the environment the gateway and the live Core read their settings from (tests inject it).
    pub fn with_env(c: &RunConfig, work: &Path, runner_exe: &Path, store: Arc<Store>, env: Env) -> Result<EngineRunner, String> {
        let r = EngineRunner {
            work: work.to_path_buf(),
            runner_exe: runner_exe.to_path_buf(),
            store,
            data_mode: c.data_mode,
            provenance: c.provenance,
            model_port: c.model_port,
            roleplay_queue: c.roleplay_queue.clone(),
            core_live: c.core_live,
            env,
            value_loop: None,
            outcome: None,
        };
        // Fail at start, not at the first job: a live Core that cannot be configured, a gateway with an incomplete setup.
        r.core()?;
        r.model()?;
        Ok(r)
    }

    /// Registers the value loop (cells -> reasoning -> registry writer): it runs for `trigger:*` jobs and after every monitor tick.
    pub fn with_value_loop(mut self, v: Option<Arc<ValueLoop>>) -> EngineRunner {
        self.value_loop = v;
        self
    }

    /// Registers the outcome step (OUT1): an `outcome` trigger of type `release.*` runs it instead of the value loop.
    pub fn with_outcome(mut self, o: Option<Arc<OutcomeStep>>) -> EngineRunner {
        self.outcome = o;
        self
    }

    fn core(&self) -> Result<Option<Rc<dyn CorePort>>, String> {
        let env = self.env.clone();
        let live = self.core_live;
        core_from_env(&move |k| if k == "PULSO_CORE_PORT" { Some(if live { "live" } else { "double" }.to_string()) } else { env(k) })
    }

    fn model(&self) -> Result<Rc<dyn ModelPort>, String> {
        Ok(match self.model_port {
            ModelPortKind::Scripted => Rc::new(ObservingScripted) as Rc<dyn ModelPort>,
            ModelPortKind::Roleplay => Rc::new(Roleplay::new(self.roleplay_queue.as_deref().ok_or("PULSO_ROLEPLAY_QUEUE is not set")?)) as Rc<dyn ModelPort>,
            ModelPortKind::Gateway => {
                let env = self.env.clone();
                Rc::new(Gateway::from_env(&move |k| env(k))?) as Rc<dyn ModelPort>
            }
        })
    }

    fn models_desc(&self) -> String {
        match self.model_port {
            ModelPortKind::Scripted => format!("scripted ({MODEL_ID}: fixed rules, no model ran)"),
            ModelPortKind::Roleplay => "roleplay replay (queue of recorded answers, not a real model)".into(),
            ModelPortKind::Gateway => "gateway (a real model only when the gateway answers; a refusal or an outage is a not_evaluable verdict, never a scripted fallback)".into(),
        }
    }

    fn core_desc(&self) -> &'static str {
        if self.core_live { "live (real Core over the bridge; the human issuer and the sealer stay stand-ins)" } else { "offline-double" }
    }

    /// The monitor labels a platform source "real platform signals". When the operator declared the source simulated, or declared nothing,
    /// that label would be false or unproven: it is replaced here, in the title, the profile and the doubles.
    fn label(&self, rec: &Value) -> String {
        match (self.data_mode, self.provenance) {
            (DataMode::Platform, Provenance::Simulated) => "simulated platform-shaped signals (the source is declared simulated by the operator; this is not the actual platform); release and observation simulated".into(),
            (DataMode::Platform, Provenance::Unspecified) => "platform-mode signals; the operator did not declare whether the source is real or simulated (PULSO_SOURCE_PROVENANCE); release and observation simulated".into(),
            _ => rec["label"].as_str().unwrap_or("unlabelled").to_string(),
        }
    }

    fn emit(&self, run: &str, ev: NewEvent) -> Result<(), String> {
        self.store.emit(run, ev).map(|_| ())
    }

    fn seeds(&self, rec: &Value) -> Vec<SignalSeed> {
        let class = match rec["data_class"].as_str() {
            Some("e0") => DataClass::E0,
            Some("original") => DataClass::Original,
            Some("synthetic") => DataClass::Synthetic,
            _ => DataClass::Treated,
        };
        let mut out = vec![];
        for (i, s) in rec.pointer("/sensor/output/signals").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let metric = s["metric_id"].as_str().filter(|m| opaque(m));
            let (num, den) = (s["numerator"].as_u64(), s["denominator"].as_u64().filter(|d| *d >= 1));
            let (Some(metric), Some(num), Some(den)) = (metric, num, den) else { continue };
            let evidence = s["evidence_ref"].as_str().filter(|e| opaque(e)).map_or_else(|| format!("ev-{:04}", i + 1), str::to_string);
            out.push(SignalSeed::new(metric, &evidence, num.min(den), den).with_metric(metric).with_data_class(class));
        }
        out
    }

    /// Runs (or resumes) the tick run `run_id`; returns what was done. Idempotent: a run the store already holds as `completed` is
    /// not run again, and a replay after a crash re-emits no verdict whose event already got out (the pipeline's delivery markers).
    pub fn process(&self, run_id: &str) -> Result<Value, String> {
        if !opaque(run_id) {
            return Err(format!("run id {run_id:?} is not an opaque id"));
        }
        let path = self.work.join("runs").join(format!("{run_id}.json"));
        let text = std::fs::read_to_string(&path).map_err(|e| format!("run record {}: {e}", path.display()))?;
        let rec: Value = serde_json::from_str(&text).map_err(|e| format!("run record {run_id}: {e}"))?;
        let state = self.store.state(run_id);
        if state.as_ref().is_some_and(|s| s["run"]["state"] == "completed") {
            return Ok(json!({"run_id": run_id, "already": "completed"}));
        }
        let seeds = self.seeds(&rec);
        let sensor = rec["sensor"]["semantics"].as_str().unwrap_or("unknown").to_string();
        let platform_real_sensor = sensor == "rust-events";
        let mode = rec["data_mode"].as_str().unwrap_or(if self.data_mode == DataMode::Dataset { "dataset" } else { "platform" }).to_string();
        let discards = rec["sensor"]["discards"].as_u64().unwrap_or(0);
        if state.is_none() {
            self.announce(run_id, &rec, &mode, &sensor, seeds.len(), discards)?;
        }
        let mut summary = json!({"run_id": run_id, "signals": seeds.len(), "sensor": sensor, "data_mode": mode});
        if !seeds.is_empty() {
            let mut o = PipelineOpts::new(self.work.join("pipeline").join(run_id), self.runner_exe.clone(), run_id, seeds.clone());
            o.model = Some(self.model()?);
            o.core = self.core()?;
            let sink: Arc<dyn RunEventSink> = self.store.clone();
            o.sink = Some(sink);
            let run = run_signals(&o)?;
            let mut declared: Vec<Value> = vec![];
            for (i, (entry, r)) in run.entries.iter().zip(&run.runs).enumerate() {
                let mut report = r.report.clone();
                self.patch_report(&mut report, &rec, &seeds[i], platform_real_sensor);
                let node = json!({"node_id": format!("proposal-{i}"), "label": format!("proposal-{i} [{}: {}] {}", entry.verdict.as_str(), entry.reason, entry.signal_id), "stage": "proposal", "status": "complete",
                    "depends_on": ["ports"], "reason_code": entry.reason, "node_kind": "material_step", "trace_id": null});
                self.emit(run_id, NewEvent::new("node_status_changed", "node", &format!("proposal-{i}"), json!({"node": node})))?;
                declared.extend(generate(&report));
                if let Some(p) = &r.payload {
                    for e in project(p, Some(&report), &now_iso()) {
                        self.emit(run_id, e)?;
                    }
                }
            }
            self.declare(run_id, &rec, declared)?;
            summary["proposals"] = json!(run.entries.len());
            summary["verdicts"] = json!(run.entries.iter().map(|e| json!({"proposal_id": e.proposal_id, "signal_id": e.signal_id, "verdict": e.verdict.as_str(), "reason": e.reason})).collect::<Vec<_>>());
            if !run.emit_errors.is_empty() {
                summary["emit_errors"] = json!(run.emit_errors);
            }
        } else {
            self.declare(run_id, &rec, vec![])?;
            summary["proposals"] = json!(0);
        }
        self.emit(run_id, NewEvent::new("run_state_changed", "run", run_id, json!({"state": "completed"})))?;
        Ok(summary)
    }

    /// The report of a proposal as this runner knows it: the trigger was the monitor tick, and with the real sensor the signal was
    /// admitted by real code over a real event package (the thread own sensors step stays the stand-in it is).
    fn patch_report(&self, report: &mut Value, rec: &Value, seed: &SignalSeed, real_sensor: bool) {
        let cell = rec["sensor"]["output"]["signals"].as_array().into_iter().flatten().find(|s| s["metric_id"].as_str() == Some(seed.signal_id.as_str())).and_then(|s| s["population"].as_str()).unwrap_or("unknown").to_string();
        report["source"] = json!({
            "sensor": rec["sensor"]["semantics"], "data_mode": rec["data_mode"], "data_origin": rec["data_origin"], "adapter": rec["adapter"], "source_id": rec["source_id"],
            "package": rec["package"], "events_read": rec["events_read"], "metric_id": seed.signal_id, "cell": cell, "numerator": seed.numerator, "denominator": seed.count,
        });
        let class = rec["data_class"].as_str().unwrap_or("treated").to_string();
        for st in report["steps"].as_array_mut().into_iter().flatten() {
            if st["data_class"] == "generated_sample" {
                st["data_class"] = json!(class); // the thread labels its own synthetic lab; this run's aggregates are of the record's class
            }
            match st["id"].as_str() {
                Some("trigger") => {
                    st["status"] = json!("real");
                    st["receipt"] = json!({"provider": "monitor-tick"});
                    st["detail"] = json!({"why": "triggered by the monitor tick of pulso run (a read of the configured source)"});
                }
                Some("signals") if real_sensor => {
                    st["status"] = json!("real");
                    st["data_class"] = json!(class);
                    st["receipt"] = json!({"provider": "rust-events"});
                    st["detail"] = json!({"semantics": "rust-events", "package": rec["package"]});
                }
                _ => {}
            }
        }
    }

    /// `run_started`, the run profile and its nodes (source, sensor, models, ports): what mode, adapter, source id, sensor, models and
    /// ports this run used, readable in the console graph and in the event log.
    fn announce(&self, run_id: &str, rec: &Value, mode: &str, sensor: &str, signals: usize, discards: u64) -> Result<(), String> {
        let label = self.label(rec);
        let label = label.as_str();
        let source_note = match (self.data_mode, self.provenance) {
            (DataMode::Platform, Provenance::Simulated) => "source declared SIMULATED by the operator (platform-shaped, not the real platform)",
            (DataMode::Platform, Provenance::Real) => "source declared real by the operator (pulso cannot verify it)",
            (DataMode::Platform, Provenance::Unspecified) => "source provenance not declared",
            (DataMode::Dataset, _) => "dataset replay",
        };
        let sensor_desc = if sensor == "rust-events" { "rust-events (real Rust sensor over the event package)".to_string() } else { format!("{sensor} (fixed-output stand-in)") };
        let title = format!(
            "{mode} run {run_id}: {label}; {signals} signal(s) admitted by sensor {sensor_desc}; models {}; Core {}; human simulated; release and observation simulated; no quality claim",
            self.models_desc(),
            self.core_desc()
        );
        self.emit(run_id, NewEvent::new("run_started", "run", run_id, json!({"title": title, "state": "running", "origin": "monitor"})))?;
        let profile = json!({
            "data_mode": mode, "data_origin": rec["data_origin"], "adapter": rec["adapter"], "source_id": rec["source_id"], "data_class": rec["data_class"], "label": label, "source_note": source_note,
            "sensor": sensor, "package": rec["package"], "watermark_from": rec["watermark_from"], "watermark_to": rec["watermark_to"], "events_read": rec["events_read"],
            "observed_until": rec["observed_until"], "history": rec["history"], "signals_admitted": signals, "discards": discards,
            "models": {"port": format!("{:?}", self.model_port).to_lowercase(), "label": self.models_desc()},
            "ports": {"core": if self.core_live { "live" } else { "offline-double" }, "release": "simulated", "observation": "simulated", "human": "simulated"},
        });
        self.emit(run_id, NewEvent::new("run_profile_set", "run", run_id, profile))?;
        let hist = &rec["history"];
        let nodes = [
            (
                "source",
                "trigger",
                format!(
                    "source [{mode} | {} | {} | {} events {} -> {} | observed until {} | history {}d/{} cases | {source_note}]",
                    rec["adapter"].as_str().unwrap_or("?"),
                    rec["source_id"].as_str().unwrap_or("?"),
                    rec["events_read"].as_u64().unwrap_or(0),
                    rec["watermark_from"].as_str().unwrap_or("?"),
                    rec["watermark_to"].as_str().unwrap_or("?"),
                    rec["observed_until"].as_str().unwrap_or("?"),
                    hist["days"].as_u64().unwrap_or(0),
                    hist["cases"].as_u64().unwrap_or(0)
                ),
                vec![],
            ),
            ("sensor", "scout", format!("sensor [{sensor_desc}: {signals} signal(s), {discards} discard(s)]"), vec!["source"]),
            ("models", "hypothesis", format!("models [{}]", self.models_desc()), vec!["sensor"]),
            (
                "ports",
                "evaluation",
                format!("ports [core {} | release simulated | observation simulated | human simulated]", if self.core_live { "live" } else { "offline-double" }),
                vec!["models"],
            ),
        ];
        for (id, stage, label, deps) in nodes {
            let node = json!({"node_id": id, "label": label, "stage": stage, "status": "complete", "depends_on": deps, "reason_code": null, "node_kind": "material_step", "trace_id": null});
            self.emit(run_id, NewEvent::new("node_status_changed", "node", id, json!({"node": node})))?;
        }
        Ok(())
    }

    /// `doubles_declared`: everything this run used that is not real. The proposals' own report doubles come in `from_reports`.
    fn declare(&self, run_id: &str, rec: &Value, from_reports: Vec<Value>) -> Result<(), String> {
        let baseline = build_with(&Input { sha: &"0".repeat(40), payload: None, events: &[], error: None }, &Extras { models: vec![], stop: None, core_real: self.core_live });
        let mut parts = if from_reports.is_empty() { generate(&baseline).into_iter().filter(|d| d["part"].as_str().is_some_and(|p| p.starts_with("port."))).collect() } else { from_reports };
        parts.push(json!({"part": "release", "status": "simulated"}));
        parts.push(json!({"part": "observation", "status": "simulated"}));
        match (self.data_mode, self.provenance) {
            (DataMode::Dataset, _) => parts.push(json!({"part": "data.dataset", "status": "demo-replay-not-production", "data_class": rec["data_class"]})),
            (DataMode::Platform, Provenance::Simulated) => parts.push(json!({"part": "data.source", "status": "simulated-operator-declared", "data_class": rec["data_class"]})),
            _ => {}
        }
        let mut items: Vec<Value> = vec![];
        for d in parts {
            if let Some(item) = double_item(&d) {
                if !items.iter().any(|x| x["id"] == item["id"]) {
                    items.push(item);
                }
            }
        }
        self.emit(run_id, NewEvent::new("doubles_declared", "run", run_id, json!({"doubles": items})))
    }
}

fn repo_err(e: RepoError) -> String {
    format!("{e:?}")
}

/// Per-finding records of the value loop in the engine job store (steps 1..; step 0 is the job summary).
struct JobPersist<'a, 'b> {
    job: &'a Claimed,
    ctx: &'a JobCtx<'b>,
}

impl Persist for JobPersist<'_, '_> {
    fn get(&self, step: u32) -> Option<String> {
        self.ctx.repo.output(self.ctx.tenant, &self.job.job, step).ok().flatten()
    }
    fn put(&self, step: u32, record: &str) -> Result<(), String> {
        match self.ctx.repo.commit_output(self.ctx.tenant, &self.job.job, step, self.ctx.worker, self.job.fence_token, self.ctx.now, record) {
            Ok(()) | Err(RepoError::Conflict(_)) => Ok(()),
            Err(e) => Err(repo_err(e)),
        }
    }
}

impl JobRunner for EngineRunner {
    fn run(&self, job: &Claimed, ctx: &JobCtx) -> Result<(), String> {
        let key = ctx.repo.job_key(ctx.tenant, &job.job).map_err(repo_err)?;
        let key = key.unwrap_or_default();
        let monitor = key.strip_prefix(JOB_KEY_PREFIX).map(str::to_string);
        let trigger = key.starts_with(TRIGGER_KEY_PREFIX);
        if monitor.is_none() && !trigger {
            return Ok(()); // neither a monitor nor a trigger job: nothing this runner can do, and retrying would not change that
        }
        if ctx.repo.output(ctx.tenant, &job.job, 0).map_err(repo_err)?.is_some() {
            return Ok(()); // an earlier attempt finished the work and only the completion was lost
        }
        // OUT1: a `release.*` outcome trigger is the outcome step, not another pass of the value loop.
        if let Some(t) = find_trigger(&self.store, &key) {
            let mut summary = json!({"trigger_job": job.job, "kind": "outcome", "event_type": t.event_type});
            summary["outcome"] = match &self.outcome {
                Some(o) => {
                    let out = o.run(&t)?;
                    self.record_outcome(&job.job, &out)?;
                    out
                }
                None => json!({"skipped": "outcome step not configured (PULSO_OUTCOME_PRE_CELLS or PULSO_CELLS_NDJSON is not set)"}),
            };
            return match ctx.repo.commit_output(ctx.tenant, &job.job, 0, ctx.worker, job.fence_token, ctx.now, &summary.to_string()) {
                Ok(()) | Err(RepoError::Conflict(_)) => Ok(()),
                Err(e) => Err(repo_err(e)),
            };
        }
        let mut summary = match &monitor {
            Some(run_id) => {
                if self.core_live {
                    // A live Core can publish: from here a crash is an unknown effect, never an automatic retry.
                    ctx.repo.begin_effect(ctx.tenant, &job.job, ctx.worker, job.fence_token, ctx.now).map_err(repo_err)?;
                }
                self.process(run_id)?
            }
            None => json!({"trigger_job": job.job, "kind": "trigger"}),
        };
        match &self.value_loop {
            Some(v) => {
                let out = v.run(&JobPersist { job, ctx })?;
                self.record_loop(&job.job, &out)?;
                // aggregates, reason codes and ids only: the readable outcome of the job next to the other run records
                let dir = self.work.join("value-loop");
                let _ = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(format!("{}.json", job.job)), serde_json::to_vec_pretty(&out).unwrap_or_default()));
                summary["value_loop"] = out;
            }
            None if trigger => summary["value_loop"] = json!({"skipped": "value loop not configured (PULSO_CELLS_NDJSON is not set)"}),
            None => {}
        }
        match ctx.repo.commit_output(ctx.tenant, &job.job, 0, ctx.worker, job.fence_token, ctx.now, &summary.to_string()) {
            Ok(()) | Err(RepoError::Conflict(_)) => Ok(()),
            Err(e) => Err(repo_err(e)),
        }
    }
}

impl EngineRunner {
    /// The outcome cards in the console store: one run per release, one node per card (verdict words and reason codes only).
    fn record_outcome(&self, job: &str, out: &Value) -> Result<(), String> {
        let id: String = format!("outcome-{job}").chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(64).collect();
        if self.store.state(&id).is_some() {
            return Ok(());
        }
        let title = format!(
            "outcome {id}: release {} ({}), state {}; descriptive association against sibling cells, not cause; success claimed: {}",
            out["release_id"].as_str().unwrap_or("?"), out["event_type"].as_str().unwrap_or("?"), out["state"].as_str().unwrap_or("?"), out["success_claimed"]
        );
        self.emit(&id, NewEvent::new("run_started", "run", &id, json!({"title": title, "state": "running", "origin": "outcome"})))?;
        for (i, c) in out["cards"].as_array().into_iter().flatten().enumerate() {
            let label = format!("card-{i} [{}{}] {} ({})", c["verdict"].as_str().unwrap_or("?"), c["reason"].as_str().map_or(String::new(), |r| format!(": {r}")), c["finding"]["metric"].as_str().unwrap_or("?"), c["period_kind"].as_str().unwrap_or("?"));
            let node = json!({"node_id": format!("card-{i}"), "label": label, "stage": "evaluation", "status": "complete", "depends_on": [], "reason_code": c["reason"], "node_kind": "material_step", "trace_id": null});
            self.emit(&id, NewEvent::new("node_status_changed", "node", &format!("card-{i}"), json!({"node": node})))?;
        }
        self.emit(&id, NewEvent::new("run_state_changed", "run", &id, json!({"state": "completed"})))
    }

    /// The value-loop outcome in the console store: one run, one node per finding (reason codes only).
    fn record_loop(&self, job: &str, out: &Value) -> Result<(), String> {
        let id: String = format!("value-loop-{job}").chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(64).collect();
        if self.store.state(&id).is_some() {
            return Ok(());
        }
        let s = &out["summary"];
        let title = format!(
            "value loop {id}: {} corroborated finding(s) from the claude-standin cells sensor ({}), {} proposed, {} delivered to agent-core as builder (never approved or published), {} denied, {} blocked, {} unlinked; baseline {}; no quality claim",
            s["corroborated"], out["data_source"].as_str().unwrap_or("?"), s["proposed"], s["delivered"], s["denied"], s["blocked"], s["unlinked"], out["baseline"]["label"].as_str().unwrap_or("?")
        );
        self.emit(&id, NewEvent::new("run_started", "run", &id, json!({"title": title, "state": "running", "origin": "value-loop"})))?;
        for (i, f) in out["findings"].as_array().into_iter().flatten().enumerate() {
            let d = &f["delivery"];
            let tail = match d["status"].as_str() {
                Some("delivered") => format!(" delivered {}", d["proposal_id"].as_str().unwrap_or("?")),
                Some(_) => format!(" denied {}", d["reason"].as_str().unwrap_or("?")),
                None => String::new(),
            };
            let label = format!("finding-{i} [{}: {}]{tail}", f["status"].as_str().unwrap_or("?"), f["reason"].as_str().unwrap_or("?"));
            let node = json!({"node_id": format!("finding-{i}"), "label": label, "stage": "proposal", "status": "complete", "depends_on": [], "reason_code": f["reason"], "node_kind": "material_step", "trace_id": null});
            self.emit(&id, NewEvent::new("node_status_changed", "node", &format!("finding-{i}"), json!({"node": node})))?;
        }
        self.emit(&id, NewEvent::new("run_state_changed", "run", &id, json!({"state": "completed"})))
    }
}
