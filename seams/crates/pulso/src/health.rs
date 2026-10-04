//! Process health (`/healthz`) and readiness (`/readyz`) for `pulso run`.
//! Liveness only says the process answers. Readiness names the first failing reason, in a fixed order:
//! shutting_down, migrations_pending|migrations_failed, db_unreachable, task_starting:<t>, task_dead:<t>.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

/// Is the database reachable right now? Implemented over Postgres by `pulso run`; always-Ok in memory mode.
pub trait DbProbe: Send + Sync {
    fn ping(&self) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migrations {
    Pending,
    Applied,
    Failed(String),
    /// Memory mode: there is no schema to migrate.
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Starting,
    Running,
    Dead(String),
}

pub struct Health {
    probe: Arc<dyn DbProbe>,
    migrations: Mutex<Migrations>,
    tasks: Mutex<BTreeMap<String, TaskState>>,
    shutting_down: AtomicBool,
}

impl Health {
    pub fn new(probe: Arc<dyn DbProbe>) -> Arc<Health> {
        Arc::new(Health { probe, migrations: Mutex::new(Migrations::Pending), tasks: Mutex::new(BTreeMap::new()), shutting_down: AtomicBool::new(false) })
    }
    pub fn set_migrations(&self, m: Migrations) {
        *self.migrations.lock().unwrap() = m;
    }
    pub fn migrations(&self) -> Migrations {
        self.migrations.lock().unwrap().clone()
    }
    pub fn register_task(&self, name: &str) {
        self.tasks.lock().unwrap().insert(name.into(), TaskState::Starting);
    }
    pub fn task_running(&self, name: &str) {
        self.tasks.lock().unwrap().insert(name.into(), TaskState::Running);
    }
    /// A task that returned (Ok or Err) before shutdown is dead: readiness goes red naming it.
    pub fn task_exited(&self, name: &str, result: Result<(), String>) {
        let why = result.err().unwrap_or_else(|| "exited".into());
        self.tasks.lock().unwrap().insert(name.into(), TaskState::Dead(why));
    }
    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
    }
    pub fn task_states(&self) -> BTreeMap<String, TaskState> {
        self.tasks.lock().unwrap().clone()
    }
    /// `Err(reason)` with a short stable code; never carries probe text (it can contain host names).
    pub fn readiness(&self) -> Result<(), String> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("shutting_down".into());
        }
        match &*self.migrations.lock().unwrap() {
            Migrations::Pending => return Err("migrations_pending".into()),
            Migrations::Failed(_) => return Err("migrations_failed".into()),
            Migrations::Applied | Migrations::NotApplicable => {}
        }
        if self.probe.ping().is_err() {
            return Err("db_unreachable".into());
        }
        let tasks = self.tasks.lock().unwrap();
        for (name, st) in tasks.iter() {
            match st {
                TaskState::Starting => return Err(format!("task_starting:{name}")),
                TaskState::Dead(_) => return Err(format!("task_dead:{name}")),
                TaskState::Running => {}
            }
        }
        Ok(())
    }
    pub fn healthz(&self) -> (u16, Value) {
        (200, json!({"ok": true}))
    }
    pub fn readyz(&self) -> (u16, Value) {
        match self.readiness() {
            Ok(()) => (200, json!({"ready": true})),
            Err(reason) => (503, json!({"ready": false, "reason": reason})),
        }
    }
}
