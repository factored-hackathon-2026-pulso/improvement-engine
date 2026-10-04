//! The two long-running tasks of `pulso run` besides HTTP: the monitor loop and the engine job worker.
//!
//! Registration point for other lanes: the monitor loop calls a `Tick` once per poll interval. `StubTick` is the
//! default; `monitor::tick` (R1M) plugs in by implementing `Tick` and replacing the one `Box::new(StubTick)` in
//! `run::build_tasks`. The worker claims jobs from a `JobRepository` and hands each to a `JobRunner`; with no runner
//! registered it stays idle and claims nothing (it never consumes a job it cannot run).
use crate::config::DataMode;
use crate::health::{Health, Migrations};
use crate::run::log::Logger;
use crate::run::supervisor::{StopToken, Task};
use pg::repo::{Claimed, JobRepository};
use serde_json::json;
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
        let mut failures = 0u32;
        while !stop.is_stopped() {
            match self.tick.tick(&self.ctx) {
                Ok(r) => {
                    failures = 0;
                    self.log.info("monitor_tick", json!({"processed": r.processed, "adapter": self.ctx.adapter}));
                }
                Err(e) => {
                    failures += 1;
                    self.log.warn("monitor_tick_failed", json!({"consecutive": failures, "reason": e}));
                    if failures >= MAX_CONSECUTIVE_TICK_FAILURES {
                        return Err(format!("{failures} consecutive tick failures"));
                    }
                }
            }
            if stop.wait(self.poll) {
                break;
            }
        }
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
        let Some(runner) = self.runner.clone() else {
            self.log.info("worker_idle_no_runner", json!({}));
            while !stop.wait(self.poll) {}
            return Ok(());
        };
        while !stop.is_stopped() {
            let mut ran = 0u32;
            while ran < self.batch_cap && !stop.is_stopped() {
                let now = (self.clock)();
                match self.repo.claim_next(&self.tenant, &self.worker_id, now, self.lease_seconds) {
                    Ok(Some(job)) => {
                        ran += 1;
                        let ctx = JobCtx { repo: self.repo.as_ref(), tenant: &self.tenant, worker: &self.worker_id, stop, now, lease_seconds: self.lease_seconds };
                        match runner.run(&job, &ctx) {
                            Ok(()) => {
                                // The terminal transition: without it the finished job is reclaimed once its lease lapses (double execution).
                                match self.repo.complete(&self.tenant, &job.job, &self.worker_id, job.fence_token, (self.clock)()) {
                                    Ok(()) => self.log.info("job_done", json!({"job": job.job, "fence": job.fence_token, "attempt": job.attempt})),
                                    Err(e) => self.log.warn("job_complete_refused", json!({"job": job.job, "attempt": job.attempt, "reason": format!("{e:?}")})),
                                }
                            }
                            Err(e) => self.log.warn("job_failed", json!({"job": job.job, "attempt": job.attempt, "reason": e})),
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        self.log.error("claim_failed", json!({"reason": format!("{e:?}")}));
                        break;
                    }
                }
            }
            if stop.wait(self.poll) {
                break;
            }
        }
        Ok(())
    }
}

/// Holds a task back until the schema is ready (applied or not applicable); it never runs against a half-migrated database.
pub struct Gated {
    pub inner: Box<dyn Task>,
    pub health: Arc<Health>,
}

impl Task for Gated {
    fn name(&self) -> String {
        self.inner.name()
    }
    fn run(&mut self, stop: &StopToken) -> Result<(), String> {
        loop {
            match self.health.migrations() {
                Migrations::Applied | Migrations::NotApplicable => break,
                Migrations::Pending | Migrations::Failed(_) => {
                    if stop.wait(Duration::from_millis(50)) {
                        return Ok(());
                    }
                }
            }
        }
        self.inner.run(stop)
    }
}
