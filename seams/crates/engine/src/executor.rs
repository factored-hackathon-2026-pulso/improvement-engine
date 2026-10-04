//! E2 executor: linear driver over `JobHandler`s with lease/fence/effect semantics of FRZ0 C-7.
//! Mirrors (never depends on) crates/core durable_jobs.rs:
//! - a lease is reclaimable when `now >= expires_at`; every claim bumps fence token and attempt by one;
//! - only a holder of the current fence with an unexpired lease can commit (else `StaleFence`);
//! - a handler whose effect was dispatched but never committed leaves a `eff/N` marker: the job is then
//!   `NeedsReconciliation` and the handler is never run again (C-7: only NoEffect jobs are claimable);
//! - committed outputs are skipped on resume; effect states DispatchBegun/UnknownPendingReconciliation block.
//!
//! Store keys: `lease` (worker|fence|attempt|expires), `in/N`, `out/N` (E1 record + `F <effect>` line), `eff/N`.
//! Known limit: the lease touch and the `out/N` CAS are two store operations; safety rests on the `out/N`
//! CAS having exactly one winner (a superseded worker that loses it reports `StaleFence`).
use crate::lib_codec::{decode_full, encode_full};
use crate::store::JobStore;
use abi::*;

pub struct ExecOptions {
    pub job_id: String,
    pub worker_id: String,
    pub lease_seconds: u64,
    /// Clock (unix seconds); injected so tests and the binary control time.
    pub now: Box<dyn Fn() -> u64>,
    /// Test hook: simulated kill right after handler N committed.
    pub crash_after: Option<usize>,
    /// Called after handler N committed (the binary uses it for the kill marker).
    pub after_commit: Option<Box<dyn Fn(usize)>>,
}

