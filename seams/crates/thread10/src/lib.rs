//! Q1 thread10: the ten-step E2E-THREAD-01 report on the Rust shell, offline. See `docs/` THREAD01.md (Q1 section).
use serde_json::Value;
use std::path::PathBuf;

pub struct Opts {
    pub work: PathBuf,
    pub runner: PathBuf,
    pub human_override: bool,
    pub denied_kind: bool,
    pub sha: String,
}

pub struct Run {
    pub events: Vec<String>,
    pub error: Option<String>,
    pub report: Value,
}

pub fn run(_o: &Opts) -> Result<Run, String> {
    Err("not implemented".into())
}
