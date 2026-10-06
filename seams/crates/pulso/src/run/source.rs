//! The monitor task of `pulso run`: the real `sources::monitor::tick` over the adapter the configuration names.
//!
//! Each tick reads a batch through the read-only adapter, writes the package and the run record under the work dir, and
//! hands the run to the engine job queue as one job keyed `monitor:<run_id>`. The hand-over happens BEFORE the watermark
//! moves (`tick_with`): a kill in between replays the same batch with the same run id, and the keyed admission finds the
//! job it already queued, so nothing is lost and nothing is duplicated. A backlog larger than the read batch is drained
//! within one call. `stub` keeps the do-nothing `StubTick` (tests, health checks).
use crate::config::{DataMode as RunMode, RunConfig};
use crate::run::tasks::{StubTick, Tick, TickCtx, TickReport};
use pg::repo::JobRepository;
use sources::config::{AdapterKind, Config};
use sources::monitor::{TickOutcome, tick_with};
use sources::pg_dataset::DatasetPg;
use sources::pg_product::PostgresProduct;
use sources::pg_store::PgStore;
use sources::sqlite::SqliteProduct;
use sources::store::{FileStore, WatermarkStore};
use sources::{SourceAdapter, SourceError};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Reads per `tick` call (a source that keeps producing more than this per poll is drained over the next polls).
const MAX_READS_PER_TICK: usize = 200;

/// Key prefix of the jobs this task admits; the worker strips it to find the run record.
pub const JOB_KEY_PREFIX: &str = "monitor:";

pub struct SourceTick {
    cfg: Config,
    sqlite: Option<PathBuf>,
    schema: Option<String>,
    store: Box<dyn WatermarkStore + Send>,
    repo: Arc<dyn JobRepository>,
    tenant: String,
    swept: bool,
}

fn dsn(var: &str) -> Result<String, SourceError> {
    std::env::var(var).ok().filter(|v| !v.is_empty()).ok_or_else(|| SourceError::BadConfig(format!("{var} is not set")))
}

/// A driver error may echo connection details: never let a DSN reach a log line.
fn scrub(e: &SourceError) -> String {
    let t = e.to_string();
    if t.contains("postgres://") || t.contains("postgresql://") { "source error (details withheld: they may carry a connection string)".into() } else { t }
}

/// The `sources` configuration a `pulso run` configuration stands for.
pub fn source_config(c: &RunConfig, runner: &Path) -> Result<Config, String> {
    let work = c.work_dir.as_ref().ok_or("PULSO_WORK_DIR is not set")?;
    let mode = if c.data_mode == RunMode::Dataset { "dataset" } else { "platform" };
    let adapter = if c.adapter.starts_with("dataset-") { "dataset-pg" } else { c.adapter.as_str() };
    let (work, runner, batch) = (work.display().to_string(), runner.display().to_string(), c.read_batch.to_string());
    let mut pairs: Vec<(&str, &str)> = vec![("data_mode", mode), ("adapter", adapter), ("source_id", &c.source_id), ("work_dir", &work), ("runner_exe", &runner), ("batch_cap", &batch)];
    // EVT2: platform cells feed (payload keys, case_type, opened_at, salted customer key). The salt is a secret: environment only.
    let salt = std::env::var("PULSO_CUSTOMER_KEY_SALT").ok().filter(|v| !v.is_empty());
    if std::env::var("PULSO_PLATFORM_CELLS").is_ok_and(|v| v == "on") {
        pairs.push(("platform_cells", "on"));
        if let Some(s) = salt.as_deref() {
            pairs.push(("customer_key_salt", s));
        }
    }
    Config::from_pairs(&pairs).map_err(|e| e.to_string())
}

/// The tick `pulso run` registers for this configuration: `StubTick` for the `stub` adapter, `SourceTick` otherwise.
pub fn build_tick(c: &RunConfig, repo: Arc<dyn JobRepository>, tenant: &str, runner: &Path) -> Result<Box<dyn Tick>, String> {
    if c.adapter == "stub" {
        return Ok(Box::new(StubTick));
    }
    let work = c.work_dir.as_ref().ok_or("PULSO_WORK_DIR is not set")?;
    let store: Box<dyn WatermarkStore + Send> = match std::env::var("PULSO_PG_WATERMARK_DSN").ok().filter(|v| !v.is_empty()) {
        Some(d) => Box::new(PgStore::connect(&d).map_err(|e| format!("watermark store: {}", scrub(&e)))?),
        None => Box::new(FileStore::open(work.join("watermarks")).map_err(|e| format!("watermark store: {e}"))?),
    };
    Ok(Box::new(SourceTick::with_store(c, repo, tenant, runner, store)?))
}

