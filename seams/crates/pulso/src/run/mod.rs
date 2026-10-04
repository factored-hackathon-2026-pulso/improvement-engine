//! `pulso run`: the single-process runtime entrypoint (supervisor, tasks, health, logs).
pub mod db;
pub mod http;
pub mod log;
pub mod signals;
pub mod supervisor;
pub mod tasks;

use crate::config::{RunConfig, Storage};
use crate::health::{DbProbe, Health, Migrations};
use crate::run::http::HttpTask;
use crate::run::log::Logger;
use crate::run::supervisor::{Supervisor, Task};
use crate::run::tasks::{Gated, JobWorker, MonitorTask, StubTick, TickCtx, unix_now};
use pg::repo::{JobRepository, MemRepo};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const USAGE: &str = "usage: pulso run [--exit-on-stdin-eof]\nConfiguration is environment only: PULSO_DATABASE_URL | PULSO_STORAGE=memory, PULSO_DATA_MODE=dataset|platform, \
PULSO_SOURCE_ADAPTER, PULSO_POLL_INTERVAL_MS, PULSO_BATCH_CAP, PULSO_LISTEN_ADDR, PULSO_ALLOW_NON_LOOPBACK, PULSO_DEBUG_TOKEN, PULSO_ADMIN_TOKEN, \
PULSO_BASE_PATH, PULSO_CONSOLE_DIR, PULSO_STORE_DIR, PULSO_STORAGE_PREFIX, PULSO_SHUTDOWN_GRACE_SECS, PULSO_TENANT, PULSO_WORKER_ID, PULSO_EXIT_ON_STDIN_EOF.";

struct AlwaysUp;
impl DbProbe for AlwaysUp {
    fn ping(&self) -> Result<(), String> {
        Ok(())
    }
}

/// The task registration point. Other lanes replace `StubTick` with their monitor tick (`monitor::tick`) and give the
/// worker its `JobRunner` here; everything else (migrations gate, health, shutdown) is already wired.
pub fn build_tasks(cfg: &RunConfig, log: &Logger, health: &Arc<Health>, repo: Arc<dyn JobRepository>) -> Vec<Box<dyn Task>> {
    let ctx = TickCtx { data_mode: cfg.data_mode, adapter: cfg.adapter.clone(), batch_cap: cfg.batch_cap };
    let monitor = MonitorTask::new(Box::new(StubTick), ctx, cfg.poll_interval, log.clone());
    let worker = JobWorker {
        repo,
        runner: None,
        tenant: cfg.tenant.clone(),
        worker_id: cfg.worker_id.clone(),
        poll: cfg.poll_interval,
        batch_cap: cfg.batch_cap,
        lease_seconds: 60,
        clock: Arc::new(unix_now),
        log: log.clone(),
    };
    vec![Box::new(Gated { inner: Box::new(monitor), health: health.clone() }), Box::new(Gated { inner: Box::new(worker), health: health.clone() })]
}

/// `pulso run`. Exit 0 clean stop; 1 failed task or startup failure; 2 refused config/usage; 3 cut at the deadline.
pub fn main(args: &[String]) -> i32 {
    let mut eof_flag = false;
    for a in args {
        match a.as_str() {
            "--exit-on-stdin-eof" => eof_flag = true,
            other => {
                eprintln!("pulso run: unknown argument {other:?}\n{USAGE}");
                return 2;
            }
        }
    }
    let cfg = match RunConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pulso run: refusing to start: {e}\n{USAGE}");
            return 2;
        }
    };
    let log = Logger::stdout();
    let (probe, pgcfg): (Arc<dyn DbProbe>, Option<postgres::Config>) = match (cfg.storage, &cfg.database_url) {
        (Storage::Postgres, Some(url)) => match db::pg_config(url) {
            Ok(c) => (Arc::new(db::PgProbe::new(c.clone())), Some(c)),
            Err(e) => {
                eprintln!("pulso run: refusing to start: config_invalid: {e}");
                return 2;
            }
        },
        _ => (Arc::new(AlwaysUp), None),
    };
    let health = Health::new(probe);
    let store = Arc::new(match &cfg.store_dir {
        Some(d) => match debug_api::Store::open(d) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("pulso run: cannot open PULSO_STORE_DIR");
                return 1;
            }
        },
        None => debug_api::Store::memory(),
    });
    let http = match HttpTask::bind(&cfg, health.clone(), store) {
        Ok(h) => h,
        Err(e) => {
            log.error("bind_failed", json!({"reason": e}));
            eprintln!("pulso run: {e}");
            return 1;
        }
    };
    let addr = http.local_addr();
    log.info(
        "run_config",
        json!({
            "data_mode": format!("{:?}", cfg.data_mode).to_lowercase(),
            "adapter": cfg.adapter,
            "storage": format!("{:?}", cfg.storage).to_lowercase(),
            "poll_ms": cfg.poll_interval.as_millis() as u64,
            "batch_cap": cfg.batch_cap,
            "grace_s": cfg.grace.as_secs(),
            "base_path": cfg.base_path,
            "tenant": cfg.tenant,
            "worker_id": cfg.worker_id,
            "auth": cfg.debug_token.is_some(),
            "admin": cfg.admin_token.is_some(),
            "storage_prefix": cfg.storage_prefix
        }),
    );
    let repo: Arc<dyn JobRepository> = match &pgcfg {
        Some(c) => Arc::new(pg::pgrepo::PgRepo::shared(c.clone())),
        None => Arc::new(MemRepo::new()),
    };
    let mut sup = Supervisor::new(health.clone(), log.clone(), cfg.grace);
    sup.add(Box::new(http));
    for t in build_tasks(&cfg, &log, &health, repo) {
        sup.add(t);
    }
    let stop = sup.stop_token();
    signals::install(stop.clone());
    if eof_flag || cfg.exit_on_stdin_eof {
        signals::stop_on_stdin_eof(stop.clone());
    }
    match pgcfg {
        None => health.set_migrations(Migrations::NotApplicable),
        Some(c) => {
            let (h, l, s) = (health.clone(), log.clone(), stop.clone());
            std::thread::Builder::new()
                .name("pulso-migrate".into())
                .spawn(move || {
                    loop {
                        match db::apply(&c) {
                            Ok(r) => {
                                l.info("migrations_applied", json!({"applied": r.applied.len(), "already": r.skipped.len()}));
                                h.set_migrations(Migrations::Applied);
                                return;
                            }
                            Err(e) => {
                                l.error("migrations_failed", json!({"reason": e}));
                                h.set_migrations(Migrations::Failed(e));
                            }
                        }
                        if s.wait(Duration::from_secs(2)) {
                            return;
                        }
                    }
                })
                .ok();
        }
    }
    log.info("listening", json!({"addr": addr.to_string()}));
    sup.run().exit_code()
}
