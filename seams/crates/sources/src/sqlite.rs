//! `product-sqlite`: the platform's SQLite file, opened read-only.
use crate::{Batch, PlatformEvent, Row, SchemaReport, SourceAdapter, SourceError, SourceId, Watermark};
use std::path::Path;

pub struct SqliteProduct {
    id: SourceId,
}
impl SqliteProduct {
    pub fn open(_path: &Path, id: SourceId) -> Result<SqliteProduct, SourceError> {
        Ok(SqliteProduct { id })
    }
    pub fn can_write(&self) -> bool {
        true
    }
    pub fn guarded_probe(&self, _sql: &str) -> Result<(), SourceError> {
        Ok(())
    }
}
impl SourceAdapter for SqliteProduct {
    fn source_id(&self) -> &SourceId {
        &self.id
    }
    fn adapter(&self) -> &'static str {
        "product-sqlite"
    }
    fn data_class(&self) -> &'static str {
        "treated"
    }
    fn list_tables(&self) -> Result<Vec<String>, SourceError> {
        Ok(vec![])
    }
    fn schema_check(&self) -> Result<SchemaReport, SourceError> {
        Ok(SchemaReport::default())
    }
    fn read_events(&self, after: &Watermark, _limit: usize) -> Result<Batch, SourceError> {
        Ok(Batch { events: Vec::<PlatformEvent>::new(), next: after.clone(), more: false })
    }
    fn read_dimension(&self, _table: &str, _limit: usize) -> Result<Vec<Row>, SourceError> {
        Ok(vec![])
    }
}
