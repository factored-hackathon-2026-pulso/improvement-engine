//! Q1 thread10: the ten-step E2E-THREAD-01 report on the Rust shell (host=rust), OFFLINE.
//! The engine executor runs the nine handlers (sensors, recompute, validation, compile, arms, gate, native_eval,
//! authority, publish) over a labelled Core double; `report` derives the ten report steps from what the job COMMITTED.
pub mod double;
pub mod platform;
pub mod report;

use abi::JobHandler;
use authority::Override;
use engine::adapters::thread_handlers;
use engine::executor::{ExecOptions, execute};
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
    let report = report::build(&report::Input { sha: &o.sha, payload: payload.as_ref(), events: &events, error: error.as_deref() });
    Ok(Run { events, error, report })
}
