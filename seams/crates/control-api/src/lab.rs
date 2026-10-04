//! Lab and wiki broker (E5R): revocable grants, k-anonymous aggregate lab queries with a `QueryReceipt`, wiki read.
//!
//! * A **grant** is issued by the control plane for one `(tenant, binding_ref, scope)` with a bounded TTL and is revocable.
//!   It is re-validated on EVERY lab call (open, query, state, result, receipt): an open session never outlives its grant.
//!   A grant of another tenant looks unknown (`grant_denied`), never "exists elsewhere".
//! * The lab is a sqlite file in the ED0L shape (`e2e-core/src/claude_standin/ed0_lab.py`), opened read-only, one per
//!   tenant (`Config::labs`). The broker serves **aggregates only**: a fixed parameterised SELECT over `lab_rows` (raw SQL is
//!   refused, `sql` in a body is `raw_sql_refused`), the numerator never leaves, and rows with fewer than `max(k of the lab,
//!   Config::min_k)` cases are withheld even if the file contains them (defence in depth against a tampered lab).
//! * Wiki read is tenant-scoped by the service JWT (no grant): pages are seeded through the admin channel until the wiki
//!   store exists; paths are confined (no `..`, absolute, backslash or empty segments).
//! Error bodies are `{"code": ...}` like the other broker routes.
use crate::app::{App, Req, Resp, code, json_resp};
use crate::auth::Expect;
use core_client::canon::{sha256_hex, z_timestamp};
use serde_json::{Value, json};
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_GRANT_TTL_S: i64 = 3600;
const MAX_ROWS: usize = 200;
const MAX_WIKI_PATHS: usize = 50;
/// The only statement the broker ever runs on a lab; parameters are bound, never interpolated.
pub const LAB_SQL: &str = "select evidence_ref, metric_id, window_id, g_group, numerator, count from lab_rows where metric_id = ? and window_id = ? order by evidence_ref";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// An unguessable reference: 128 bits from the process-random SipHash keys plus a counter.
fn fresh(prefix: &str) -> String {
    let rs = std::collections::hash_map::RandomState::new();
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    format!("{prefix}{:016x}{:016x}", rs.hash_one((n, 1u8)), rs.hash_one((n, 2u8)))
}

