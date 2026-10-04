//! Health / readiness state machine: /healthz is process-up only; /readyz names the first failing reason.
use pulso::health::{DbProbe, Health, Migrations};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct Probe(AtomicBool);
impl DbProbe for Probe {
    fn ping(&self) -> Result<(), String> {
        if self.0.load(Ordering::SeqCst) { Ok(()) } else { Err("connection refused".into()) }
    }
}

fn health(db_up: bool) -> (Arc<Health>, Arc<Probe>) {
    let p = Arc::new(Probe(AtomicBool::new(db_up)));
    (Health::new(p.clone()), p)
}

#[test]
fn liveness_is_up_even_when_everything_else_is_red() {
    let (h, _) = health(false);
    h.set_migrations(Migrations::Failed("boom".into()));
    h.begin_shutdown();
    let (status, body) = h.healthz();
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
}

#[test]
fn readiness_walks_the_reasons_in_order() {
    let (h, db) = health(false);
    h.register_task("monitor");
    assert_eq!(h.readiness().unwrap_err(), "migrations_pending");
    h.set_migrations(Migrations::Failed("0003: syntax".into()));
    assert_eq!(h.readiness().unwrap_err(), "migrations_failed");
    h.set_migrations(Migrations::Applied);
    assert_eq!(h.readiness().unwrap_err(), "db_unreachable");
    db.0.store(true, Ordering::SeqCst);
    assert_eq!(h.readiness().unwrap_err(), "task_starting:monitor");
    h.task_running("monitor");
    assert_eq!(h.readiness(), Ok(()));
    h.task_exited("monitor", Err("lost connection".into()));
    assert_eq!(h.readiness().unwrap_err(), "task_dead:monitor");
    h.task_running("monitor");
    assert_eq!(h.readiness(), Ok(()));
    h.begin_shutdown();
    assert_eq!(h.readiness().unwrap_err(), "shutting_down");
}

#[test]
fn not_applicable_migrations_are_ready_for_the_memory_mode() {
    let (h, _) = health(true);
    h.set_migrations(Migrations::NotApplicable);
    assert_eq!(h.readiness(), Ok(()));
}

#[test]
fn readyz_response_is_503_with_a_named_reason_and_never_the_probe_text() {
    let (h, _) = health(false);
    h.set_migrations(Migrations::Applied);
    let (status, body) = h.readyz();
    assert_eq!(status, 503);
    assert_eq!(body["ready"], false);
    assert_eq!(body["reason"], "db_unreachable");
    assert!(!body.to_string().contains("connection refused"), "probe text may carry a host name");
    let (h2, _) = health(true);
    h2.set_migrations(Migrations::Applied);
    let (status, body) = h2.readyz();
    assert_eq!((status, body["ready"].as_bool()), (200, Some(true)));
}
