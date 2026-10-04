//! Run-once harness (E1). NAIVE first version (RED): outputs live only in memory.
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
    /// Test hook: stop (simulated kill) right after handler N finished.
    pub crash_after: Option<usize>,
    /// Called after handler N committed (binary uses it for the marker file).
    pub after_commit: Option<Box<dyn Fn(usize)>>,
}

/// Runs handlers linearly; returns the final payload.
pub fn run_once(
    store: &dyn JobStore,
    handlers: &[Box<dyn JobHandler>],
    initial: &str,
    opts: &Options,
) -> Result<String, RunError> {
    let fence = Fence { worker_id: opts.worker_id.clone(), fence_token: 1, attempt: 1 };
    let mut payload = initial.to_string();
    for (i, h) in handlers.iter().enumerate() {
        let input = InputEnvelope { job_id: opts.job_id.clone(), step_index: i, payload: payload.clone() };
        let out = h.run(&fence, &input).map_err(RunError::Handler)?;
        for e in &out.events {
            store.append_event(e).map_err(RunError::Store)?;
        }
        payload = out.payload;
        if opts.crash_after == Some(i) {
            return Err(RunError::Crashed(i));
        }
    }
    Ok(payload)
}
