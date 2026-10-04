//! `monitor::tick`: one read-batch -> package -> sensor job -> run record -> watermark commit cycle.
use crate::config::Config;
use crate::store::WatermarkStore;
use crate::{SourceAdapter, SourceError, Watermark};

#[derive(Debug)]
pub enum TickOutcome {
    Idle { watermark: Watermark },
    Processed { run_id: String, events: usize, admitted: usize, quarantined: usize, from: Watermark, to: Watermark, more: bool, record: serde_json::Value },
}

pub fn tick(_config: &Config, _adapter: &dyn SourceAdapter, _store: &dyn WatermarkStore) -> Result<TickOutcome, SourceError> {
    Err(SourceError::Io("todo".into()))
}
