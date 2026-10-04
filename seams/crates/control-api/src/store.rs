//! Persistence port of control-api LITE. Only the in-memory implementation exists (a Postgres-backed store over the
//! `pg` crate is a later package); the app serialises writers itself, so implementations need not be atomic across calls.
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRec {
    pub request_digest: String,
    pub job_id: String,
    pub core_run_id: String,
    pub attempt: i64,
    pub task_binding_ref: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PutOutcome {
    Created,
    Exists,
    Conflict,
}

pub trait Store: Send + Sync {
    fn binding(&self, tenant: &str, command_key: &str) -> Option<BindingRec>;
    fn job_owner(&self, tenant: &str, job_id: &str) -> Option<String>;
    /// The single effect of a binding: records it, its job owner and its `task_binding_ref -> tenant` mapping.
    fn put_binding(&self, tenant: &str, command_key: &str, rec: BindingRec);
    fn binding_ref_tenant(&self, binding_ref: &str) -> Option<String>;
    fn preauthorize_binding_ref(&self, binding_ref: &str, tenant: &str);
    fn binding_effects(&self, tenant: &str, job_id: &str) -> u32;
    /// Idempotent by `(tenant, artifact id)`; `Conflict` when the same id exists with a different `artifact` ref.
    fn put_artifact(&self, tenant: &str, envelope: Value) -> PutOutcome;
    fn get_artifact(&self, tenant: &str, id: &str) -> Option<Value>;
    /// Namespaced, tenant-scoped JSON documents (ingest ledger/cursors/receipts/quarantine, grants, lab queries, run events).
    /// The app serialises writers, so a get-modify-put needs no atomicity from the store.
    fn put_doc(&self, ns: &str, tenant: &str, id: &str, doc: Value);
    fn get_doc(&self, ns: &str, tenant: &str, id: &str) -> Option<Value>;
    /// All documents of `(ns, tenant)`, ordered by id.
    fn list_docs(&self, ns: &str, tenant: &str) -> Vec<(String, Value)>;
}

#[derive(Default)]
struct Inner {
    bindings: HashMap<(String, String), BindingRec>,
    jobs: HashMap<(String, String), String>,
    effects: HashMap<(String, String), u32>,
    refs: HashMap<String, String>,
    artifacts: HashMap<(String, String), Value>,
    docs: std::collections::BTreeMap<(String, String, String), Value>,
}

#[derive(Default)]
pub struct MemStore(Mutex<Inner>);

impl Store for MemStore {
    fn binding(&self, tenant: &str, key: &str) -> Option<BindingRec> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).bindings.get(&(tenant.into(), key.into())).cloned()
    }
    fn job_owner(&self, tenant: &str, job: &str) -> Option<String> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).jobs.get(&(tenant.into(), job.into())).cloned()
    }
    fn put_binding(&self, tenant: &str, key: &str, rec: BindingRec) {
        let mut g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.jobs.insert((tenant.into(), rec.job_id.clone()), key.into());
        *g.effects.entry((tenant.into(), rec.job_id.clone())).or_default() += 1;
        g.refs.insert(rec.task_binding_ref.clone(), tenant.into());
        g.bindings.insert((tenant.into(), key.into()), rec);
    }
    fn binding_ref_tenant(&self, r: &str) -> Option<String> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).refs.get(r).cloned()
    }
    fn preauthorize_binding_ref(&self, r: &str, tenant: &str) {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).refs.insert(r.into(), tenant.into());
    }
    fn binding_effects(&self, tenant: &str, job: &str) -> u32 {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).effects.get(&(tenant.into(), job.into())).copied().unwrap_or(0)
    }
    fn put_artifact(&self, tenant: &str, envelope: Value) -> PutOutcome {
        let id = envelope["artifact"]["id"].as_str().unwrap_or_default().to_string();
        let mut g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match g.artifacts.get(&(tenant.to_string(), id.clone())) {
            Some(prior) if prior["artifact"] == envelope["artifact"] => PutOutcome::Exists,
            Some(_) => PutOutcome::Conflict,
            None => {
                g.artifacts.insert((tenant.to_string(), id), envelope);
                PutOutcome::Created
            }
        }
    }
    fn get_artifact(&self, tenant: &str, id: &str) -> Option<Value> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).artifacts.get(&(tenant.into(), id.into())).cloned()
    }
    fn put_doc(&self, ns: &str, tenant: &str, id: &str, doc: Value) {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).docs.insert((ns.into(), tenant.into(), id.into()), doc);
    }
    fn get_doc(&self, ns: &str, tenant: &str, id: &str) -> Option<Value> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).docs.get(&(ns.into(), tenant.into(), id.into())).cloned()
    }
    fn list_docs(&self, ns: &str, tenant: &str) -> Vec<(String, Value)> {
        let g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.docs.iter().filter(|((n, t, _), _)| n == ns && t == tenant).map(|((_, _, id), v)| (id.clone(), v.clone())).collect()
    }
}
