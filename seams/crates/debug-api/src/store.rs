//! Run-event store: per-run append-only log with monotonic sequence, a purge floor and a projection snapshot.
use crate::event::{NewEvent, RunEventSink};
use serde_json::Value;
use std::path::Path;

pub struct Store {}

impl Store {
    pub fn memory() -> Store {
        todo!()
    }
    pub fn open(dir: impl AsRef<Path>) -> Result<Store, String> {
        let _ = dir;
        todo!()
    }
    pub fn runs(&self) -> Vec<String> {
        todo!()
    }
    pub fn state(&self, run: &str) -> Option<Value> {
        let _ = run;
        todo!()
    }
    pub fn head(&self, run: &str) -> Option<i64> {
        let _ = run;
        todo!()
    }
    pub fn floor(&self, run: &str) -> Option<i64> {
        let _ = run;
        todo!()
    }
    pub fn events_after(&self, run: &str, after: i64, limit: usize) -> Vec<Value> {
        let _ = (run, after, limit);
        todo!()
    }
    pub fn purge_through(&self, run: &str, seq: i64) -> Result<(), String> {
        let _ = (run, seq);
        todo!()
    }
}

impl RunEventSink for Store {
    fn emit(&self, run_id: &str, ev: NewEvent) -> Result<Value, String> {
        let _ = (run_id, ev);
        todo!()
    }
}
