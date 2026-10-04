//! R1M source adapters: a read-only `SourceAdapter` port with `product-sqlite`, `product-postgres` and `dataset-pg`
//! implementations, a per-source watermark store and the monitor tick. Every adapter builds its SQL from the allow-list in
//! `policy` and never returns denylisted tables or columns.
pub mod config;
pub mod policy;
pub mod sqlite;
pub mod store;

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SourceError {
    AccessDenied(String),
    BadLimit(usize),
    BadWatermark(String),
    BadSourceId(String),
    SchemaDrift(String),
    Conflict(String),
    BadConfig(String),
    Io(String),
}
impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SourceError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataMode {
    Dataset,
    Platform,
}
impl DataMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DataMode::Dataset => "dataset",
            DataMode::Platform => "platform",
        }
    }
    pub fn parse(s: &str) -> Option<DataMode> {
        match s {
            "dataset" => Some(DataMode::Dataset),
            "platform" => Some(DataMode::Platform),
            _ => None,
        }
    }
    /// Section 4 of the design: dataset replays are never labelled production.
    pub fn data_origin(self) -> &'static str {
        match self {
            DataMode::Dataset => "dataset_replay",
            DataMode::Platform => "platform_live",
        }
    }
}

/// Stable per-source id, `dataset:<name>` or `platform:<name>`; the prefix must match the data mode so sources never mix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(String);
impl SourceId {
    pub fn new(mode: DataMode, id: &str) -> Result<SourceId, SourceError> {
        let rest = id.strip_prefix(&format!("{}:", mode.as_str()));
        let ok = rest.is_some_and(|r| {
            !r.is_empty()
                && id.len() <= 96
                && r.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-' | b':'))
        });
        if ok { Ok(SourceId(id.to_owned())) } else { Err(SourceError::BadSourceId(id.to_owned())) }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn mode(&self) -> DataMode {
        if self.0.starts_with("dataset:") { DataMode::Dataset } else { DataMode::Platform }
    }
}

/// Where the next read starts. Product: `event_log.sequence`. Datasets: `_ingested_at` + `_batch_id` (+ the row key, so a
/// batch larger than the read cap resumes without losing rows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watermark {
    Sequence(i64),
    Dataset { ingested_at: String, batch_id: String, key: String },
}
impl Watermark {
    pub fn origin_for(adapter: &str) -> Watermark {
        if adapter == "dataset-pg" {
            Watermark::Dataset { ingested_at: String::new(), batch_id: String::new(), key: String::new() }
        } else {
            Watermark::Sequence(0)
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Watermark::Sequence(_) => "sequence",
            Watermark::Dataset { .. } => "dataset",
        }
    }
    pub fn encode(&self) -> String {
        match self {
            Watermark::Sequence(n) => format!("seq:{n}"),
            Watermark::Dataset { ingested_at, batch_id, key } => format!("ds:{}|{}|{}", esc(ingested_at), esc(batch_id), esc(key)),
        }
    }
    pub fn decode(s: &str) -> Result<Watermark, SourceError> {
        let bad = || SourceError::BadWatermark(s.chars().take(40).collect());
        if let Some(n) = s.strip_prefix("seq:") {
            return n.parse::<i64>().ok().filter(|n| *n >= 0).map(Watermark::Sequence).ok_or_else(bad);
        }
        let parts: Vec<&str> = s.strip_prefix("ds:").ok_or_else(bad)?.split('|').collect();
        if let [a, b, k] = parts[..] {
            return Ok(Watermark::Dataset { ingested_at: unesc(a), batch_id: unesc(b), key: unesc(k) });
        }
        Err(bad())
    }
}
fn esc(s: &str) -> String {
    s.replace('%', "%25").replace('|', "%7C")
}
fn unesc(s: &str) -> String {
    s.replace("%7C", "|").replace("%25", "%")
}

/// One platform event (contract 1.1.0 `event_log` minus payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformEvent {
    /// `event_log.sequence` for product sources; `None` for dataset replays (no platform sequence exists).
    pub sequence: Option<i64>,
    pub event_id: String,
    pub event_type: String,
    pub entity: Option<String>,
    pub entity_id: Option<String>,
    pub case_id: Option<String>,
    pub actor_role: Option<String>,
    pub actor_id: Option<String>,
    pub event_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub events: Vec<PlatformEvent>,
    /// Watermark to commit once the batch is processed (equals the input watermark when `events` is empty).
    pub next: Watermark,
    /// True when more rows were available beyond the cap.
    pub more: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaReport {
    /// Allow-listed tables that exist.
    pub tables: Vec<String>,
    /// (table, column) required by the adapter but absent: drift, fail closed.
    pub missing: Vec<(String, String)>,
    /// Denylisted tables that exist in the source; reported by name, never read.
    pub denied_present: Vec<String>,
}
impl SchemaReport {
    pub fn ok(&self) -> bool {
        self.missing.is_empty()
    }
}

pub type Row = BTreeMap<String, Option<String>>;

/// Read-only source port. `read_events` never returns more than `limit` (<= `policy::HARD_CAP`) events.
pub trait SourceAdapter {
    fn source_id(&self) -> &SourceId;
    fn adapter(&self) -> &'static str;
    /// `treated` (allow-listed columns only) or `e0`.
    fn data_class(&self) -> &'static str;
    /// Allow-listed tables present in the source; denylisted or unknown tables are never listed.
    fn list_tables(&self) -> Result<Vec<String>, SourceError>;
    fn schema_check(&self) -> Result<SchemaReport, SourceError>;
    fn read_events(&self, after: &Watermark, limit: usize) -> Result<Batch, SourceError>;
    /// Allow-listed columns of up to `limit` rows of an allow-listed dimension table (never `event_log`).
    fn read_dimension(&self, table: &str, limit: usize) -> Result<Vec<Row>, SourceError>;
}
