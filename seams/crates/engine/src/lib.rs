//! Run-once harness (E1): linear JobHandler list over a job store port.
//! Each handler's input is persisted with compare-and-set before it runs, and each output
//! (payload + events) is committed as one CAS record. The event log is derived from the
//! committed records, so a kill between any two handlers resumes with an identical log.
//! `executor` (E2) supersedes this for real jobs: lease/fence/effect enforcement live there.
//! Known limits of run_once: EffectState is typed but not enforced here (a non-NoEffect output is committed like any other;
//! no skip-on-resume yet); the stale-fence check is read-then-commit, not atomic with the out/N CAS;
//! lease expiry/reclaim (C-7 now>=expires) is not modelled; FileStore locking is a lock file, not OS-level.
use abi::*;

pub mod adapters;
pub mod conformance;
pub mod demo;
pub mod executor;
pub mod ledger;
pub mod live;
pub mod live_core;
pub mod real_core;
pub mod models;
mod lib_codec;
pub mod store;
pub mod synth;
pub use store::{CommitGuard, FileStore, JobStore};

#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    Crashed(usize),
    Handler(HandlerError),
    Store(String),
}

pub struct Options {
    pub job_id: String,
    pub worker_id: String,
    /// Test hook: stop (simulated kill) right after handler N committed.
    pub crash_after: Option<usize>,
    /// Called after handler N committed (the binary uses it for the kill marker).
    pub after_commit: Option<Box<dyn Fn(usize)>>,
}

fn se(e: String) -> RunError {
    RunError::Store(e)
}

fn encode(out: &OutputEnvelope) -> String {
    let mut s = format!("P {}", out.payload);
    for e in &out.events {
        s.push_str(&format!("\nE {e}"));
    }
    s
}

fn decode(rec: &str) -> (String, Vec<String>) {
    let mut payload = String::new();
    let mut events = vec![];
    for l in rec.lines() {
        if let Some(p) = l.strip_prefix("P ") {
            payload = p.to_string();
        } else if let Some(e) = l.strip_prefix("E ") {
            events.push(e.to_string());
        }
    }
    (payload, events)
}

/// Event log = events of committed handler outputs, in handler order.
pub fn event_log(store: &dyn JobStore, handlers: usize) -> Result<Vec<String>, String> {
    let mut log = vec![];
    for i in 0..handlers {
        match store.get(&format!("out/{i}"))? {
            Some((_, rec)) => log.extend(decode(&rec).1),
            None => break,
        }
    }
    Ok(log)
}

/// Runs (or resumes) the handlers linearly; returns the final payload.
pub fn run_once(
    store: &dyn JobStore,
    handlers: &[Box<dyn JobHandler>],
    initial: &str,
    opts: &Options,
) -> Result<String, RunError> {
    // claim: bump fence/attempt with CAS
    let (ver, attempt) = match store.get("fence").map_err(se)? {
        Some((v, s)) => (v, s.parse::<u64>().map_err(|_| se("corrupt fence".into()))?),
        None => (0, 0),
    };
    let attempt = attempt + 1;
    let token = store.cas("fence", ver, &attempt.to_string()).map_err(se)?;
    let fence = Fence { worker_id: opts.worker_id.clone(), fence_token: token, attempt };

    if initial.contains(char::is_control) {
        return Err(RunError::Handler(HandlerError::Invalid("initial payload contains a newline".into())));
    }
    let mut payload = initial.to_string();
    for (i, h) in handlers.iter().enumerate() {
        if let Some((_, rec)) = store.get(&format!("out/{i}")).map_err(se)? {
            payload = decode(&rec).0; // already committed by an earlier attempt
            continue;
        }
        // persist the input with CAS; an earlier attempt's persisted input wins
        let ik = format!("in/{i}");
        let input_payload = match store.get(&ik).map_err(se)? {
            Some((_, v)) => v, // an earlier attempt's persisted input wins
            None => {
                store.cas(&ik, 0, &payload).map_err(se)?;
                payload.clone()
            }
        };
        let input = InputEnvelope { job_id: opts.job_id.clone(), step_index: i, payload: input_payload };
        let out = h.run(&fence, &input).map_err(RunError::Handler)?;
        for t in std::iter::once(&out.payload).chain(out.events.iter()) {
            if t.contains(char::is_control) {
                return Err(RunError::Handler(HandlerError::Invalid("payload or event contains a newline".into())));
            }
        }
        // stale-fence check: a newer claim bumped the fence while this handler ran
        match store.get("fence").map_err(se)? {
            Some((v, _)) if v == token => {}
            _ => return Err(RunError::Handler(HandlerError::StaleFence)),
        }
        store.cas(&format!("out/{i}"), 0, &encode(&out)).map_err(se)?;
        payload = out.payload;
        if let Some(f) = &opts.after_commit {
            f(i);
        }
        if opts.crash_after == Some(i) {
            return Err(RunError::Crashed(i));
        }
    }
    Ok(payload)
}
