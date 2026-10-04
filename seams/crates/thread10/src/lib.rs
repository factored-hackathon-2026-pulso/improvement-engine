//! Q1 thread10: the ten-step E2E-THREAD-01 report on the Rust shell (host=rust), OFFLINE.
//! The engine executor runs the nine handlers (sensors, recompute, validation, compile, arms, gate, native_eval,
//! authority, publish) over a labelled Core double; `report` derives the ten report steps from what the job COMMITTED.
//!
//! R1E: the scout claim, the verifier check and the builder change spec are calls through an `engine::models::ModelPort`
//! (`Opts::model`, default `Scripted` = the old fixed values), answered before the job runs and persisted in the job store
//! (`model/<role>`) so a resume never asks a model twice. The Core is a `CorePort` too (`Opts::core`, default the offline double).
//! `pipeline` runs this thread once per signal and records a viability verdict for every proposal.
pub mod double;
pub mod note;
pub mod pipeline;
pub mod platform;
pub mod report;
pub mod requests;

use abi::JobHandler;
use authority::Override;
use engine::adapters::thread_handlers;
use engine::executor::{ExecOptions, execute, read_lease};
use engine::live::{CorePort, LiveConfig, dry_run_hook, live_handlers};
use engine::ledger::bk0_check;
use engine::models::tps::DEFAULT_K;
use engine::models::{ModelError, ModelPort, Recording, Scripted};
use engine::synth::LabRow;
use engine::{FileStore, JobStore, event_log, synth};
use serde_json::Value;
use std::path::PathBuf;
use std::rc::Rc;

pub const JOB: &str = "thread-1";

/// One signal under test: its id (the pipeline's, free of the stand-in sensor's `sig-0001`), the evidence ref and the aggregate
/// numerator/count of its lab row. Aggregates only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalSeed {
    pub signal_id: String,
    pub evidence_ref: String,
    pub numerator: u64,
    pub count: u64,
}

impl SignalSeed {
    /// The lab signal of the default thread: 120/400 = 0.30.
    pub fn lab_default() -> SignalSeed {
        SignalSeed { signal_id: "sig-0001".into(), evidence_ref: "ev-0001".into(), numerator: 120, count: 400 }
    }

    /// Rate at two decimals, round half even, as the recompute step computes it.
    pub fn rate(&self) -> f64 {
        let (n, c) = (u128::from(self.numerator) * 100, u128::from(self.count.max(1)));
        let (q, r) = (n / c, n % c);
        let up = r * 2 > c || (r * 2 == c && q % 2 == 1);
        (q + u128::from(up)) as f64 / 100.0
    }

    /// The opaque id the model sees for the evidence (`ev_` + 16 hex of its digest), never the evidence ref itself.
    pub fn model_evidence_ref(&self) -> String {
        format!("ev_{}", &core_client::canon::sha256_hex(self.evidence_ref.as_bytes())[..16])
    }

    pub fn lab_row(&self) -> LabRow {
        LabRow { evidence_ref: self.evidence_ref.clone(), numerator: self.numerator, count: self.count }
    }
}

pub struct Opts {
    pub work: PathBuf,
    pub runner: PathBuf,
    pub human_override: bool,
    pub denied_kind: bool,
    /// What the scripted scout claims (default 0.3, the lab value). Ignored when `model` is set.
    pub claimed_rate: Option<f64>,
    pub sha: String,
    /// Unix seconds for the executor lease clock (a resume after a kill passes a later one).
    pub now: u64,
    /// After handler N commits, create the file and block (the process is then killed by the test).
    pub kill_marker: Option<(PathBuf, usize)>,
    /// Live hook: called after handler `i` commits with the PARTIAL report built from what is committed so far (steps whose
    /// handler has not run say `not_exercised`). Used by `pulso demo` to stream the run; it never alters the run.
    pub on_commit: Option<Rc<dyn Fn(usize, &Value)>>,
    /// Append one line per Core-double publish invocation (observable side effect).
    pub ledger: Option<PathBuf>,
    /// Inside the publish effect (after the ledger line, before the commit): create the file and block.
    pub kill_in_publish: Option<PathBuf>,
    /// The port behind scout, verifier and builder. `None` = `Scripted` (from `claimed_rate` / `denied_kind`).
    pub model: Option<Rc<dyn ModelPort>>,
    /// The signal under test (default: the lab signal).
    pub seed: SignalSeed,
    /// The executor job id (default `JOB`).
    pub job: String,
    /// The Core behind arms, native evaluation, approve and publish. `None` = the offline `DoublePort`.
    pub core: Option<Rc<dyn CorePort>>,
}