fn plain_id(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

fn path_ok(p: &str) -> bool {
    !p.is_empty() && p.len() <= 256 && !p.starts_with('/') && !p.contains(['\\', '\0', ':']) && p.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// Rate of `num / cnt` rounded half-even to 2 decimals with exact integer arithmetic (the Python lab's `rate_of`).
pub fn rate(num: i64, cnt: i64) -> f64 {
    let (num, cnt) = (num as i128, cnt as i128); // a tampered lab may hold counts whose x100 overflows i64
    let (q, rem) = (num * 100 / cnt, num * 100 % cnt);
    let q = if rem * 2 > cnt || (rem * 2 == cnt && q % 2 == 1) { q + 1 } else { q };
    q as f64 / 100.0
}

impl App {
    fn broker_claims(&self, r: &Req, scope: &str) -> Result<Value, Resp> {
        self.authn(&self.broker, r, &Expect { aud: "lab-broker", scope: Some(scope), purpose: None }).map_err(Self::denied)
    }

    /// Liveness of a grant for one use; the order of the checks is the order of the error codes.
    fn check_grant(&self, tenant: &str, grant_ref: &str, scope: &str, binding_ref: Option<&str>) -> Result<(), Resp> {
        let Some(g) = self.store.get_doc("grant", tenant, grant_ref) else { return Err(code(403, "grant_denied")) };
        if g["revoked"] == true {
            return Err(code(403, "grant_revoked"));
        }
        if (self.now)() >= g["expires_at_epoch"].as_f64().unwrap_or(0.0) {
            return Err(code(403, "grant_expired"));
        }
        if g["scope"] != scope {
            return Err(code(403, "grant_scope_mismatch"));
        }
        if binding_ref.is_some_and(|b| g["binding_ref"] != b) {
            return Err(code(403, "grant_binding_mismatch"));
        }
        Ok(())
    }

    // ---- POST /grants ----------------------------------------------------------------------------------------------
    pub(crate) fn grant_issue(&self, r: &Req) -> Resp {
        let claims = match self.broker_claims(r, "grant_issue") {
            Ok(c) => c,
            Err(e) => return e,
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default();
        let Some(b) = Self::parse(r) else { return code(422, "schema_invalid") };
        let (Some(binding), Some(scope), Some(ttl)) = (b.get("binding_ref").and_then(Value::as_str), b.get("scope").and_then(Value::as_str), b.get("ttl_seconds").and_then(Value::as_i64)) else {
            return code(422, "schema_invalid");
        };
        if !["lab", "wiki"].contains(&scope) || !(1..=MAX_GRANT_TTL_S).contains(&ttl) || b.len() != 3 {
            return code(422, "schema_invalid");
        }
        if self.store.binding_ref_tenant(binding).as_deref() != Some(tenant) {
            return code(403, "binding_unknown"); // unknown or another tenant's binding
        }
        let now = (self.now)();
        let grant_ref = fresh("grant:");
        let doc = json!({"grant_ref": grant_ref, "binding_ref": binding, "scope": scope, "issued_at_epoch": now, "expires_at_epoch": now + ttl as f64, "revoked": false});
        let _w = self.write_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.store.put_doc("grant", tenant, &grant_ref, doc);
        json_resp(201, json!({"grant_ref": grant_ref, "tenant_id": tenant, "binding_ref": binding, "scope": scope, "expires_at": z_timestamp((now + ttl as f64) as i64)}))
    }

    // ---- POST /grants/{ref}/revoke ---------------------------------------------------------------------------------
    pub(crate) fn grant_revoke(&self, r: &Req, grant_ref: &str) -> Resp {
        let claims = match self.broker_claims(r, "grant_issue") {
            Ok(c) => c,
            Err(e) => return e,
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default();
        let _w = self.write_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(mut g) = self.store.get_doc("grant", tenant, grant_ref) else { return code(404, "grant_not_found") };
        g["revoked"] = json!(true);
        self.store.put_doc("grant", tenant, grant_ref, g);
        json_resp(200, json!({"grant_ref": grant_ref, "state": "revoked"}))
    }

    fn lab_db(&self, tenant: &str) -> Result<rusqlite::Connection, Resp> {
        let path = self.cfg.labs.get(tenant).ok_or_else(|| code(404, "lab_unavailable"))?;
        let con = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|_| code(503, "lab_unavailable"))?;
        con.pragma_update(None, "query_only", true).map_err(|_| code(503, "lab_unavailable"))?;
        Ok(con)
    }

    fn meta(con: &rusqlite::Connection, key: &str) -> i64 {
        con.query_row("select value from lab_meta where key = ?", [key], |r| r.get::<_, String>(0)).ok().and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    fn lab_k(&self, con: &rusqlite::Connection) -> i64 {
        Self::meta(con, "k").max(self.cfg.min_k).max(1)
    }

    /// The numerator/complement floor: the larger of the lab's own `min_cell` and the broker's.
    fn lab_min_cell(&self, con: &rusqlite::Connection) -> i64 {
        Self::meta(con, "min_cell").max(self.cfg.min_cell).max(0)
    }

    // ---- POST /lab/sessions ----------------------------------------------------------------------------------------
    pub(crate) fn lab_open(&self, r: &Req) -> Resp {
        let claims = match self.broker_claims(r, "lab") {
            Ok(c) => c,
            Err(e) => return e,
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default();
        let Some(grant) = r.headers.get("x-grant-ref") else { return code(403, "grant_required") };
        let Some(binding) = Self::parse(r).and_then(|b| b.get("binding_ref").and_then(Value::as_str).map(str::to_string)) else { return code(422, "schema_invalid") };
        if let Err(e) = self.check_grant(tenant, grant, "lab", Some(&binding)) {
            return e;
        }
        let con = match self.lab_db(tenant) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let k = self.lab_k(&con);
        let rows: i64 = con.query_row("select count(*) from lab_rows", [], |r| r.get(0)).unwrap_or(0);
        let sid = fresh("sess-");
        let _w = self.write_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.store.put_doc("lab_session", tenant, &sid, json!({"grant_ref": grant, "binding_ref": binding, "closed": false}));
        json_resp(200, json!({"session_ref": sid, "revision": 1, "manifest_digest": sha256_hex(format!("lab|k={k}|rows={rows}").as_bytes()),
                              "limits": {"max_rows": MAX_ROWS, "k": k}, "table_catalog": []}))
    }

    fn session(&self, tenant: &str, sid: &str) -> Result<Value, Resp> {
        let s = self.store.get_doc("lab_session", tenant, sid).ok_or_else(|| code(404, "session_not_found"))?;
        self.check_grant(tenant, s["grant_ref"].as_str().unwrap_or_default(), "lab", s["binding_ref"].as_str())?;
        Ok(s)
    }

    // ---- GET /lab/sessions/{sid} and POST /lab/sessions/{sid}/close ----------------------------------------------------
    pub(crate) fn lab_session(&self, r: &Req, sid: &str, close: bool) -> Resp {
        let claims = match self.broker_claims(r, "lab") {
            Ok(c) => c,
            Err(e) => return e,
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default();
        let mut s = match self.session(tenant, sid) {
            Ok(s) => s,
            Err(e) => return e,
        };
        if close {
            let _w = self.write_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            s["closed"] = json!(true);
            self.store.put_doc("lab_session", tenant, sid, s.clone());
        }
        json_resp(200, json!({"state": if s["closed"] == true { "closed" } else { "open" }, "revision": 1}))
    }

    // ---- POST /lab/sessions/{sid}/queries ----------------------------------------------------------------------------
    pub(crate) fn lab_query(&self, r: &Req, sid: &str) -> Resp {
        let claims = match self.broker_claims(r, "lab") {
            Ok(c) => c,
            Err(e) => return e,
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default();
        let session = match self.session(tenant, sid) {
            Ok(s) => s,
            Err(e) => return e,
        };
        if session["closed"] == true {
            return code(409, "session_closed");
        }
        let Some(b) = Self::parse(r) else { return code(422, "schema_invalid") };
        if b.contains_key("sql") {
            return code(422, "raw_sql_refused");
        }
        let get = |k: &str| b.get(k).and_then(Value::as_str).filter(|s| plain_id(s));
        let (Some(key), Some(metric), Some(window)) = (get("query_key"), get("metric_id"), get("window_id")) else { return code(422, "schema_invalid") };
        if b.keys().any(|k| !["query_key", "metric_id", "window_id", "run_ref"].contains(&k.as_str())) || b.get("run_ref").is_some_and(|v| !v.is_string()) {
            return code(422, "schema_invalid");
        }
        let params = json!({"metric_id": metric, "window_id": window});
        let qref = format!("q-{key}");
        let reply = |status| json_resp(status, json!({"query_ref": qref, "status_url": format!("/lab/queries/{qref}")}));
        let _w = self.write_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(prior) = self.store.get_doc("lab_query", tenant, key) {
            return if prior["params"] == params { reply(202) } else { code(409, "query_key_conflict") };
        }
        let con = match self.lab_db(tenant) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let k = self.lab_k(&con);
        let min_cell = self.lab_min_cell(&con);
        let started = std::time::Instant::now();
        let fetched: Result<Vec<(String, String, String, String, i64, i64)>, rusqlite::Error> = con.prepare(LAB_SQL).and_then(|mut st| {
            st.query_map([metric, window], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))?.collect()
        });
        let Ok(all) = fetched else { return code(503, "lab_unavailable") };
        let (mut rows, mut withheld, mut digest_src) = (Vec::new(), 0usize, String::new());
        for (ev, m, w, g, num, cnt) in all {
            if cnt < k || num < 0 || num > cnt || num < min_cell || cnt - num < min_cell {
                withheld += 1;
                continue;
            }
            let rt = rate(num, cnt);
            digest_src.push_str(&format!("{ev}|{cnt}|{}\n", (rt * 100.0).round() as i64));
            rows.push(json!([m, w, g, cnt, rt, ev]));
        }
        let total = rows.len();
        let truncated = total > MAX_ROWS;
        rows.truncate(MAX_ROWS);
        let id = sha256_hex(format!("{tenant}|{key}").as_bytes());
        let (result_ref, receipt_ref) = (format!("res-{}", &id[..16]), format!("rcpt-{}", &id[..16]));
        let result_digest = sha256_hex(digest_src.as_bytes());
        let findings = if withheld > 0 { vec![format!("rows_below_k_withheld:{withheld}")] } else { vec![] };
        let bytes = serde_json::to_vec(&rows).map(|v| v.len()).unwrap_or(0);
        let own = json!({"grant_ref": session["grant_ref"], "binding_ref": session["binding_ref"]});
        let merge = |mut v: Value| {
            v["grant_ref"] = own["grant_ref"].clone();
            v["binding_ref"] = own["binding_ref"].clone();
            v
        };
        let columns = json!([{"name": "metric_id", "type": "text", "data_class": "public"}, {"name": "window_id", "type": "text", "data_class": "public"},
                             {"name": "g_group", "type": "text", "data_class": "public"}, {"name": "count", "type": "int", "data_class": "public"},
                             {"name": "rate", "type": "float", "data_class": "public"}, {"name": "evidence_ref", "type": "text", "data_class": "public"}]);
        self.store.put_doc("lab_result", tenant, &result_ref, merge(json!({"columns": columns, "rows": rows, "truncated": truncated, "next_cursor": null,
                                                                           "total_rows": total, "receipt_ref": receipt_ref, "result_digest": result_digest})));
        self.store.put_doc("lab_receipt", tenant, &receipt_ref, merge(json!({
            "query_id": qref, "run_ref": b.get("run_ref").cloned().unwrap_or(json!("")), "sql_sanitized": LAB_SQL, "sql_digest": sha256_hex(LAB_SQL.as_bytes()),
            "param_classes": ["metric_id", "window_id"], "rows": rows.len(), "bytes": bytes, "duration_ms": started.elapsed().as_secs_f64() * 1000.0,
            "truncated": truncated, "outcome": "ok", "quality_findings": findings, "result_digest": result_digest, "k": k})));
        self.store.put_doc("lab_query", tenant, key, merge(json!({"params": params, "result_ref": result_ref, "receipt_ref": receipt_ref, "state": "completed"})));
        reply(202)
    }

    /// GET of a stored lab document (query state, result, receipt): tenant-scoped, grant re-validated, internals stripped.
    pub(crate) fn lab_read(&self, r: &Req, ns: &str, id: &str, missing: &str) -> Resp {
        let claims = match self.broker_claims(r, "lab") {
            Ok(c) => c,
            Err(e) => return e,
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default();
        let key = if ns == "lab_query" { id.strip_prefix("q-").unwrap_or(id) } else { id };
        let Some(mut doc) = self.store.get_doc(ns, tenant, key) else { return code(404, missing) };
        if let Err(e) = self.check_grant(tenant, doc["grant_ref"].as_str().unwrap_or_default(), "lab", doc["binding_ref"].as_str()) {
            return e;
        }
        let obj = doc.as_object_mut().expect("doc is an object");
        obj.remove("binding_ref");
        if ns == "lab_query" {
            obj.remove("params");
            obj.remove("grant_ref");
            obj.insert("reason_code".into(), Value::Null);
        } else if ns == "lab_result" {
            obj.remove("grant_ref");
        } // the receipt keeps its grant_ref: it is the audit trail
        json_resp(200, doc)
    }

    // ---- POST /wiki/read -----------------------------------------------------------------------------------------------
    pub(crate) fn wiki_read(&self, r: &Req) -> Resp {
        let claims = match self.broker_claims(r, "wiki") {
            Ok(c) => c,
            Err(e) => return e,
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default();
        let Some(paths) = Self::parse(r).and_then(|b| b.get("paths").and_then(Value::as_array).cloned()).filter(|p| !p.is_empty() && p.len() <= MAX_WIKI_PATHS) else { return code(422, "schema_invalid") };
        let Some(paths) = paths.iter().map(|p| p.as_str()).collect::<Option<Vec<&str>>>() else { return code(422, "schema_invalid") };
        if paths.iter().any(|p| !path_ok(p)) {
            return code(422, "path_invalid");
        }
        let mut entries = Vec::new();
        for p in paths {
            let Some(text) = self.store.get_doc("wiki", tenant, p).and_then(|v| v.as_str().map(str::to_string)) else { return code(404, "page_not_found") };
            entries.push(json!({"path": p, "content": text, "digest": sha256_hex(text.as_bytes()), "evidence_refs": []}));
        }
        json_resp(200, json!({"entries": entries, "base_digest": sha256_hex(format!("base:{tenant}").as_bytes())}))
    }
}
