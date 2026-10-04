//! Run-once harness (E1): linear JobHandler list over a job store port.
//! Each handler's input is persisted with compare-and-set before it runs, and each output
//! (payload + events) is committed as one CAS record. The event log is derived from the
//! committed records, so a kill between any two handlers resumes with an identical log.
use abi::*;

pub mod demo;
pub mod store;
pub use store::{FileStore, JobStore};

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
        Some((v, s)) => (v, s.parse::<u64>().unwrap_or(0)),
        None => (0, 0),
    };
    let attempt = attempt + 1;
    let token = store.cas("fence", ver, &attempt.to_string()).map_err(se)?;
    let fence = Fence { worker_id: opts.worker_id.clone(), fence_token: token, attempt };

    let mut payload = initial.to_string();
    for (i, h) in handlers.iter().enumerate() {
        if let Some((_, rec)) = store.get(&format!("out/{i}")).map_err(se)? {
            payload = decode(&rec).0; // already committed by an earlier attempt
            continue;
        }
        // persist the input with CAS; an earlier attempt's persisted input wins
        let input_payload = match store.cas(&format!("in/{i}"), 0, &payload) {
            Ok(_) => payload.clone(),
            Err(_) => store.get(&format!("in/{i}")).map_err(se)?.map(|(_, v)| v).unwrap_or(payload.clone()),
        };
        let input = InputEnvelope { job_id: opts.job_id.clone(), step_index: i, payload: input_payload };
        let out = h.run(&fence, &input).map_err(RunError::Handler)?;
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
