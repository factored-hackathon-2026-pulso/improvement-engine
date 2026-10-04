//! Monitor configuration. Unknown keys, unknown values and inconsistent combinations are refused (never defaulted).
use crate::{DataMode, SourceError, SourceId};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterKind {
    ProductSqlite,
    ProductPostgres,
    DatasetPg,
}
impl AdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AdapterKind::ProductSqlite => "product-sqlite",
            AdapterKind::ProductPostgres => "product-postgres",
            AdapterKind::DatasetPg => "dataset-pg",
        }
    }
}

/// Which sensor follows the package: the fixed-output `claude-standin` runner or the R1G `rust-events` sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorKind {
    StandIn,
    RustEvents,
}
impl SensorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SensorKind::StandIn => "stand-in",
            SensorKind::RustEvents => "rust-events",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub data_mode: DataMode,
    pub adapter: AdapterKind,
    pub source_id: SourceId,
    pub poll_interval: Duration,
    pub batch_cap: usize,
    pub work_dir: PathBuf,
    pub runner_exe: PathBuf,
    pub min_history_days: u32,
    pub min_history_cases: u32,
    pub min_support: u32,
    pub sensor: SensorKind,
    pub k_anon: u32,
    pub min_cell_cases: u32,
}

impl Config {
    /// Keys: data_mode, adapter, source_id, work_dir, runner_exe (required); poll_interval_secs (30), batch_cap (1000),
    /// min_history_days (14), min_history_cases (200), min_support (5), sensor (`rust-events` in platform mode, `stand-in` in dataset mode), k_anon (10), min_cell_cases (30).
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Result<Config, SourceError> {
        let bad = |m: String| SourceError::BadConfig(m);
        let mut get: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for (k, v) in pairs {
            const KEYS: &[&str] = &["data_mode", "adapter", "source_id", "work_dir", "runner_exe", "poll_interval_secs", "batch_cap", "min_history_days", "min_history_cases", "min_support", "sensor", "k_anon", "min_cell_cases"];
            if !KEYS.contains(k) {
                return Err(bad(format!("unknown config key {k:?}")));
            }
            if get.insert(k, v).is_some() {
                return Err(bad(format!("duplicate config key {k:?}")));
            }
        }
        let req = |k: &str| get.get(k).copied().filter(|v| !v.is_empty()).ok_or_else(|| bad(format!("missing {k}")));
        let num = |k: &str, default: u64, lo: u64, hi: u64| -> Result<u64, SourceError> {
            let n = match get.get(k) {
                None => default,
                Some(v) => v.parse::<u64>().map_err(|_| bad(format!("{k} must be a whole number")))?,
            };
            if (lo..=hi).contains(&n) { Ok(n) } else { Err(bad(format!("{k} must be {lo}..={hi}"))) }
        };
        let data_mode = DataMode::parse(req("data_mode")?).ok_or_else(|| bad("data_mode must be dataset or platform".into()))?;
        let adapter = match req("adapter")? {
            "product-sqlite" => AdapterKind::ProductSqlite,
            "product-postgres" => AdapterKind::ProductPostgres,
            "dataset-pg" => AdapterKind::DatasetPg,
            other => return Err(bad(format!("unknown adapter {other:?}"))),
        };
        if (data_mode == DataMode::Dataset) != (adapter == AdapterKind::DatasetPg) {
            return Err(bad(format!("adapter {} cannot serve data_mode {}", adapter.as_str(), data_mode.as_str())));
        }
        let sensor = match get.get("sensor").copied() {
            None if data_mode == DataMode::Platform => SensorKind::RustEvents,
            None => SensorKind::StandIn,
            Some("rust-events") => SensorKind::RustEvents,
            Some("stand-in") => SensorKind::StandIn,
            Some(other) => return Err(bad(format!("unknown sensor {other:?}"))),
        };
        Ok(Config {
            data_mode,
            adapter,
            source_id: SourceId::new(data_mode, req("source_id")?)?,
            poll_interval: Duration::from_secs(num("poll_interval_secs", 30, 1, 86_400)?),
            batch_cap: num("batch_cap", 1000, 1, crate::policy::HARD_CAP as u64)? as usize,
            work_dir: PathBuf::from(req("work_dir")?),
            runner_exe: PathBuf::from(req("runner_exe")?),
            min_history_days: num("min_history_days", 14, 14, 3650)? as u32,
            min_history_cases: num("min_history_cases", 200, 200, 10_000_000)? as u32,
            min_support: num("min_support", 5, 1, 1_000_000)? as u32,
            sensor,
            k_anon: num("k_anon", 10, 1, 1_000_000)? as u32,
            min_cell_cases: num("min_cell_cases", 30, 1, 1_000_000)? as u32,
        })
    }
}
