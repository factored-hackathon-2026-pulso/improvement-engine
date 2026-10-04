//! R1M source adapters. See `policy` (allow-list), `sqlite`/`pg_product`/`pg_dataset` (adapters), `store` (watermarks).
pub mod policy;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SourceError {
    AccessDenied(String),
    BadLimit(usize),
    Io(String),
}
