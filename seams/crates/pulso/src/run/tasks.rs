//! The two long-running tasks of `pulso run` besides HTTP: the monitor loop and the engine job worker.
//!
//! Registration point for other lanes: the monitor loop calls a `Tick` once per poll interval. `StubTick` is the
//! default; `monitor::tick` (R1M) plugs in by implementing `Tick` and replacing the one `Box::new(StubTick)` in
//! `run::build_tasks`. The worker claims jobs from a `JobRepository` and hands each to a `JobRunner`; with no runner
//! registered it stays idle and claims nothing (it never consumes a job it cannot run).
use crate::config::DataMode;
use crate::run::log::Logger;
use crate::run::supervisor::{StopToken, Task};
use pg::repo::{Claimed, JobRepository};
use std::sync::Arc;
use std::time::Duration;

/// What a tick may use. Counts and identifiers only; never row values.
#[derive(Debug, Clone)]
pub struct TickCtx {
    pub data_mode: DataMode,
    pub adapter: String,
    pub batch_cap: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickReport {
    pub processed: u32,
}

pub trait Tick: Send {
    fn tick(&mut self, ctx: &TickCtx) -> Result<TickReport, String>;
}

/// Default monitor: does nothing (reports zero). Replaced by R1M's `monitor::tick`.
pub struct StubTick;
impl Tick for StubTick {
    fn tick(&mut self, _ctx: &TickCtx) -> Result<TickReport, String> {
        Ok(TickReport::default())
    }
}

/// Consecutive failing ticks after which the monitor task gives up (readiness then names it).
pub const MAX_CONSECUTIVE_TICK_FAILURES: u32 = 5;

pub struct MonitorTask {
    tick: Box<dyn Tick>,
    ctx: TickCtx,
    poll: Duration,
    log: Logger,
}

impl MonitorTask {
    pub fn new(tick: Box<dyn Tick>, ctx: TickCtx, poll: Duration, log: Logger) -> MonitorTask {
        MonitorTask { tick, ctx, poll, log }
    }
}

impl Task for MonitorTask {
    fn name(&self) -> String {
        "monitor".into()
    }
    fn run(&mut self, stop: &StopToken) -> Result<(), String> {
        let _ = (&mut self.tick, &self.ctx, self.poll, &self.log, stop);
        Ok(())
    }
}

/// Runs one claimed job. It owns committing outputs through `ctx.repo`; returning `Err` leaves the lease to expire
/// so the job is reclaimed (retry, fence bumped). It must return promptly once `ctx.stop` is signalled.
pub trait JobRunner: Send + Sync {
    fn run(&self, job: &Claimed, ctx: &JobCtx) -> Result<(), String>;
}

pub struct JobCtx<'a> {
    pub repo: &'a dyn JobRepository,
    pub tenant: &'a str,
    pub worker: &'a str,
    pub stop: &'a StopToken,
    pub now: u64,
    pub lease_seconds: u64,
}

pub struct JobWorker {
    pub repo: Arc<dyn JobRepository>,
    pub runner: Option<Arc<dyn JobRunner>>,
    pub tenant: String,
    pub worker_id: String,
    pub poll: Duration,
    pub batch_cap: u32,
    pub lease_seconds: u64,
    pub clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    pub log: Logger,
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Task for JobWorker {
    fn name(&self) -> String {
        "worker".into()
    }
    fn run(&mut self, stop: &StopToken) -> Result<(), String> {
        let _ = (&self.repo, &self.runner, &self.tenant, &self.worker_id, self.poll, self.batch_cap, self.lease_seconds, &self.clock, &self.log, stop);
        Ok(())
    }
}