impl SourceTick {
    pub fn with_store(c: &RunConfig, repo: Arc<dyn JobRepository>, tenant: &str, runner: &Path, store: Box<dyn WatermarkStore + Send>) -> Result<SourceTick, String> {
        let default_schema = match c.adapter.as_str() {
            "dataset-augmented" => Some("augmented".to_string()),
            "dataset-raw" | "dataset-pg" => Some("raw".to_string()),
            "product-postgres" => Some("product".to_string()),
            _ => None,
        };
        Ok(SourceTick { cfg: source_config(c, runner)?, sqlite: c.source_sqlite.clone(), schema: c.source_schema.clone().or(default_schema), store, repo, tenant: tenant.to_string(), swept: false })
    }

    fn build_adapter(&self) -> Result<Box<dyn SourceAdapter>, SourceError> {
        let id = self.cfg.source_id.clone();
        Ok(match self.cfg.adapter {
            AdapterKind::ProductSqlite => {
                let path = self.sqlite.as_deref().ok_or_else(|| SourceError::BadConfig("PULSO_SOURCE_SQLITE is not set".into()))?;
                if self.cfg.platform_cells { Box::new(SqliteProduct::open_platform(path, id)?) } else { Box::new(SqliteProduct::open(path, id)?) }
            }
            AdapterKind::ProductPostgres => Box::new(PostgresProduct::connect(&dsn("PULSO_PG_PRODUCT_DSN")?, self.schema.as_deref().unwrap_or("product"), id)?),
            AdapterKind::DatasetPg => Box::new(DatasetPg::connect(&dsn("PULSO_PG_DATASET_DSN")?, self.schema.as_deref().unwrap_or("raw"), id)?),
        })
    }
}

impl SourceTick {
    /// Once per process: queue again (keyed, so idempotent) every run record of this source already on disk. The watermark is durable but
    /// an in-memory queue is not: a kill after the watermark commit and before the worker finished would otherwise lose the job for good.
    /// Finished runs are cheap to re-admit: the engine job finds them completed in the console store and does nothing.
    fn sweep(&mut self) -> Result<(), String> {
        let Some(dir) = self.cfg.work_dir.join("runs").read_dir().ok() else { return Ok(()) };
        let mut runs: Vec<(String, String)> = vec![];
        for e in dir.flatten() {
            let Some(rec) = std::fs::read_to_string(e.path()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
            if rec["source_id"] != self.cfg.source_id.as_str() {
                continue;
            }
            if let Some(id) = rec["run_id"].as_str() {
                runs.push((rec["observed_until"].as_str().unwrap_or("").to_string(), id.to_string()));
            }
        }
        runs.sort();
        for (_, id) in runs {
            self.repo.admit_keyed(&self.tenant, &format!("{JOB_KEY_PREFIX}{id}")).map_err(|e| format!("cannot re-queue run {id}: {e:?}"))?;
        }
        self.swept = true;
        Ok(())
    }
}

impl Tick for SourceTick {
    fn tick(&mut self, _ctx: &TickCtx) -> Result<TickReport, String> {
        if !self.swept {
            self.sweep()?;
        }
        let adapter = self.build_adapter().map_err(|e| scrub(&e))?;
        let (repo, tenant) = (self.repo.clone(), self.tenant.clone());
        let hand_over = |run_id: &str, _record: &serde_json::Value| -> Result<(), SourceError> {
            repo.admit_keyed(&tenant, &format!("{JOB_KEY_PREFIX}{run_id}")).map(|_| ()).map_err(|e| SourceError::Io(format!("cannot enqueue the engine job: {e:?}")))
        };
        let mut processed = 0u32;
        for _ in 0..MAX_READS_PER_TICK {
            match tick_with(&self.cfg, adapter.as_ref(), self.store.as_ref(), &hand_over).map_err(|e| scrub(&e))? {
                TickOutcome::Idle { .. } => break,
                TickOutcome::Processed { events, more, .. } => {
                    processed = processed.saturating_add(u32::try_from(events).unwrap_or(u32::MAX));
                    if !more {
                        break;
                    }
                }
            }
        }
        Ok(TickReport { processed })
    }
}