impl ExecOptions {
    /// Fixed clock at `now`, 60 s lease.
    pub fn new(job_id: &str, worker_id: &str, now: u64) -> Self {
        Self {
            job_id: job_id.into(),
            worker_id: worker_id.into(),
            lease_seconds: 60,
            now: Box::new(move || now),
            crash_after: None,
            after_commit: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExecError {
    /// Live lease held by someone else (`now < expires_at`).
    LeaseHeld { expires_at: u64 },
    /// Fence superseded, lease expired, or lost the commit CAS.
    StaleFence,
    /// An effect of handler N may have happened and was not acknowledged: reconcile, never re-run.
    NeedsReconciliation(usize),
    Crashed(usize),
    Handler(HandlerError),
    Store(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub worker_id: String,
    pub fence_token: u64,
    pub attempt: u64,
    pub expires_at: u64,
}

fn se(e: String) -> ExecError {
    ExecError::Store(e)
}

fn parse_lease(s: &str) -> Result<Lease, ExecError> {
    let p: Vec<&str> = s.split('|').collect();
    if p.len() != 4 {
        return Err(se("corrupt lease".into()));
    }
    let n = |i: usize| p[i].parse::<u64>().map_err(|_| se("corrupt lease".into()));
    Ok(Lease { worker_id: p[0].into(), fence_token: n(1)?, attempt: n(2)?, expires_at: n(3)? })
}

fn fmt_lease(l: &Lease) -> String {
    format!("{}|{}|{}|{}", l.worker_id, l.fence_token, l.attempt, l.expires_at)
}

/// Current lease record, if any (expires_at 0 = released).
pub fn read_lease(store: &dyn JobStore) -> Result<Option<Lease>, ExecError> {
    match store.get("lease").map_err(se)? {
        Some((_, v)) => parse_lease(&v).map(Some),
        None => Ok(None),
    }
}

fn blocking(e: EffectState) -> bool {
    matches!(e, EffectState::DispatchBegun | EffectState::UnknownPendingReconciliation)
}

/// Verifies we still hold the current, unexpired fence; returns the lease version read.
fn verify(store: &dyn JobStore, me: &Fence, now: u64) -> Result<u64, ExecError> {
    match store.get("lease").map_err(se)? {
        Some((v, s)) => {
            let l = parse_lease(&s)?;
            if l.fence_token == me.fence_token && l.worker_id == me.worker_id && now < l.expires_at {
                Ok(v)
            } else {
                Err(ExecError::StaleFence)
            }
        }
        None => Err(ExecError::StaleFence),
    }
}

/// Runs (or resumes) the handlers linearly under a lease; returns the final payload.
pub fn execute(
    store: &dyn JobStore,
    handlers: &[Box<dyn JobHandler>],
    initial: &str,
    opts: &ExecOptions,
) -> Result<String, ExecError> {
    if opts.worker_id.is_empty() || opts.worker_id.contains(|c: char| c == '|' || c.is_control()) {
        return Err(ExecError::Handler(HandlerError::Invalid("bad worker id".into())));
    }
    if initial.contains(char::is_control) {
        return Err(ExecError::Handler(HandlerError::Invalid("initial payload contains a newline".into())));
    }
    // ---- claim: reclaimable at now >= expires; fence and attempt +1
    let now = (opts.now)();
    let (ver, prev) = match store.get("lease").map_err(se)? {
        Some((v, s)) => (v, Some(parse_lease(&s)?)),
        None => (0, None),
    };
    if let Some(p) = &prev
        && now < p.expires_at
    {
        return Err(ExecError::LeaseHeld { expires_at: p.expires_at });
    }
    // C-7: a job with an unreconciled effect is not claimable
    for i in 0..handlers.len() {
        if store.get(&format!("eff/{i}")).map_err(se)?.is_some() && store.get(&format!("out/{i}")).map_err(se)?.is_none() {
            return Err(ExecError::NeedsReconciliation(i));
        }
    }
    let lease = Lease {
        worker_id: opts.worker_id.clone(),
        fence_token: prev.as_ref().map_or(1, |p| p.fence_token + 1),
        attempt: prev.as_ref().map_or(1, |p| p.attempt + 1),
        expires_at: now + opts.lease_seconds,
    };
    let mut lease_ver = store.cas("lease", ver, &fmt_lease(&lease)).map_err(se)?;
    let fence = Fence { worker_id: lease.worker_id.clone(), fence_token: lease.fence_token, attempt: lease.attempt };

    let mut payload = initial.to_string();
    for (i, h) in handlers.iter().enumerate() {
        if let Some((_, rec)) = store.get(&format!("out/{i}")).map_err(se)? {
            let (p, _, effect) = decode_full(&rec);
            if blocking(effect) {
                return Err(ExecError::NeedsReconciliation(i));
            }
            payload = p; // committed by an earlier attempt: skipped, effects included
            continue;
        }
        let ik = format!("in/{i}");
        let input_payload = match store.get(&ik).map_err(se)? {
            Some((_, v)) => v,
            None => {
                store.cas(&ik, 0, &payload).map_err(se)?;
                payload.clone()
            }
        };
        if h.effectful() {
            verify(store, &fence, (opts.now)())?;
            store.cas(&format!("eff/{i}"), 0, "UnknownPendingReconciliation").map_err(se)?;
        }
        let input = InputEnvelope { job_id: opts.job_id.clone(), step_index: i, payload: input_payload };
        let out = h.run(&fence, &input).map_err(ExecError::Handler)?;
        for t in std::iter::once(&out.payload).chain(out.events.iter()) {
            if t.contains(char::is_control) {
                return Err(ExecError::Handler(HandlerError::Invalid("payload or event contains a newline".into())));
            }
        }
        // commit: still the current unexpired fence (and renew), then the single-winner out/N CAS
        let now = (opts.now)();
        let v = verify(store, &fence, now)?;
        let renewed = Lease { expires_at: now + opts.lease_seconds, ..lease.clone() };
        lease_ver = store.cas("lease", v, &fmt_lease(&renewed)).map_err(|_| ExecError::StaleFence)?;
        store.cas(&format!("out/{i}"), 0, &encode_full(&out)).map_err(|_| ExecError::StaleFence)?;
        payload = out.payload.clone();
        if let Some(f) = &opts.after_commit {
            f(i);
        }
        if blocking(out.effect) {
            return Err(ExecError::NeedsReconciliation(i));
        }
        if opts.crash_after == Some(i) {
            return Err(ExecError::Crashed(i));
        }
    }
    // release (best effort): the job is done, the lease need not run out
    let released = Lease { expires_at: 0, ..lease };
    let _ = store.cas("lease", lease_ver, &fmt_lease(&released));
    Ok(payload)
}