impl Opts {
    pub fn new(work: PathBuf, runner: PathBuf) -> Opts {
        Opts {
            work,
            runner,
            human_override: false,
            denied_kind: false,
            claimed_rate: None,
            sha: "0".repeat(40),
            now: 1000,
            kill_marker: None,
            on_commit: None,
            ledger: None,
            kill_in_publish: None,
            model: None,
            seed: SignalSeed::lab_default(),
            job: JOB.into(),
            core: None,
        }
    }
}

pub struct Run {
    pub events: Vec<String>,
    pub error: Option<String>,
    pub report: Value,
    /// The last committed job payload (`{"spec":..,"out":..}`); `None` when the job never ran.
    pub payload: Option<Value>,
    /// `(role, code, why)` when a model stage stopped the job before it ran (see `report::Extras::stop`).
    pub stop: Option<(String, String, String)>,
    /// The builder's proposal (`kind`, `op`, `target_ref`, `new_ref`, ...) when it answered one.
    pub proposal: Option<Value>,
}

fn committed_payload(store: &FileStore, n: usize) -> Result<Option<Value>, String> {
    let mut last = None;
    for i in 0..n {
        match store.get(&format!("out/{i}"))? {
            Some((_, rec)) => last = rec.lines().find_map(|l| l.strip_prefix("P ")).map(str::to_string),
            None => break,
        }
    }
    last.map(|p| serde_json::from_str(&p).map_err(|e| e.to_string())).transpose()
}

fn published_release(out: &Value) -> Option<platform::Release> {
    let p = out.get("publish")?;
    Some(platform::Release {
        release_id: p.get("release_id")?.as_str()?.to_string(),
        agent_id: "atencion-tarea".into(),
        alias: p.get("alias")?.as_str()?.to_string(),
        candidate_hash: out.get("arms")?.get("frozen")?.get("candidate_hash")?.as_str()?.to_string(),
    })
}

/// Record the published release, deliver `release.published` (then a replay of it) and prove the unmatched case is retryable.
fn correlate(r: &platform::Release) -> Result<Value, String> {
    let p = platform::Platform::new();
    let unknown = platform::Release { release_id: format!("{}-unrecorded", r.release_id), ..r.clone() };
    let unmatched = p.post_published("evt-unmatched", &unknown).0;
    p.record_release(r)?;
    let (first, body) = p.post_published("evt-1", r);
    let (replay, _) = p.post_published("evt-1", r);
    let keys = p.successor_keys();
    if first != 202 {
        return Err(format!("release correlation answered {first}: {body}"));
    }
    Ok(serde_json::json!({"unique_key": keys.first(), "runs": keys.len(), "first_status": first, "replay_status": replay,
        "unmatched_status": unmatched, "observation_window": body["observation_window"], "platform": "in-process control-api over MemStore (double)"}))
}

// ---------------------------------------------------------------------------------------------------------------
// The model stage: scout, verifier, builder, answered once and persisted
// ---------------------------------------------------------------------------------------------------------------

/// Ask `role` through `rec`, or replay the answer persisted under `model/<role>` by an earlier attempt. An outage is not
/// persisted (a resume may retry it); a refusal, an answer and an unusable answer are.
fn ask(store: &FileStore, rec: &Recording, req: &engine::models::ModelRequest) -> Result<Result<(Value, Value), (String, String, Value)>, String> {
    let key = format!("model/{}", req.role.as_str());
    if let Some((_, text)) = store.get(&key)? {
        let doc: Value = serde_json::from_str(&text).map_err(|e| format!("corrupt {key}: {e}"))?;
        return Ok(match doc.get("error") {
            None => Ok((doc["content"].clone(), doc["record"].clone())),
            Some(e) => Err((e["code"].as_str().unwrap_or("model_invalid").into(), e["why"].as_str().unwrap_or("").into(), doc["record"].clone())),
        });
    }
    let result = rec.call(req);
    let record = rec.calls().last().map(engine::models::CallRecord::to_json).ok_or("the model port recorded no call")?;
    let (doc, out) = match result {
        Ok(a) => (serde_json::json!({"content": a.content, "record": record}), Ok((a.content, record))),
        Err(e) => {
            let (code, why) = match e {
                ModelError::Refused(w) => ("model_refused", w),
                ModelError::Unavailable(w) => ("model_unavailable", w),
                ModelError::Invalid(w) => ("model_invalid", w),
            };
            (serde_json::json!({"error": {"code": code, "why": why}, "record": record}), Err((code.to_string(), why, record)))
        }
    };
    if !matches!(&out, Err((c, _, _)) if c == "model_unavailable") {
        let _ = store.cas(&key, 0, &doc.to_string());
    }
    Ok(out)
}

