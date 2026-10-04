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
}

impl Config {
    /// Keys: data_mode, adapter, source_id, work_dir, runner_exe (required); poll_interval_secs (30), batch_cap (1000),
    /// min_history_days (14), min_history_cases (200), min_support (5).
    pub fn from_pairs(_pairs: &[(&str, &str)]) -> Result<Config, SourceError> {
        Err(SourceError::BadConfig("todo".into()))
    }
}
