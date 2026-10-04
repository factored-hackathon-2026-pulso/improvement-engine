//! Routes of control-api LITE. Wire shapes mirror the Python double (`codex_standin.fixtures_app`, `ingest_fixture`):
//! control/broker routes answer errors as `{"code": ...}`, artifact upload as `{"error": ...}`.
use crate::auth::{Denied, Expect, KeyRing, Verifier};
use crate::store::{BindingRec, PutOutcome, Store};
use core_client::canon::{jcs, sha256_hex, z_timestamp};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

pub const BROKER: &str = "/internal/v1/broker";
pub const MAX_ARTIFACT_BYTES: usize = 1024 * 1024;

pub struct Req {
    pub method: String,
    pub path: String,
    /// Raw query string without the `?` (empty when absent).
    pub query: String,
    /// Lower-cased header names.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
}

pub(crate) fn json_resp(status: u16, v: Value) -> Resp {
    Resp { status, body: serde_json::to_vec(&v).expect("json") }
}
pub(crate) fn code(status: u16, c: &str) -> Resp {
    json_resp(status, json!({ "code": c }))
}
pub(crate) fn error(status: u16, c: &str) -> Resp {
    json_resp(status, json!({ "error": c }))
}

pub struct Config {
    pub ring: Arc<KeyRing>,
    /// `(sub, tenant_id)` the artifact upload route is pinned to (the ingest fixture's registered binding), if any.
    pub upload_pin: Option<(String, String)>,
    /// `/_e2e/config` admin channel (seeding, denials, faults). Off unless explicitly enabled.
    pub admin: bool,
    /// Treated lab (sqlite, ED0L shape) per tenant; a tenant without an entry has no lab (`lab_unavailable`).
    pub labs: HashMap<String, std::path::PathBuf>,
    /// Floor of the k-anonymity threshold: rows with fewer cases are never served, whatever the lab file says.
    pub min_k: i64,
    /// Floor of the numerator/complement size: a row whose numerator or complement is below it is withheld (the rate would expose it).
    pub min_cell: i64,
    /// Where a correlated `release.published` schedules its successor run; `None` = a durable queue in the `Store`.
    pub successors: Option<Arc<dyn crate::correlation::SuccessorSink>>,
}

impl Config {
    /// Everything optional off: no upload pin, no admin channel.
    pub fn new(ring: Arc<KeyRing>) -> Config {
        Config { ring, upload_pin: None, admin: false, labs: HashMap::new(), min_k: 10, min_cell: 0, successors: None }
    }
}

#[derive(Default)]
struct Admin {
    deny_operations: HashSet<String>,
    faults: HashMap<String, VecDeque<String>>,
}

pub struct App {
    pub(crate) cfg: Config,
    pub(crate) control: Verifier,
    pub(crate) broker: Verifier,
    pub(crate) store: Arc<dyn Store>,
    admin: Mutex<Admin>,
    pub(crate) write_lock: Mutex<()>,
    pub(crate) now: Box<dyn Fn() -> f64 + Send + Sync>,
}

