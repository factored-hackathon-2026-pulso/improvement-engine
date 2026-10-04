//! Q1 thread10: the ten-step E2E-THREAD-01 report on the Rust shell (host=rust), OFFLINE.
//! The engine executor runs the nine handlers (sensors, recompute, validation, compile, arms, gate, native_eval,
//! authority, publish) over a labelled Core double; `report` derives the ten report steps from what the job COMMITTED.
pub mod double;
pub mod note;
pub mod platform;
pub mod report;

use abi::JobHandler;
use authority::Override;
use engine::adapters::thread_handlers;
use engine::executor::{ExecOptions, execute, read_lease};
use engine::live::{CorePort, LiveConfig, dry_run_hook, live_handlers};
use engine::{FileStore, JobStore, event_log, synth};
use serde_json::Value;
use std::path::PathBuf;
use std::rc::Rc;

pub const JOB: &str = "thread-1";

pub struct Opts {
    pub work: PathBuf,
    pub runner: PathBuf,
    pub human_override: bool,
    pub denied_kind: bool,
    /// What the scripted scout claims (default 0.3, the lab value).
    pub claimed_rate: Option<f64>,
    pub sha: String,
    /// Unix seconds for the executor lease clock (a resume after a kill passes a later one).
    pub now: u64,
    /// After handler N commits, create the file and block (the process is then killed by the test).
    pub kill_marker: Option<(PathBuf, usize)>,
}

impl Opts {
    pub fn new(work: PathBuf, runner: PathBuf) -> Opts {
        Opts { work, runner, human_override: false, denied_kind: false, claimed_rate: None, sha: "0".repeat(40), now: 1000, kill_marker: None }
    }
}

pub struct Run {
    pub events: Vec<String>,
    pub error: Option<String>,
    pub report: Value,
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

pub fn run(o: &Opts) -> Result<Run, String> {
    std::fs::create_dir_all(&o.work).map_err(|e| e.to_string())?;
    let (env, mut spec) = synth::build(&o.work, &o.runner, o.claimed_rate)?;
    if o.denied_kind {
        // an `add` of a prompt is not a supported (op, kind) pair of the seeded base world: kind_not_supported
        spec = spec.replacen(r#""op":"replace""#, r#""op":"add""#, 1);
    }
    let store = FileStore::open(o.work.join("store"))?;
    let port: Rc<dyn CorePort> = Rc::new(double::DoublePort);
    let over = o.human_override.then(|| Override { by: "human".into(), actor: double::ACTOR.into(), reason: "exercise approve/publish of a failed structural gate; no quality claim".into() });
    let cfg = LiveConfig { human_actor: double::ACTOR.into(), human_override: over, decision_ttl_seconds: 600 };
    let hs: Vec<Box<dyn JobHandler>> = live_handlers(thread_handlers(env, Some(dry_run_hook(port.clone()))), port, cfg);
    let mut eo = ExecOptions::new(JOB, "w1", o.now);
    if let Some((marker, n)) = o.kill_marker.clone() {
        eo.after_commit = Some(Box::new(move |i| {
            if i == n {
                std::fs::write(&marker, "x").unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }));
    }
    let res = execute(&store, &hs, &spec, &eo);
    let events = event_log(&store, hs.len())?;
    let payload = committed_payload(&store, hs.len())?;
    let error = res.err().map(|e| format!("{e:?}"));
    let mut report = report::build(&report::Input { sha: &o.sha, payload: payload.as_ref(), events: &events, error: error.as_deref() });
    let release = payload.as_ref().and_then(|p| p.get("out")).and_then(published_release);
    report["successor"] = match &release {
        Some(r) => correlate(r)?,
        None => Value::Null,
    };
    let attempt = read_lease(&store).ok().flatten().map_or(0, |l| l.attempt);
    report["run"] = serde_json::json!({"job": JOB, "attempt": attempt, "store": "engine FileStore"});
    let gate = report["gate"]["verdict"].as_str().map(str::to_string);
    report["memory_note"] = note::post_run_note(JOB, &events, gate.as_deref(), release.is_some())?;
    Ok(Run { events, error, report })
}
