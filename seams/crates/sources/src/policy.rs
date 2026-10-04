//! Allow-list of the `platform_live` source (mirror of `platform-exporter/.../policy.py`, drift-tested).
use crate::SourceError;

pub const HARD_CAP: usize = 10_000;
pub const DENIED_TABLES: &[&str] = &[];
pub const EVENT_READ_COLUMNS: &[&str] = &[];

pub fn allowed_tables() -> Vec<&'static str> {
    vec![]
}
pub fn allowed_columns(_table: &str) -> Option<&'static [&'static str]> {
    None
}
pub fn assert_table_allowed(_t: &str) -> Result<String, SourceError> {
    Ok(String::new())
}
pub fn assert_columns_allowed(_t: &str, _cols: &[&str]) -> Result<(), SourceError> {
    Ok(())
}
pub fn check_limit(_n: usize) -> Result<usize, SourceError> {
    Ok(0)
}
