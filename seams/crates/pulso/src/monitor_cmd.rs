//! `pulso monitor` (R1M): loops `sources::monitor::tick`. Stub for the RED step.
use std::path::PathBuf;

#[derive(Debug)]
pub struct MonitorArgs {
    pub once: bool,
    pub config: sources::config::Config,
    pub sqlite: Option<PathBuf>,
    pub schema: Option<String>,
}

pub fn parse(_args: &[String], _env: &dyn Fn(&str) -> Option<String>) -> Result<MonitorArgs, String> {
    Err("todo".into())
}

pub fn main(_args: &[String]) -> i32 {
    2
}
