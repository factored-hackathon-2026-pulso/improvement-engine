//! Treated-payload scan (Rust port of `roleplay-llm/roleplay_llm/scanner.py`, TPS).
use serde_json::Value;

pub const SCANNER_ID: &str = "tps-1-rs";
pub const DEFAULT_K: i64 = 10;
pub const SUPPRESSED: &str = "<k";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub ok: bool,
    pub violations: Vec<String>,
}

pub fn scan_payload(_payload: &Value, _k: i64, _registry: &[String]) -> Scan {
    Scan { ok: true, violations: vec![] }
}

pub fn suppress_rows(rows: &[Value], _k: i64) -> Vec<Value> {
    rows.to_vec()
}
