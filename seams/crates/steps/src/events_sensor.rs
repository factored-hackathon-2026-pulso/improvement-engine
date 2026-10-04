//! STUB (RED): real Rust sensor over event packages. See the GREEN commit.
use crate::StepError;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Params {
    pub min_support: u32,
    pub min_cell_cases: u32,
    pub k_anon: u32,
    pub min_history_days: u32,
    pub min_history_cases: u32,
    pub alpha: f64,
    pub discovery_frac: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params { min_support: 5, min_cell_cases: 30, k_anon: 10, min_history_days: 14, min_history_cases: 200, alpha: 0.05, discovery_frac: 0.6 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub metric_id: String,
    pub family: String,
    pub cell: String,
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Discard {
    pub metric_id: String,
    pub family: String,
    pub cell: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub signals: Vec<Signal>,
    pub discards: Vec<Discard>,
    pub drift: Vec<String>,
    pub quarantined: u64,
    pub tests_run: u64,
}

impl Report {
    pub fn steps_output(&self, _run_id: &str, _data_class: &str) -> String {
        String::new()
    }
    pub fn to_json(&self) -> String {
        String::new()
    }
}

pub fn analyze(_events: &str, _cases: &str, _p: &Params) -> Report {
    Report::default()
}

pub fn analyze_package(_root: &Path, _pkg: &str, _p: &Params) -> Result<Report, StepError> {
    Err(StepError::Io("not implemented".into()))
}
