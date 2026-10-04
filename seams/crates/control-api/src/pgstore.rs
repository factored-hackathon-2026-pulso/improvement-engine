//! Postgres-backed `Store` (CPG) over migration 0052 (`pulso_ca_*`). SKELETON: every method is still to be written.
use crate::store::{BindingRec, PutOutcome, Store};
use serde_json::Value;

pub struct PgStore;

impl PgStore {
    pub fn connect(_url: &str) -> Result<PgStore, String> {
        Err("not implemented".into())
    }
}

impl Store for PgStore {
    fn binding(&self, _: &str, _: &str) -> Option<BindingRec> {
        todo!()
    }
    fn job_owner(&self, _: &str, _: &str) -> Option<String> {
        todo!()
    }
    fn put_binding(&self, _: &str, _: &str, _: BindingRec) -> bool {
        todo!()
    }
    fn binding_ref_tenant(&self, _: &str) -> Option<String> {
        todo!()
    }
    fn preauthorize_binding_ref(&self, _: &str, _: &str) {
        todo!()
    }
    fn binding_effects(&self, _: &str, _: &str) -> u32 {
        todo!()
    }
    fn put_artifact(&self, _: &str, _: Value) -> PutOutcome {
        todo!()
    }
    fn get_artifact(&self, _: &str, _: &str) -> Option<Value> {
        todo!()
    }
    fn put_doc(&self, _: &str, _: &str, _: &str, _: Value) {
        todo!()
    }
    fn get_doc(&self, _: &str, _: &str, _: &str) -> Option<Value> {
        todo!()
    }
    fn list_docs(&self, _: &str, _: &str) -> Vec<(String, Value)> {
        todo!()
    }
    fn jti_claim(&self, _: &str, _: &str, _: &str, _: f64, _: f64) -> bool {
        todo!()
    }
}