struct Consulted {
    models: Vec<Value>,
    stop: Option<(String, String, String)>,
    claimed_rate: f64,
    proposal: Value,
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty() && !s.contains(char::is_control))
}

/// The three model calls. A stop is `(role, code, why)`: no further call is made and the job does not run.
fn consult(store: &FileStore, rec: &Recording, seed: &SignalSeed) -> Result<Consulted, String> {
    let mut c = Consulted { models: vec![], stop: None, claimed_rate: 0.0, proposal: Value::Null };
    if seed.count < DEFAULT_K as u64 {
        c.stop = Some(("scout".into(), "evidence_insufficient".into(), format!("the aggregate row has {} cases, below k={DEFAULT_K}: it is suppressed, nothing to claim from", seed.count)));
        return Ok(c);
    }
    macro_rules! step {
        ($req:expr, $role:literal) => {
            match ask(store, rec, &$req)? {
                Ok((content, record)) => {
                    c.models.push(record);
                    content
                }
                Err((code, why, record)) => {
                    c.models.push(record);
                    c.stop = Some(($role.into(), code, why));
                    return Ok(c);
                }
            }
        };
    }
    let invalid = |c: &mut Consulted, role: &str, why: &str| {
        c.stop = Some((role.into(), "model_invalid".into(), why.into()));
    };
    let scout = step!(requests::scout_request(seed), "scout");
    let h = scout.get("hypotheses").and_then(Value::as_array).and_then(|a| a.first());
    let rate = h.filter(|h| str_of(h, "signal_id") == Some(seed.signal_id.as_str())).and_then(|h| h.get("claimed_rate")).and_then(Value::as_f64).filter(|r| (0.0..=1.0).contains(r));
    let Some(rate) = rate else {
        invalid(&mut c, "scout", "the scout answer has no hypothesis for this signal with a claimed_rate in [0,1]");
        return Ok(c);
    };
    c.claimed_rate = rate;
    let verdict = step!(requests::verifier_request(seed, rate), "verifier");
    match str_of(&verdict, "verdict") {
        Some("agree") => {}
        Some("disagree") => {
            c.stop = Some(("verifier".into(), "verifier_refuted".into(), "the independent verifier disagrees with the scout claim".into()));
            return Ok(c);
        }
        _ => {
            invalid(&mut c, "verifier", "the verifier answer has no verdict agree|disagree");
            return Ok(c);
        }
    }
    let built = step!(requests::builder_request(seed), "builder");
    let p = built.get("proposal").filter(|p| ["kind", "op", "target_ref", "new_ref"].iter().all(|k| str_of(p, k).is_some()));
    match p {
        Some(p) => {
            c.proposal = p.clone();
            // The compile step can express only prompt and eval_suite changes (it names its own denials for those); for any other
            // kind BK0 is the authority and the job does not run: not_evaluable, never a forged proposal.
            let (kind, op) = (str_of(p, "kind").unwrap_or(""), str_of(p, "op").unwrap_or(""));
            if !["prompt", "eval_suite"].contains(&kind)
                && let Err(reason) = bk0_check(kind, op)
            {
                c.stop = Some(("builder".into(), reason, format!("BK0: ({kind},{op}) is not a supported change family")));
            }
        }
        None => invalid(&mut c, "builder", "the builder answer has no proposal with kind, op, target_ref and new_ref"),
    }
    Ok(c)
}

/// The fixture compile spec carries one `replace` of the seeded prompt; the builder's proposal rewrites that first operation.
/// A target other than the seeded prompt has no known precondition digest, so compile denies it by its own rules.
fn apply_proposal(spec: &str, p: &Value) -> Result<String, String> {
    let mut v: Value = serde_json::from_str(spec).map_err(|e| e.to_string())?;
    let op = v.pointer_mut("/spec/compile/change_spec/operations/0").and_then(Value::as_object_mut).ok_or("spec has no compile operation to rewrite")?;
    let known = str_of(p, "kind") == Some("prompt") && str_of(p, "target_ref") == Some("prompt:resumen_radicado@1");
    for (to, from) in [("op", "op"), ("target_kind", "kind"), ("target_ref", "target_ref"), ("new_ref", "new_ref")] {
        op.insert(to.into(), p[from].clone());
    }
    if !known {
        op.insert("precondition_digest".into(), Value::String(format!("sha256:{}", "0".repeat(64))));
    }
    if let (Some(m), Some(cs)) = (str_of(p, "mechanism"), v.pointer_mut("/spec/compile/change_spec").and_then(Value::as_object_mut)) {
        cs.insert("expected_mechanism".into(), Value::String(m.into()));
    }
    serde_json::to_string(&v).map_err(|e| e.to_string())
}

