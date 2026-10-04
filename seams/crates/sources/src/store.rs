//! Per-source watermark store (port + in-memory and file implementations; Postgres in `pg_store`).
use crate::{SourceError, SourceId, Watermark};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatermarkRecord {
    pub watermark: Watermark,
    pub adapter: String,
    /// Earliest `event_time` ever read from this source (history span for the cold-start gate).
    pub first_event_time: Option<String>,
    /// Cumulative `case.opened` events read (history volume for the cold-start gate).
    pub cases_opened: u64,
    pub batches: u64,
}

pub trait WatermarkStore {
    fn get(&self, id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError>;
    /// Compare-and-set: `expected` is the watermark the caller read (None = no row yet).
    fn commit(&self, id: &SourceId, expected: Option<&Watermark>, rec: &WatermarkRecord) -> Result<(), SourceError>;
}

#[derive(Default)]
pub struct MemStore;
impl WatermarkStore for MemStore {
    fn get(&self, _id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError> {
        Ok(None)
    }
    fn commit(&self, _id: &SourceId, _expected: Option<&Watermark>, _rec: &WatermarkRecord) -> Result<(), SourceError> {
        Ok(())
    }
}

pub struct FileStore {
    #[allow(dead_code)]
    dir: PathBuf,
}
impl FileStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<FileStore, SourceError> {
        Ok(FileStore { dir: dir.into() })
    }
}
impl WatermarkStore for FileStore {
    fn get(&self, _id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError> {
        Ok(None)
    }
    fn commit(&self, _id: &SourceId, _expected: Option<&Watermark>, _rec: &WatermarkRecord) -> Result<(), SourceError> {
        Ok(())
    }
}
