//! `pulso monitor` (R1M): loops `sources::monitor::tick` against one source.
//!
//! Exit 0 clean (`--once` done, or stopped by SIGTERM / ctrl-c / stdin EOF with `--exit-on-stdin-eof`); 1 a tick or the adapter
//! failed under `--once`, or the adapter could not be built; 2 usage or an honest config refusal. Secrets (DSNs) come from the
//! environment only, never argv. Output lines carry ids and counts, never row values. A stop request is honoured between
//! ticks: the watermark is committed last inside a tick, so even a hard kill replays at most one batch idempotently.
use sources::config::{AdapterKind, Config};
use sources::monitor::{TickOutcome, tick};
use sources::pg_dataset::DatasetPg;
use sources::pg_product::PostgresProduct;
use sources::pg_store::PgStore;
use sources::sqlite::SqliteProduct;
use sources::store::{FileStore, WatermarkStore};
use sources::{SourceAdapter, SourceError};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const USAGE: &str = "usage: pulso monitor [--once] [--data-mode dataset|platform] [--adapter product-sqlite|product-postgres|dataset-pg] [--source-id ID] [--sqlite FILE] [--schema NAME] [--work-dir DIR] [--runner EXE] [--poll-interval-secs N] [--batch-cap N] [--min-history-days N] [--min-history-cases N] [--exit-on-stdin-eof]\nEnv: PULSO_DATA_MODE, PULSO_SOURCE_ADAPTER, PULSO_SOURCE_ID, PULSO_SOURCE_SQLITE, PULSO_SOURCE_SCHEMA, PULSO_WORK_DIR, STEPS_RUNNER_EXE, PULSO_POLL_INTERVAL_SECS, PULSO_BATCH_CAP; DSNs (secrets, env only): PULSO_PG_PRODUCT_DSN, PULSO_PG_DATASET_DSN (read-only roles), PULSO_PG_WATERMARK_DSN (engine role; default is a file store under the work dir).";

static STOP: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub struct MonitorArgs {
    pub once: bool,
    pub exit_on_stdin_eof: bool,
    pub config: Config,
    pub sqlite: Option<PathBuf>,
    pub schema: Option<String>,
}

pub fn parse(args: &[String], env: &dyn Fn(&str) -> Option<String>) -> Result<MonitorArgs, String> {
    let mut once = false;
    let mut eof = false;
    let mut vals: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    let flag_key = |f: &str| -> Option<&'static str> {
        Some(match f {
            "--data-mode" => "data_mode",
            "--adapter" => "adapter",
            "--source-id" => "source_id",
            "--work-dir" => "work_dir",
            "--runner" => "runner_exe",
            "--poll-interval-secs" => "poll_interval_secs",
            "--batch-cap" => "batch_cap",
            "--min-history-days" => "min_history_days",
            "--min-history-cases" => "min_history_cases",
            "--sqlite" => "sqlite",
            "--schema" => "schema",
            _ => return None,
        })
    };
    for (k, e) in [("data_mode", "PULSO_DATA_MODE"), ("adapter", "PULSO_SOURCE_ADAPTER"), ("source_id", "PULSO_SOURCE_ID"), ("work_dir", "PULSO_WORK_DIR"), ("runner_exe", "STEPS_RUNNER_EXE"), ("poll_interval_secs", "PULSO_POLL_INTERVAL_SECS"), ("batch_cap", "PULSO_BATCH_CAP"), ("sqlite", "PULSO_SOURCE_SQLITE"), ("schema", "PULSO_SOURCE_SCHEMA")] {
        if let Some(v) = env(e).filter(|v| !v.is_empty()) {
            vals.insert(k, v);
        }
    }
    let mut it = args.iter();
    while let Some(f) = it.next() {
        match f.as_str() {
            "--once" => once = true,
            "--exit-on-stdin-eof" => eof = true,
            other => {
                let k = flag_key(other).ok_or_else(|| format!("unknown argument {other:?}"))?;
                vals.insert(k, it.next().cloned().ok_or_else(|| format!("{other} needs a value"))?);
            }
        }
    }
    if !vals.contains_key("runner_exe") {
        let sibling = std::env::current_exe().ok().map(|mut p| {
            p.set_file_name(format!("pulso-synth-runner{}", std::env::consts::EXE_SUFFIX));
            p
        });
        match sibling.filter(|p| p.is_file()) {
            Some(p) => {
                vals.insert("runner_exe", p.display().to_string());
            }
            None => return Err("no sensor runner: pass --runner or set STEPS_RUNNER_EXE".into()),
        }
    }
    let sqlite = vals.remove("sqlite").map(PathBuf::from);
    let schema = vals.remove("schema");
    let pairs: Vec<(&str, &str)> = vals.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let config = Config::from_pairs(&pairs).map_err(|e| e.to_string())?;
    if config.adapter == AdapterKind::ProductSqlite && sqlite.is_none() {
        return Err("product-sqlite needs --sqlite FILE (or PULSO_SOURCE_SQLITE)".into());
    }
    Ok(MonitorArgs { once, exit_on_stdin_eof: eof, config, sqlite, schema })
}

