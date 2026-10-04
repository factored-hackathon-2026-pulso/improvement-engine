//! `pulso run`: the single-process runtime entrypoint (supervisor, tasks, health, logs).
pub mod db;
pub mod engine_job;
pub mod http;
pub mod log;
pub mod models;
pub mod source;
pub mod signals;
pub mod supervisor;
pub mod tasks;

use crate::config::{RunConfig, Storage};
use crate::health::{DbProbe, Health, Migrations};
use crate::run::http::HttpTask;
use crate::run::log::Logger;
use crate::run::supervisor::{Supervisor, Task};
use crate::run::tasks::{Gated, JobRunner, JobWorker, MonitorTask, StubTick, Tick, TickCtx, unix_now};
use pg::repo::{JobRepository, MemRepo};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const USAGE: &str = "usage: pulso run [--exit-on-stdin-eof]\nConfiguration is environment only: PULSO_DATABASE_URL | PULSO_STORAGE=memory, PULSO_DATA_MODE=dataset|platform, \
PULSO_SOURCE_ADAPTER, PULSO_POLL_INTERVAL_MS, PULSO_BATCH_CAP, PULSO_LISTEN_ADDR, PULSO_ALLOW_NON_LOOPBACK, PULSO_DEBUG_TOKEN, PULSO_ADMIN_TOKEN, \
PULSO_BASE_PATH, PULSO_CONSOLE_DIR, PULSO_STORE_DIR, PULSO_STORAGE_PREFIX, PULSO_SHUTDOWN_GRACE_SECS, PULSO_TENANT, PULSO_WORKER_ID, PULSO_EXIT_ON_STDIN_EOF.
Source: PULSO_WORK_DIR, PULSO_SOURCE_ID, PULSO_SOURCE_SQLITE, PULSO_SOURCE_SCHEMA, PULSO_READ_BATCH, PULSO_SOURCE_PROVENANCE=simulated|real; PULSO_PG_PRODUCT_DSN, PULSO_PG_DATASET_DSN, PULSO_PG_WATERMARK_DSN (secrets, env only); STEPS_RUNNER_EXE.
Ports: PULSO_MODEL_PORT=scripted|roleplay|gateway (PULSO_ROLEPLAY_QUEUE, PULSO_MODEL_GATEWAY), PULSO_CORE_PORT=offline|live.";

struct AlwaysUp;
impl DbProbe for AlwaysUp {
    fn ping(&self) -> Result<(), String> {
        Ok(())
    }
}

/// The sensor-step runner binary the thread needs (`STEPS_RUNNER_EXE`, else `pulso-synth-runner` next to this executable).
pub fn runner_exe() -> Option<std::path::PathBuf> {
    std::env::var_os("STEPS_RUNNER_EXE").map(std::path::PathBuf::from).filter(|p| p.is_file()).or_else(|| {
        let mut p = std::env::current_exe().ok()?;
        p.set_file_name(format!("pulso-synth-runner{}", std::env::consts::EXE_SUFFIX));
        p.is_file().then_some(p)
    })
}

/// The task registration point. With the `stub` adapter the monitor does nothing and the worker has no runner (health checks, tests);
/// with a real adapter the monitor is `source::SourceTick` (the real `monitor::tick`, job hand-over before the watermark) and the
/// worker runs `engine_job::EngineRunner` (`run_signals` per admitted signal, events into `store` in process).
pub fn build_tasks(cfg: &RunConfig, log: &Logger, health: &Arc<Health>, repo: Arc<dyn JobRepository>, store: Arc<debug_api::Store>) -> Result<Vec<Box<dyn Task>>, String> {
    let ctx = TickCtx { data_mode: cfg.data_mode, adapter: cfg.adapter.clone(), batch_cap: cfg.batch_cap };
    let (tick, runner): (Box<dyn Tick>, Option<Arc<dyn JobRunner>>) = if cfg.adapter == "stub" {
        (Box::new(StubTick), None)
    } else {
        let exe = runner_exe().ok_or("no sensor-step runner: set STEPS_RUNNER_EXE or keep pulso-synth-runner next to the pulso executable")?;
        let work = cfg.work_dir.as_ref().ok_or("PULSO_WORK_DIR is not set")?;
        let tick = source::build_tick(cfg, repo.clone(), &cfg.tenant, &exe)?;
        let job = engine_job::EngineRunner::new(cfg, work, &exe, store)?;
        (tick, Some(Arc::new(job)))
    };
    let monitor = MonitorTask::new(tick, ctx, cfg.poll_interval, log.clone());
    let worker = JobWorker {
        repo,
        runner,
        tenant: cfg.tenant.clone(),
        worker_id: cfg.worker_id.clone(),
        poll: cfg.poll_interval,
        batch_cap: cfg.batch_cap,
        // A tick run can hold several proposals (each a ten-step thread); the lease must outlast them.
        lease_seconds: 900,
        clock: Arc::new(unix_now),
        log: log.clone(),
    };
    Ok(vec![Box::new(Gated { inner: Box::new(monitor), health: health.clone() }), Box::new(Gated { inner: Box::new(worker), health: health.clone() })])
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
    let http = match HttpTask::bind(&cfg, health.clone(), store.clone()) {
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
            "storage_prefix": cfg.storage_prefix,
            "source_id": cfg.source_id,
            "work_dir": cfg.work_dir.as_ref().map(|p| p.display().to_string()),
            "read_batch": cfg.read_batch,
            "provenance": format!("{:?}", cfg.provenance).to_lowercase(),
            "model_port": format!("{:?}", cfg.model_port).to_lowercase(),
            "core_port": if cfg.core_live { "live" } else { "offline-double" }
        }),
    );
    let repo: Arc<dyn JobRepository> = match &pgcfg {
        Some(c) => Arc::new(pg::pgrepo::PgRepo::shared(c.clone())),
        None => Arc::new(MemRepo::new()),
    };
    let mut sup = Supervisor::new(health.clone(), log.clone(), cfg.grace);
    sup.add(Box::new(http));
    let tasks = match build_tasks(&cfg, &log, &health, repo, store) {
        Ok(t) => t,
        Err(e) => {
            log.error("startup_refused", json!({"reason": e}));
            eprintln!("pulso run: refusing to start: {e}");
            return 2;
        }
    };
    for t in tasks {
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
                    // Backoff 2s -> 60s: a database that is down or a drifted migration must not hot-loop or flood the log;
                    // an unchanged reason is logged once, not at every attempt.
                    let (mut delay, mut last) = (Duration::from_secs(2), String::new());
                    loop {
                        match db::apply(&c) {
                            Ok(r) => {
                                l.info("migrations_applied", json!({"applied": r.applied.len(), "already": r.skipped.len()}));
                                h.set_migrations(Migrations::Applied);
                                return;
                            }
                            Err(e) => {
                                if e != last {
                                    l.error("migrations_failed", json!({"reason": e, "retry_in_s": delay.as_secs()}));
                                    last = e.clone();
                                }
                                h.set_migrations(Migrations::Failed(e));
                            }
                        }
                        if s.wait(delay) {
                            return;
                        }
                        delay = (delay * 2).min(Duration::from_secs(60));
                    }
                })
                .ok();
        }
    }
    log.info("listening", json!({"addr": addr.to_string()}));
    sup.run().exit_code()
}
