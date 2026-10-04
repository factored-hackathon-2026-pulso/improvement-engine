//! Persistence port of control-api LITE: `MemStore` (in memory) and `pgstore::PgStore` (Postgres, CPG). The app serialises
//! its own get-modify-put sequences, but every single call below is atomic and single-winner on its own, so two
//! processes (or threads without the app's lock) sharing one database still bind and store exactly once.
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

/// A replay-set row outlives its `exp` by this many seconds: processes sharing one database may disagree on `now`, and a token
/// is still accepted (`exp > now`) by a process whose clock is behind the one that evicts.
pub const JTI_SKEW_SECS: f64 = 120.0;

pub trait Store: Send + Sync {
    fn binding(&self, tenant: &str, command_key: &str) -> Option<BindingRec>;
    fn job_owner(&self, tenant: &str, job_id: &str) -> Option<String>;
    /// The single effect of a binding: records it, its job owner and its `task_binding_ref -> tenant` mapping, once.
    /// `false` (nothing written) when the `(tenant, command_key)` is already bound, the job id belongs to another command
    /// key, or the `task_binding_ref` is owned by another tenant.
    fn put_binding(&self, tenant: &str, command_key: &str, rec: BindingRec) -> bool;
    fn binding_ref_tenant(&self, binding_ref: &str) -> Option<String>;
    /// First owner wins: a ref already owned (by any tenant) is never re-assigned.
    fn preauthorize_binding_ref(&self, binding_ref: &str, tenant: &str);
    fn binding_effects(&self, tenant: &str, job_id: &str) -> u32;
    /// Idempotent by `(tenant, artifact id)`; `Conflict` when the same id exists with a different `artifact` ref.
    fn put_artifact(&self, tenant: &str, envelope: Value) -> PutOutcome;
    fn get_artifact(&self, tenant: &str, id: &str) -> Option<Value>;
    /// Namespaced, tenant-scoped JSON documents (ingest ledger/cursors/receipts/quarantine, grants, lab queries, run events).
    /// The app serialises writers, so a get-modify-put needs no atomicity from the store.
    fn put_doc(&self, ns: &str, tenant: &str, id: &str, doc: Value);
    /// Insert-if-absent: `true` for the single writer that created the document, `false` (nothing written) when it exists.
    /// The default is get-then-put (the app serialises writers); stores shared by several processes override it atomically.
    fn put_doc_new(&self, ns: &str, tenant: &str, id: &str, doc: Value) -> bool {
        if self.get_doc(ns, tenant, id).is_some() {
            return false;
        }
        self.put_doc(ns, tenant, id, doc);
        true
    }
    fn get_doc(&self, ns: &str, tenant: &str, id: &str) -> Option<Value>;
    /// All documents of `(ns, tenant)`, ordered by id.
    fn list_docs(&self, ns: &str, tenant: &str) -> Vec<(String, Value)>;
    /// Receiver-owned JWT replay set: records `(scope, iss, jti)` until `exp` and returns `true` the first time only.
    /// Entries whose expiry is `<= now` are evicted first, so the set stays bounded.
    fn jti_claim(&self, scope: &str, iss: &str, jti: &str, exp: f64, now: f64) -> bool;
    /// Whether `jti_claim` survives a restart (then the verifier needs no boot floor).
    fn durable_replay(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct Inner {
    bindings: HashMap<(String, String), BindingRec>,
    jobs: HashMap<(String, String), String>,
    effects: HashMap<(String, String), u32>,
    refs: HashMap<String, String>,
    artifacts: HashMap<(String, String), Value>,
    docs: std::collections::BTreeMap<(String, String, String), Value>,
    jti: HashMap<(String, String, String), f64>,
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
    fn put_binding(&self, tenant: &str, key: &str, rec: BindingRec) -> bool {
        let mut g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let owned_elsewhere = g.refs.get(&rec.task_binding_ref).is_some_and(|t| t != tenant);
        if g.bindings.contains_key(&(tenant.into(), key.into())) || g.jobs.contains_key(&(tenant.into(), rec.job_id.clone())) || owned_elsewhere {
            return false;
        }
        g.jobs.insert((tenant.into(), rec.job_id.clone()), key.into());
        *g.effects.entry((tenant.into(), rec.job_id.clone())).or_default() += 1;
        g.refs.insert(rec.task_binding_ref.clone(), tenant.into());
        g.bindings.insert((tenant.into(), key.into()), rec);
        true
    }
    fn binding_ref_tenant(&self, r: &str) -> Option<String> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).refs.get(r).cloned()
    }
    fn preauthorize_binding_ref(&self, r: &str, tenant: &str) {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).refs.entry(r.into()).or_insert_with(|| tenant.into());
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
    fn put_doc_new(&self, ns: &str, tenant: &str, id: &str, doc: Value) -> bool {
        let mut g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match g.docs.entry((ns.into(), tenant.into(), id.into())) {
            std::collections::btree_map::Entry::Occupied(_) => false,
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert(doc);
                true
            }
        }
    }
    fn get_doc(&self, ns: &str, tenant: &str, id: &str) -> Option<Value> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).docs.get(&(ns.into(), tenant.into(), id.into())).cloned()
    }
    fn list_docs(&self, ns: &str, tenant: &str) -> Vec<(String, Value)> {
        let g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.docs.iter().filter(|((n, t, _), _)| n == ns && t == tenant).map(|((_, _, id), v)| (id.clone(), v.clone())).collect()
    }
    fn jti_claim(&self, scope: &str, iss: &str, jti: &str, exp: f64, now: f64) -> bool {
        let mut g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.jti.retain(|_, e| *e + JTI_SKEW_SECS > now);
        match g.jti.entry((scope.into(), iss.into(), jti.into())) {
            std::collections::hash_map::Entry::Occupied(_) => false,
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(exp);
                true
            }
        }
    }
}