pub fn run(o: &Opts) -> Result<Run, String> {
    std::fs::create_dir_all(&o.work).map_err(|e| e.to_string())?;
    let store = Rc::new(FileStore::open(o.work.join("store"))?);
    let port_for_models: Rc<dyn ModelPort> = o.model.clone().unwrap_or_else(|| {
        Rc::new(Scripted { claimed_rate: o.claimed_rate.unwrap_or(0.3), op: if o.denied_kind { "add".into() } else { "replace".into() } })
    });
    let rec = Recording::new(port_for_models);
    let consulted = consult(&store, &rec, &o.seed)?;
    let core: Rc<dyn CorePort> = o.core.clone().unwrap_or_else(|| Rc::new(double::DoublePort { ledger: o.ledger.clone(), kill_in_publish: o.kill_in_publish.clone() }));
    let core_real = core.is_real();
    let extras = report::Extras { models: consulted.models.clone(), stop: consulted.stop.as_ref().map(|(r, c, _)| (r.clone(), c.clone())), core_real };
    if let Some((role, code, why)) = &consulted.stop {
        let error = format!("blocked({code}): {role}: {why}");
        let report = report::build_with(&report::Input { sha: &o.sha, payload: None, events: &[], error: Some(&error) }, &extras);
        let mut report = report;
        report["successor"] = Value::Null;
        report["run"] = serde_json::json!({"job": o.job, "attempt": 0, "store": "engine FileStore"});
        report["memory_note"] = Value::Null;
        return Ok(Run { events: vec![], error: Some(error), report, payload: None, stop: consulted.stop.clone(), proposal: Some(consulted.proposal.clone()).filter(|p| !p.is_null()) });
    }
    let (env, mut spec) = synth::build_row(&o.work, &o.runner, &o.seed.lab_row(), Some(consulted.claimed_rate))?;
    spec = apply_proposal(&spec, &consulted.proposal)?;
    let over = o.human_override.then(|| Override { by: "human".into(), actor: double::ACTOR.into(), reason: "exercise approve/publish of a failed structural gate; no quality claim".into() });
    let cfg = LiveConfig { human_actor: double::ACTOR.into(), human_override: over, decision_ttl_seconds: 600 };
    let hs: Vec<Box<dyn JobHandler>> = live_handlers(thread_handlers(env, Some(dry_run_hook(core.clone()))), core, cfg);
    let mut eo = ExecOptions::new(&o.job, "w1", o.now);
    let (live, live_store, live_sha, n_handlers, live_extras) = (o.on_commit.clone(), store.clone(), o.sha.clone(), hs.len(), extras.clone());
    let kill = o.kill_marker.clone();
    if live.is_some() || kill.is_some() {
        eo.after_commit = Some(Box::new(move |i| {
            if let Some(f) = &live {
                let events = event_log(&*live_store, n_handlers).unwrap_or_default();
                let payload = committed_payload(&live_store, i + 1).ok().flatten();
                f(i, &report::build_with(&report::Input { sha: &live_sha, payload: payload.as_ref(), events: &events, error: None }, &live_extras));
            }
            let Some((marker, n)) = &kill else { return };
            if i == *n {
                std::fs::write(marker, "x").unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }));
    }
    let res = execute(&*store, &hs, &spec, &eo);
    let events = event_log(&*store, hs.len())?;
    let payload = committed_payload(&store, hs.len())?;
    let error = res.err().map(|e| format!("{e:?}"));
    let mut report = report::build_with(&report::Input { sha: &o.sha, payload: payload.as_ref(), events: &events, error: error.as_deref() }, &extras);
    let release = payload.as_ref().and_then(|p| p.get("out")).and_then(published_release);
    report["successor"] = match &release {
        Some(r) => correlate(r)?,
        None => Value::Null,
    };
    let attempt = read_lease(&*store).ok().flatten().map_or(0, |l| l.attempt);
    report["run"] = serde_json::json!({"job": o.job, "attempt": attempt, "store": "engine FileStore"});
    let gate = report["gate"]["verdict"].as_str().map(str::to_string);
    report["memory_note"] = note::post_run_note(&o.job, &events, gate.as_deref(), release.is_some())?;
    Ok(Run { events, error, report, payload, stop: None, proposal: Some(consulted.proposal).filter(|p| !p.is_null()) })
}