fn dsn(var: &str) -> Result<String, SourceError> {
    std::env::var(var).ok().filter(|v| !v.is_empty()).ok_or_else(|| SourceError::BadConfig(format!("{var} is not set")))
}

fn build_adapter(a: &MonitorArgs) -> Result<Box<dyn SourceAdapter>, SourceError> {
    let id = a.config.source_id.clone();
    Ok(match a.config.adapter {
        AdapterKind::ProductSqlite => Box::new(SqliteProduct::open(a.sqlite.as_deref().unwrap_or_else(|| std::path::Path::new("")), id)?),
        AdapterKind::ProductPostgres => Box::new(PostgresProduct::connect(&dsn("PULSO_PG_PRODUCT_DSN")?, a.schema.as_deref().unwrap_or("product"), id)?),
        AdapterKind::DatasetPg => Box::new(DatasetPg::connect(&dsn("PULSO_PG_DATASET_DSN")?, a.schema.as_deref().unwrap_or("raw"), id)?),
    })
}

fn build_store(a: &MonitorArgs) -> Result<Box<dyn WatermarkStore>, SourceError> {
    match std::env::var("PULSO_PG_WATERMARK_DSN").ok().filter(|v| !v.is_empty()) {
        Some(d) => Ok(Box::new(PgStore::connect(&d)?)),
        None => Ok(Box::new(FileStore::open(a.config.work_dir.join("watermarks"))?)),
    }
}

#[cfg(windows)]
mod signals {
    use super::STOP;
    use std::sync::atomic::Ordering;
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }
    unsafe extern "system" fn on_ctrl(_kind: u32) -> i32 {
        STOP.store(true, Ordering::SeqCst);
        1
    }
    pub fn install() {
        // SAFETY: registers a handler that only stores to an atomic.
        unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
    }
}

#[cfg(unix)]
mod signals {
    use super::STOP;
    use std::sync::atomic::Ordering;
    unsafe extern "C" {
        fn signal(sig: i32, handler: usize) -> usize;
    }
    extern "C" fn on_sig(_sig: i32) {
        STOP.store(true, Ordering::SeqCst);
    }
    pub fn install() {
        // SAFETY: the handler only stores to an atomic (async-signal-safe).
        unsafe {
            signal(2, on_sig as usize); // SIGINT
            signal(15, on_sig as usize); // SIGTERM
        }
    }
}

fn one_tick(a: &MonitorArgs, adapter: &dyn SourceAdapter, store: &dyn WatermarkStore) -> Result<(String, bool), SourceError> {
    let head = format!("pulso monitor: source={} data_mode={} adapter={}", a.config.source_id.as_str(), a.config.data_mode.as_str(), a.config.adapter.as_str());
    Ok(match tick(&a.config, adapter, store)? {
        TickOutcome::Idle { watermark } => (format!("{head} idle watermark={}", watermark.encode()), false),
        TickOutcome::Processed { run_id, events, admitted, quarantined, to, more, .. } => (format!("{head} run={run_id} events={events} admitted={admitted} quarantined={quarantined} watermark={} more={more}", to.encode()), more),
    })
}

/// `args` are the arguments after `monitor`. Returns the process exit code.
pub fn main(args: &[String]) -> i32 {
    let env = |k: &str| std::env::var(k).ok();
    let a = match parse(args, &env) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("pulso monitor: {e}\n{USAGE}");
            return 2;
        }
    };
    signals::install();
    if a.exit_on_stdin_eof && !a.once {
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            STOP.store(true, Ordering::SeqCst);
        });
    }
    let store = match build_store(&a) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("pulso monitor: cannot open the watermark store: {e}");
            return 1;
        }
    };
    let mut adapter: Option<Box<dyn SourceAdapter>> = None;
    while !STOP.load(Ordering::SeqCst) {
        let mut backlog = false;
        let built = match adapter.take() {
            Some(x) => Ok(x),
            None => build_adapter(&a),
        };
        match built.and_then(|ad| one_tick(&a, ad.as_ref(), store.as_ref()).map(|r| (ad, r))) {
            Ok((ad, (line, more))) => {
                println!("{line}");
                backlog = more;
                adapter = Some(ad);
            }
            Err(e) => {
                eprintln!("pulso monitor: tick failed: {e}");
                if a.once {
                    return 1;
                }
            }
        }
        if a.once {
            break;
        }
        let until = Instant::now() + if backlog { Duration::ZERO } else { a.config.poll_interval };
        while Instant::now() < until && !STOP.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    0
}
