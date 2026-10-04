//! The single-process supervisor: runs every `Task` on its own thread, tracks them in `Health`, and on stop gives
//! them a bounded grace period; a task that ignores the stop is cut at the deadline (the process then exits non-zero).
use crate::health::Health;
use crate::run::log::Logger;
use serde_json::json;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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
        *self.inner.0.lock().unwrap() = true;
        self.inner.1.notify_all();
    }
    pub fn is_stopped(&self) -> bool {
        *self.inner.0.lock().unwrap()
    }
    /// Sleeps up to `d`; returns true when stopped (early wake-up on stop).
    pub fn wait(&self, d: Duration) -> bool {
        let guard = self.inner.0.lock().unwrap();
        let (guard, _) = self.inner.1.wait_timeout_while(guard, d, |stopped| !*stopped).unwrap();
        *guard
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
        if !self.cut.is_empty() {
            3
        } else if !self.failed.is_empty() {
            1
        } else {
            0
        }
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
        let Supervisor { health, log, grace, stop, tasks } = self;
        let (done_tx, done_rx) = mpsc::channel::<String>();
        let mut names = Vec::new();
        let mut handles = Vec::new();
        for mut task in tasks {
            let name = task.name();
            health.register_task(&name);
            names.push(name.clone());
            let (h, l, s, tx) = (health.clone(), log.clone(), stop.clone(), done_tx.clone());
            let spawned = std::thread::Builder::new().name(format!("pulso-task-{name}")).spawn(move || {
                h.task_running(&name);
                l.info("task_started", json!({"task": name}));
                let result = std::panic::catch_unwind(AssertUnwindSafe(|| task.run(&s))).unwrap_or_else(|_| Err("panicked".into()));
                if !s.is_stopped() {
                    // returned before anyone asked it to stop: it is dead, readiness names it
                    h.task_exited(&name, result.clone());
                    l.error("task_died", json!({"task": name, "reason": result.err().unwrap_or_else(|| "exited".into())}));
                } else {
                    l.info("task_stopped", json!({"task": name, "ok": result.is_ok()}));
                }
                let _ = tx.send(name);
            });
            handles.push(spawned);
        }
        drop(done_tx);
        // Block until asked to stop (signal, stdin EOF, or a caller).
        while !stop.wait(Duration::from_millis(250)) {}
        health.begin_shutdown();
        log.info("shutdown_begin", json!({"grace_ms": grace.as_millis() as u64}));
        let deadline = Instant::now() + grace;
        let mut finished = std::collections::BTreeSet::new();
        while finished.len() < names.len() {
            let left = deadline.saturating_duration_since(Instant::now());
            match done_rx.recv_timeout(left) {
                Ok(n) => {
                    finished.insert(n);
                }
                Err(_) => break,
            }
        }
        let states = health.task_states();
        let cut: Vec<String> = names.iter().filter(|n| !finished.contains(*n)).cloned().collect();
        // `failed` = died on its own before the stop (those are marked Dead; clean shutdown leaves them Running).
        let failed: Vec<String> = names.iter().filter(|n| matches!(states.get(*n), Some(crate::health::TaskState::Dead(_)))).cloned().collect();
        for h in handles.into_iter().flatten() {
            // join only the finished ones; a cut task's thread is abandoned and dies with the process
            if h.is_finished() {
                let _ = h.join();
            }
        }
        let out = Outcome { failed, cut };
        log.info("shutdown_end", json!({"exit_code": out.exit_code(), "cut": out.cut.len(), "failed": out.failed.len()}));
        out
    }
}
