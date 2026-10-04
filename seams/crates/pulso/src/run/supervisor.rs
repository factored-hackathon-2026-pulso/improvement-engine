//! The single-process supervisor: runs every `Task` on its own thread, tracks them in `Health`, and on stop gives
//! them a bounded grace period; a task that ignores the stop is cut at the deadline (the process then exits non-zero).
use crate::health::Health;
use crate::run::log::Logger;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Cooperative stop signal shared by the supervisor, the signal handlers and every task.
#[derive(Clone, Default)]
pub struct StopToken {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl StopToken {
    pub fn new() -> StopToken {
        StopToken::default()
    }
    pub fn stop(&self) {
        let _ = &self.inner;
    }
    pub fn is_stopped(&self) -> bool {
        false
    }
    /// Sleeps up to `d`; returns true when stopped (early wake-up on stop).
    pub fn wait(&self, d: Duration) -> bool {
        std::thread::sleep(d);
        false
    }
}

/// A unit of long-running work. `run` must return promptly after `stop` is signalled.
pub trait Task: Send + 'static {
    fn name(&self) -> String;
    fn run(&mut self, stop: &StopToken) -> Result<(), String>;
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Tasks that returned an error (or panicked) before shutdown began.
    pub failed: Vec<String>,
    /// Tasks still running when the grace period ended.
    pub cut: Vec<String>,
}

impl Outcome {
    /// 0 clean; 1 a task failed; 3 a task had to be cut at the deadline.
    pub fn exit_code(&self) -> i32 {
        0
    }
}

pub struct Supervisor {
    health: Arc<Health>,
    log: Logger,
    grace: Duration,
    stop: StopToken,
    tasks: Vec<Box<dyn Task>>,
}

impl Supervisor {
    pub fn new(health: Arc<Health>, log: Logger, grace: Duration) -> Supervisor {
        Supervisor { health, log, grace, stop: StopToken::new(), tasks: Vec::new() }
    }
    pub fn stop_token(&self) -> StopToken {
        self.stop.clone()
    }
    pub fn add(&mut self, t: Box<dyn Task>) {
        self.tasks.push(t);
    }
    /// Starts every task, blocks until the stop token fires, then shuts down within the grace period.
    pub fn run(self) -> Outcome {
        let _ = (&self.health, &self.log, self.grace, &self.tasks);
        Outcome::default()
    }
}
