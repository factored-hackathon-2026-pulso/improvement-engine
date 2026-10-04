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
    pub fn set_migrations(&self, _m: Migrations) {}
    pub fn register_task(&self, _name: &str) {}
    pub fn task_running(&self, _name: &str) {}
    pub fn task_exited(&self, _name: &str, _result: Result<(), String>) {}
    pub fn begin_shutdown(&self) {}
    pub fn task_states(&self) -> BTreeMap<String, TaskState> {
        BTreeMap::new()
    }
    pub fn readiness(&self) -> Result<(), String> {
        let _ = (&self.probe, &self.migrations, &self.tasks, &self.shutting_down, Ordering::SeqCst);
        Err("unimplemented".into())
    }
    pub fn healthz(&self) -> (u16, Value) {
        (500, json!({}))
    }
    pub fn readyz(&self) -> (u16, Value) {
        (500, json!({}))
    }
}