pub(crate) fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl App {
    pub fn new(cfg: Config, store: Box<dyn Store>) -> App {
        let secs = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        App::with_clock(cfg, store, Box::new(secs))
    }

    pub fn with_clock(cfg: Config, store: Box<dyn Store>, now: Box<dyn Fn() -> f64 + Send + Sync>) -> App {
        let boot = now();
        let store: Arc<dyn Store> = Arc::from(store);
        App {
            control: Verifier::new(cfg.ring.clone()).with_boot_floor(boot).with_store(store.clone(), "control"),
            broker: Verifier::new(cfg.ring.clone()).with_boot_floor(boot).with_store(store.clone(), "broker"),
            cfg,
            store,
            admin: Mutex::new(Admin::default()),
            write_lock: Mutex::new(()),
            now,
        }
    }

    /// A store failure (the port is infallible, so the durable store panics) answers 503 for that request only.
    pub fn handle(&self, r: &Req) -> Resp {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.route(r))).unwrap_or_else(|_| code(503, "store_unavailable"))
    }

    fn route(&self, r: &Req) -> Resp {
        let broker_path = r.path.strip_prefix(BROKER);
        match (r.method.as_str(), r.path.as_str(), broker_path) {
            ("POST", "/internal/v1/core-task-bindings", _) => self.binding(r),
            ("POST", _, Some("/authorizations/check")) => self.authz(r),
            ("POST", _, Some("/artifacts")) => self.artifact_put(r),
            ("GET", _, Some(p)) if p.starts_with("/artifacts/") => self.artifact_get(r, &p["/artifacts/".len()..]),
            ("POST", _, Some("/grants")) => self.grant_issue(r),
            ("POST", _, Some(p)) if p.starts_with("/grants/") && p.ends_with("/revoke") => self.grant_revoke(r, &p["/grants/".len()..p.len() - "/revoke".len()]),
            ("POST", _, Some("/lab/sessions")) => self.lab_open(r),
            ("POST", _, Some(p)) if p.starts_with("/lab/sessions/") && p.ends_with("/queries") => self.lab_query(r, &p["/lab/sessions/".len()..p.len() - "/queries".len()]),
            ("POST", _, Some(p)) if p.starts_with("/lab/sessions/") && p.ends_with("/close") => self.lab_session(r, &p["/lab/sessions/".len()..p.len() - "/close".len()], true),
            ("GET", _, Some(p)) if p.starts_with("/lab/sessions/") => self.lab_session(r, &p["/lab/sessions/".len()..], false),
            ("GET", _, Some(p)) if p.starts_with("/lab/queries/") => self.lab_read(r, "lab_query", &p["/lab/queries/".len()..], "query_not_found"),
            ("GET", _, Some(p)) if p.starts_with("/lab/results/") => self.lab_read(r, "lab_result", &p["/lab/results/".len()..], "result_not_found"),
            ("GET", _, Some(p)) if p.starts_with("/lab/receipts/") => self.lab_read(r, "lab_receipt", &p["/lab/receipts/".len()..], "receipt_not_found"),
            ("POST", _, Some("/wiki/read")) => self.wiki_read(r),
            ("GET", "/healthz", _) => json_resp(200, json!({"status": "ok", "service": "control-api"})),
            ("POST", "/internal/v1/platform/observations", _) => self.observations(r),
            ("POST", "/internal/v1/platform/releases", _) => self.release_event(r),
            ("GET", "/internal/v1/platform/quarantine", _) => self.quarantine_list(r),
            ("GET", p, _) if p.starts_with("/internal/v1/platform/exporters/") => self.cursor_get(r),
            ("POST", "/_e2e/config", _) if self.cfg.admin => self.admin_config(r),
            _ => code(404, "not_found"),
        }
    }

    fn bearer<'a>(&self, r: &'a Req) -> Option<&'a str> {
        let h = r.headers.get("authorization")?;
        let (scheme, token) = h.split_once(' ')?;
        (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
    }

    pub(crate) fn authn(&self, v: &Verifier, r: &Req, want: &Expect) -> Result<Value, Denied> {
        let token = self.bearer(r).ok_or(Denied { reason: "missing_bearer", status: 401 })?;
        v.verify(token, (self.now)(), want)
    }

    pub(crate) fn denied(d: Denied) -> Resp {
        code(d.status, &format!("pulso:auth_{}", d.reason))
    }

    pub(crate) fn fault(&self, route: &str) -> Option<String> {
        self.admin.lock().unwrap_or_else(std::sync::PoisonError::into_inner).faults.get_mut(route).and_then(VecDeque::pop_front)
    }

    pub(crate) fn parse(r: &Req) -> Option<Map<String, Value>> {
        match serde_json::from_slice::<Value>(if r.body.is_empty() { b"{}" } else { &r.body }) {
            Ok(Value::Object(m)) => Some(m),
            _ => None,
        }
    }

    // ---- POST /internal/v1/core-task-bindings -------------------------------------------------------------
    fn binding(&self, r: &Req) -> Resp {
        let claims = match self.authn(&self.control, r, &Expect { aud: "control-api", scope: Some("binding"), purpose: Some("core_task_binding") }) {
            Ok(c) => c,
            Err(d) => return Self::denied(d),
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default().to_string();
        let Some(b) = Self::parse(r) else { return code(422, "schema_invalid") };
        let s = |k: &str| b.get(k).and_then(Value::as_str).map(str::to_string);
        if s("tenant_id").as_deref() != Some(tenant.as_str()) {
            return code(403, "tenant_mismatch");
        }
        let (Some(command_key), Some(request_digest), Some(job_id), Some(core_run_id), Some(task_binding_ref)) =
            (s("command_key"), s("request_digest"), s("job_id"), s("core_run_id"), s("task_binding_ref"))
        else {
            return code(422, "schema_invalid");
        };
        if r.headers.get("idempotency-key").map(String::as_str) != Some(command_key.as_str()) {
            return code(422, "idempotency_key_mismatch");
        }
        if [&command_key, &request_digest, &job_id, &core_run_id, &task_binding_ref].iter().any(|v| v.is_empty() || v.chars().count() > 256 || v.contains('\0')) {
            return code(422, "schema_invalid"); // the durable store bounds every key (and Postgres text cannot hold NUL)
        }
        let mode = self.fault("bind");
        let _w = self.write_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let prior = self.store.binding(&tenant, &command_key);
        if prior.as_ref().is_some_and(|p| p.request_digest != request_digest) {
            return code(409, "digest_mismatch");
        }
        if prior.is_none() && self.store.job_owner(&tenant, &job_id).is_some_and(|o| o != command_key) {
            return code(409, "binding_conflict");
        }
        if mode.as_deref() == Some("503") {
            return code(503, "unavailable"); // before any effect
        }
        if prior.is_none() {
            let attempt = b.get("attempt").and_then(Value::as_i64).unwrap_or(0);
            let digest = request_digest.clone();
            if !self.store.put_binding(&tenant, &command_key, BindingRec { request_digest, job_id: job_id.clone(), core_run_id, attempt, task_binding_ref }) {
                // lost a race (another process sharing the database): the winner's row decides, never a silent overwrite
                return match self.store.binding(&tenant, &command_key) {
                    Some(w) if w.request_digest == digest => json_resp(200, json!({"schema_version": "1", "state": "confirmed", "tenant_id": tenant, "job_id": job_id})),
                    Some(_) => code(409, "digest_mismatch"),
                    None => code(409, "binding_conflict"), // the job id belongs to another command key or the ref is another tenant's
                };
            }
        }
        if mode.as_deref() == Some("applied_then_503") {
            return code(503, "unavailable"); // effect committed, answer lost
        }
        json_resp(200, json!({"schema_version": "1", "state": "confirmed", "tenant_id": tenant, "job_id": job_id}))
    }

    // ---- POST /internal/v1/broker/authorizations/check ------------------------------------------------------
    fn authz(&self, r: &Req) -> Resp {
        let claims = match self.authn(&self.broker, r, &Expect { aud: "lab-broker", scope: Some("authorization_check"), purpose: None }) {
            Ok(c) => c,
            Err(d) => return Self::denied(d),
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default();
        let body = Self::parse(r).unwrap_or_default();
        let operation = body.get("operation").and_then(Value::as_str).unwrap_or_default();
        let owner = body.get("binding_ref").and_then(Value::as_str).and_then(|b| self.store.binding_ref_tenant(b));
        let reason = if owner.as_deref() != Some(tenant) {
            Some("binding_unknown") // unknown or another tenant's binding: never allowed
        } else if self.admin.lock().unwrap_or_else(std::sync::PoisonError::into_inner).deny_operations.contains(operation) {
            Some("revoked")
        } else {
            None
        };
        let until = z_timestamp((self.now)() as i64 + 60);
        json_resp(200, json!({"allowed": reason.is_none(), "authorization_revision": 1, "valid_until": until, "reason_code": reason}))
    }

    // ---- GET /internal/v1/broker/artifacts/{id} -----------------------------------------------------------------
    fn artifact_get(&self, r: &Req, id: &str) -> Resp {
        let claims = match self.authn(&self.broker, r, &Expect { aud: "lab-broker", scope: Some("artifact_read"), purpose: None }) {
            Ok(c) => c,
            Err(d) => return Self::denied(d),
        };
        match self.store.get_artifact(claims["tenant_id"].as_str().unwrap_or_default(), id) {
            Some(env) => json_resp(200, env),
            None => code(404, "artifact_not_found"),
        }
    }

    // ---- POST /internal/v1/broker/artifacts (idempotent by digest) -------------------------------------------
    fn artifact_put(&self, r: &Req) -> Resp {
        let claims = match self.authn(&self.broker, r, &Expect { aud: "lab-broker", scope: Some("artifact_write"), purpose: Some("artifact_upload") }) {
            Ok(c) => c,
            Err(d) => return error(d.status, d.reason),
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default().to_string();
        if let Some((sub, t)) = &self.cfg.upload_pin
            && (claims["sub"].as_str() != Some(sub) || tenant != *t)
        {
            return error(403, "binding_or_tenant_denied");
        }
        if r.body.len() > MAX_ARTIFACT_BYTES + 8192 {
            return error(413, "artifact_too_large");
        }
        let Some(up) = Self::parse(r).and_then(Self::validate_upload) else { return error(422, "schema_invalid") };
        if up["content"].to_string().contains("\\u0000") {
            return error(422, "schema_invalid"); // Postgres JSONB cannot hold U+0000 (every store answers alike)
        }
        if up["source_schema_ref"].is_null() && up["artifact_kind"] != "schema" {
            return error(422, "source_schema_ref_required");
        }
        if !up["source_schema_ref"].is_null() {
            let rf = &up["source_schema_ref"];
            let found = self.store.get_artifact(&tenant, rf["id"].as_str().unwrap_or_default());
            if found.is_none_or(|e| e["artifact"] != *rf) {
                return error(422, "unresolvable_artifact_ref");
            }
        }
        let digest = up["content_digest"].as_str().unwrap_or_default().to_string();
        let (actual, byte_length) = if up["encoding"] == "utf8" {
            let Some(text) = up["content"].as_str() else { return error(422, "schema_invalid") };
            if text.len() > MAX_ARTIFACT_BYTES {
                return error(413, "artifact_too_large");
            }
            (sha256_hex(text.as_bytes()), text.len())
        } else {
            let Ok(c) = jcs(&up["content"]) else { return error(422, "schema_invalid") };
            (sha256_hex(c.as_bytes()), c.len())
        };
        if actual != digest || r.headers.get("idempotency-key").map(String::as_str) != Some(digest.as_str()) || !is_hex64(&digest) {
            return error(422, "digest_mismatch");
        }
        let rf = json!({"id": format!("artifact:{digest}"), "digest": digest, "media_type": up["media_type"]});
        let envelope = json!({"schema_version": "1", "artifact": rf, "encoding": up["encoding"], "content": up["content"], "byte_length": byte_length});
        let _w = self.write_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let outcome = self.store.put_artifact(&tenant, envelope);
        if outcome == PutOutcome::Conflict {
            return error(409, "digest_conflict");
        }
        let mut receipt = rf.clone();
        receipt["id"] = json!(format!("receipt:{digest}"));
        json_resp(if outcome == PutOutcome::Created { 201 } else { 200 }, json!({"artifact_ref": rf, "receipt_ref": receipt}))
    }

    /// Strict upload model (no extra keys, closed enums, `content` and `source_schema_ref` required).
    fn validate_upload(m: Map<String, Value>) -> Option<Value> {
        const KEYS: [&str; 10] = ["schema_version", "binding_ref", "source_schema_ref", "classification", "information_partition", "artifact_kind", "media_type", "encoding", "content", "content_digest"];
        if m.len() != KEYS.len() || KEYS.iter().any(|k| !m.contains_key(*k)) {
            return None;
        }
        let one_of = |k: &str, set: &[&str]| m[k].as_str().is_some_and(|v| set.contains(&v));
        let is_str = |k: &str| m[k].is_string();
        let ref_ok = match &m["source_schema_ref"] {
            Value::Null => true,
            Value::Object(o) => o.len() == 3 && ["id", "digest", "media_type"].iter().all(|k| o.get(*k).is_some_and(Value::is_string)),
            _ => false,
        };
        (m["schema_version"] == "1"
            && is_str("binding_ref")
            && is_str("information_partition")
            && is_str("content_digest")
            && ref_ok
            && one_of("classification", &["treated", "restricted_original"])
            && one_of("artifact_kind", &["schema", "source_material", "layer_mapping", "cut", "other"])
            && one_of("media_type", &["application/json", "application/x-ndjson", "text/plain"])
            && one_of("encoding", &["json", "utf8"]))
        .then(|| Value::Object(m))
    }

    // ---- POST /_e2e/config (driver/test channel, off by default) -------------------------------------------------
    fn admin_config(&self, r: &Req) -> Resp {
        let Some(cfg) = Self::parse(r) else { return code(422, "schema_invalid") };
        let mut a = self.admin.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(ops) = cfg.get("deny_operations").and_then(Value::as_array) {
            a.deny_operations = ops.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
        }
        if let Some(f) = cfg.get("faults").and_then(Value::as_object) {
            for (route, modes) in f {
                a.faults.insert(route.clone(), modes.as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect());
            }
        }
        drop(a);
        for item in cfg.get("artifacts").and_then(Value::as_array).into_iter().flatten() {
            let (Some(tenant), Some(id)) = (item["tenant"].as_str(), item["id"].as_str()) else { return code(422, "schema_invalid") };
            let canon = jcs(&item["content"]).unwrap_or_default();
            let env = json!({"schema_version": "1", "artifact": {"id": id, "digest": format!("sha256:{}", sha256_hex(canon.as_bytes())), "media_type": "application/json"},
                             "encoding": "json", "content": item["content"], "byte_length": canon.len()});
            let _ = self.store.put_artifact(tenant, env);
        }
        for item in cfg.get("run_events").and_then(Value::as_array).into_iter().flatten() {
            if let Err(e) = self.seed_run_events(item) {
                return e;
            }
        }
        for item in cfg.get("wiki").and_then(Value::as_array).into_iter().flatten() {
            let (Some(tenant), Some(path), Some(text)) = (item["tenant"].as_str(), item["path"].as_str(), item["content"].as_str()) else { return code(422, "schema_invalid") };
            self.store.put_doc("wiki", tenant, path, json!(text));
        }
        for item in cfg.get("preauthorized_bindings").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(b), Some(t)) = (item["binding_ref"].as_str(), item["tenant"].as_str()) {
                self.store.preauthorize_binding_ref(b, t);
            }
        }
        for item in cfg.get("published_releases").and_then(Value::as_array).into_iter().flatten() {
            let f = |k: &str| item[k].as_str().map(str::to_string);
            let (Some(tenant), Some(release_id), Some(agent_id), Some(alias), Some(candidate_hash)) = (f("tenant"), f("release_id"), f("agent_id"), f("alias"), f("candidate_hash")) else { return code(422, "schema_invalid") };
            crate::correlation::record_published(&*self.store, &tenant, &crate::correlation::PublishedRelease { release_id, agent_id, alias, candidate_hash });
        }
        json_resp(200, json!({"ok": true}))
    }
}
