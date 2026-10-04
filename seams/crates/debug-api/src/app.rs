//! `App::handle`: a pure request -> response function over the store (testable without sockets); `server` is the tiny_http glue.
//! Routes mirror `debug-console/fixture-server/server.mjs` (the executable contract) plus the spec 25 panel routes the
//! typed `DebugApi` provider asks for. Read-only: no command is ever executed (`available_commands[]` is what the data says).
use crate::event::{NewEvent, RunEventSink};
use crate::store::{Store, now_iso};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const PREFIX: &str = "/internal/v1/debug";
const EVENT_PAGE: usize = 1000;
static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct Config {
    pub tenant: String,
    /// Static local-dev bearer token for the debug routes; `None` = open (the bin only binds loopback).
    pub token: Option<String>,
    /// Admin (append/purge/ingest) token; `None` = the admin surface does not exist (404).
    pub admin_token: Option<String>,
    pub heartbeat: Duration,
    /// Directory of a built console (`index.html`, assets) served for non-API GETs; `None` = no static surface.
    pub static_dir: Option<std::path::PathBuf>,
    /// Body served for `/config.json` instead of the file in `static_dir` (points the console's data provider at this server).
    pub config_json: Option<String>,
}

impl Default for Config {
    fn default() -> Config {
        Config { tenant: "tenant-local".into(), token: None, admin_token: None, heartbeat: Duration::from_secs(5), static_dir: None, config_json: None }
    }
}

pub struct Req {
    pub method: String,
    pub path: String,
    pub query: String,
    /// Lower-cased names.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct StreamPlan {
    pub run: String,
    pub after: i64,
}

pub struct App {
    pub(crate) store: Arc<Store>,
    pub(crate) cfg: Config,
    csrf: String,
    cookie: String,
}

fn opaque(tag: &str) -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let h = Sha256::digest(format!("{tag}:{nanos}:{}:{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
    h.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn resp(status: u16, body: &Value, headers: Vec<(String, String)>) -> Resp {
    let mut h = vec![("Content-Type".to_string(), "application/json".to_string()), ("Cache-Control".to_string(), "no-store".to_string())];
    h.extend(headers);
    Resp { status, headers: h, body: body.to_string().into_bytes() }
}
fn ok(body: &Value) -> Resp {
    resp(200, body, vec![])
}
fn problem(code: &str, status: u16, extra: Value) -> Resp {
    let mut b = json!({"code": code, "message": code, "correlation_id": opaque("corr"), "retryable": false, "current_ref": null, "blocking_refs": [], "conflict": null});
    if let (Some(m), Some(e)) = (b.as_object_mut(), extra.as_object()) {
        m.extend(e.clone());
    }
    resp(status, &b, vec![])
}
fn field_error(field: &str, code: &str) -> Resp {
    problem("validation_error", 422, json!({"field_errors": [{"field": field, "code": code}]}))
}

fn query_param<'a>(q: &'a str, key: &str) -> Option<&'a str> {
    q.split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

fn bearer_ok(r: &Req, expected: &str) -> bool {
    let got = r.headers.get("authorization").and_then(|v| v.strip_prefix("Bearer ")).unwrap_or_default();
    // fixed-time compare over equal-length digests
    Sha256::digest(got) == Sha256::digest(expected)
}

impl App {
    pub fn new(store: Arc<Store>, cfg: Config) -> App {
        let cookie = format!("pulso_local_session={}; HttpOnly; SameSite=Lax; Path=/", opaque("session"));
        App { store, cfg, csrf: opaque("csrf"), cookie }
    }
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }
    pub fn heartbeat(&self) -> Duration {
        self.cfg.heartbeat
    }

    fn env(&self, entity_ref: Value, rev: i64, status: &str, extra: Value) -> Value {
        let mut b = json!({
            "schema_version": "1", "tenant_id": self.cfg.tenant, "entity_ref": entity_ref, "projection_revision": rev, "as_of": now_iso(),
            "environment": "sandbox", "source_kind": "engine_event", "validation": "unverified", "status": status, "blocking_reasons": [],
            "next_automatic_action": null, "available_commands": [],
            "links": [], "coverage": {"status": "complete", "observed_count": null, "expected_count": null, "reason_code": null},
        });
        if let (Some(m), Some(e)) = (b.as_object_mut(), extra.as_object()) {
            m.extend(e.clone());
        }
        b
    }

    fn states(&self) -> Vec<(String, Value)> {
        self.store.runs().into_iter().filter_map(|r| self.store.state(&r).map(|s| (r, s))).collect()
    }

    pub fn handle(&self, r: &Req) -> Resp {
        let (p, m) = (r.path.as_str(), r.method.as_str());
        if p == "/healthz" {
            return ok(&json!({"ok": true}));
        }
        if let Some(tail) = p.strip_prefix("/__admin/v1/") {
            return self.admin(r, tail);
        }
        if p.starts_with(PREFIX) || p.starts_with("/api/v1/auth/") {
            if let Some(t) = &self.cfg.token {
                if !bearer_ok(r, t) {
                    return problem("unauthorized", 401, json!({}));
                }
            }
        }
        match (m, p) {
            ("GET", "/api/v1/auth/session") => return self.session(),
            ("POST", "/api/v1/auth/step-up") => {
                return if r.headers.get("x-csrf-token") != Some(&self.csrf) { problem("csrf_failed", 403, json!({})) } else { problem("step_up_unavailable", 403, json!({})) };
            }
            _ => {}
        }
        if m == "GET" && !p.starts_with(PREFIX) && !p.starts_with("/api/") {
            if let Some(r) = self.static_file(p) {
                return r;
            }
        }
        let Some(rest) = p.strip_prefix(PREFIX).and_then(|s| s.strip_prefix('/')) else { return problem("not_found", 404, json!({})) };
        let parts: Vec<&str> = rest.split('/').collect();
        match (m, parts.as_slice()) {
            ("GET", ["profile"]) => self.profile(),
            ("GET", ["runs"]) => self.runs(),
            ("GET", ["runs", id, tail]) => self.run_route(id, tail, r),
            ("GET", ["proposals", id, "diff"]) => self.diff(id),
            ("GET", ["memory"]) => self.memory(),
            ("GET", ["decisions", id]) => self.decision(id),
            ("GET", ["health", "dependencies"]) => ok(&self.env(Value::Null, 0, "ok", json!({"items": [], "next_cursor": null}))),
            ("POST", ["decisions", id, "responses"]) => self.respond(id, r),
            _ => problem("not_found", 404, json!({})),
        }
    }

    /// Static console. `None` = not configured (the caller answers 404). Any path that could leave the directory is a 404.
    fn static_file(&self, path: &str) -> Option<Resp> {
        let dir = self.cfg.static_dir.as_ref()?;
        let not_found = || problem("not_found", 404, json!({}));
        let file = |ctype: &str, body: Vec<u8>| Resp { status: 200, headers: vec![("Content-Type".into(), ctype.into()), ("Cache-Control".into(), "no-store".into())], body };
        if path == "/config.json" {
            if let Some(c) = &self.cfg.config_json {
                return Some(file("application/json", c.clone().into_bytes()));
            }
        }
        let rel = path.trim_start_matches('/');
        if rel.contains("..") || rel.bytes().any(|b| matches!(b, 92 | b'%' | b':' | 0)) {
            return Some(not_found());
        }
        let wanted = if rel.is_empty() { "index.html" } else { rel };
        let target = dir.join(wanted);
        let (target, spa) = if target.is_file() {
            (target, false)
        } else if wanted.rsplit('/').next().is_some_and(|n| n.contains('.')) {
            return Some(not_found());
        } else {
            (dir.join("index.html"), true)
        };
        // A symlink (or junction) inside the directory must not lead outside it.
        match (target.canonicalize(), dir.canonicalize()) {
            (Ok(t), Ok(d)) if t.starts_with(&d) => {}
            _ => return Some(not_found()),
        }
        let Ok(body) = std::fs::read(&target) else { return Some(not_found()) };
        let ext = if spa { "html" } else { target.extension().and_then(|e| e.to_str()).unwrap_or("") };
        let ctype = match ext {
            "html" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" | "map" => "application/json",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "ico" => "image/x-icon",
            "woff2" => "font/woff2",
            "txt" => "text/plain; charset=utf-8",
            _ => "application/octet-stream",
        };
        Some(file(ctype, body))
    }

    fn session(&self) -> Resp {
        let b = json!({
            "principal": "local-dev", "tenant_id": self.cfg.tenant, "scopes": ["debug/read"], "expires_at": "2099-01-01T00:00:00Z",
            "csrf_token": self.csrf, "auth": {"simulated": true, "level": "basic", "auth_at": now_iso()},
        });
        resp(200, &b, vec![("Set-Cookie".into(), self.cookie.clone())])
    }

    fn profile(&self) -> Resp {
        let mut doubles: Vec<Value> = Vec::new();
        for (_, s) in self.states() {
            for d in s["doubles"].as_array().into_iter().flatten() {
                if !doubles.iter().any(|x| x["id"] == d["id"]) {
                    doubles.push(d.clone());
                }
            }
        }
        let mut b = json!({
            "target": "debug-api", "runtime_profile": "rust_run_event_store",
            "doubles": doubles.iter().map(|d| d["id"].clone()).collect::<Vec<_>>(), "pin": null,
        });
        if !doubles.is_empty() {
            b["mode"] = json!("stand_in");
            b["doubles_detail"] = Value::Array(doubles);
        }
        ok(&b)
    }

    fn runs(&self) -> Resp {
        let mut max = 0;
        let items: Vec<Value> = self
            .states()
            .into_iter()
            .map(|(id, s)| {
                let head = self.store.head(&id).unwrap_or(0);
                max = max.max(head);
                json!({"run_id": id, "title": s["run"]["title"], "state": s["run"]["state"], "origin": s["run"]["origin"], "projection_revision": head})
            })
            .collect();
        ok(&self.env(Value::Null, max, "ok", json!({"items": items, "next_cursor": null})))
    }

    fn run_route(&self, id: &str, tail: &str, r: &Req) -> Resp {
        let Some(state) = self.store.state(id) else { return problem("not_found", 404, json!({})) };
        let head = self.store.head(id).unwrap_or(0);
        let rf = json!({"kind": "run", "id": id});
        let page = |items: Value| ok(&self.env(rf.clone(), head, "ok", json!({"items": items, "next_cursor": null})));
        match tail {
            "graph" => ok(&self.env(rf, head, state["run"]["state"].as_str().unwrap_or("unknown"), json!({"nodes": state["nodes"]}))),
            "events" => match self.cursor_check(id, query_param(&r.query, "after_sequence").map_or(Ok(0), |s| s.parse::<i64>().ok().filter(|n| *n >= 0).ok_or(()))) {
                Err(resp) => resp,
                Ok(after) => ok(&json!({"items": self.store.events_after(id, after, EVENT_PAGE), "next_cursor": null})),
            },
            "investigation" => {
                let inv = if state["investigation"].is_object() { state["investigation"].clone() } else { json!({"hypothesis": null, "verifier": "unknown", "evidence": []}) };
                ok(&self.env(rf, head, "ok", inv))
            }
            "gates" => {
                let mut g = if state["gates"].is_object() {
                    state["gates"].clone()
                } else {
                    json!({
                        "native": {"status": "not_evaluable", "reason_code": "no_evaluation_in_run", "checked_at": null},
                        "improvement": {"status": "not_evaluable", "reason_code": "no_evaluation_in_run", "receipt_refs": [], "checked_at": null},
                        "combined": {"decision": "not_applicable", "reason_code": "no_evaluation_in_run"},
                    })
                };
                if g["proposal_id"].is_null() {
                    g["proposal_id"] = state["diff"]["proposal_id"].clone();
                }
                if !g["attempts"].is_array() {
                    g["attempts"] = json!([]);
                }
                ok(&self.env(rf, head, "ok", g))
            }
            "alternatives" => page(state["alternatives"].clone()),
            "model-calls" | "queries" | "evals" | "external-commands" => page(json!([])),
            _ => problem("not_found", 404, json!({})),
        }
    }

    /// 410 when the cursor is below the purge floor or beyond the head (another history, e.g. a restarted server).
    fn cursor_check(&self, run: &str, after: Result<i64, ()>) -> Result<i64, Resp> {
        let after = after.map_err(|()| problem("bad_cursor", 400, json!({})))?;
        let (floor, head) = (self.store.floor(run).unwrap_or(0), self.store.head(run).unwrap_or(0));
        if after < floor || after > head {
            let snapshot = format!("{PREFIX}/runs/{run}/graph");
            return Err(problem("cursor_expired", 410, json!({
                "current_ref": {"kind": "run", "id": run}, "recovery_after_sequence": head, "snapshot_url": snapshot,
                "recovery": {"snapshot_ref": snapshot, "after_sequence": head},
            })));
        }
        Ok(after)
    }

    /// `None`: not a stream route. `Some(Err)`: answer this response. `Some(Ok)`: the server should stream.
    pub fn stream_route(&self, r: &Req) -> Option<Result<StreamPlan, Resp>> {
        let run = r.path.strip_prefix(PREFIX)?.strip_prefix("/runs/")?;
        let run = run.strip_suffix("/events/stream").or_else(|| run.strip_suffix("/stream"))?;
        if r.method != "GET" || run.is_empty() || run.contains('/') {
            return None;
        }
        if let Some(t) = &self.cfg.token {
            if !bearer_ok(r, t) {
                return Some(Err(problem("unauthorized", 401, json!({}))));
            }
        }
        if self.store.state(run).is_none() {
            return Some(Err(problem("not_found", 404, json!({}))));
        }
        let mut after = Ok(0);
        for raw in [query_param(&r.query, "after_sequence"), r.headers.get("last-event-id").map(String::as_str)].into_iter().flatten() {
            match (raw.trim().parse::<i64>().ok().filter(|n| *n >= 0), after) {
                (Some(n), Ok(a)) => after = Ok(a.max(n)),
                (None, _) => after = Err(()),
                _ => {}
            }
        }
        Some(self.cursor_check(run, after).map(|after| StreamPlan { run: run.to_string(), after }))
    }

    fn diff(&self, id: &str) -> Resp {
        for (_, s) in self.states() {
            if s["diff"]["proposal_id"] == id {
                return ok(&self.env(json!({"kind": "proposal", "id": id}), 1, "ok", json!({"lines": s["diff"]["lines"]})));
            }
        }
        problem("not_found", 404, json!({}))
    }

    fn memory(&self) -> Resp {
        let mut items: Vec<Value> = Vec::new();
        for (_, s) in self.states() {
            for m in s["memory"].as_array().into_iter().flatten() {
                if !items.iter().any(|x| x["memory_id"] == m["memory_id"]) {
                    items.push(m.clone());
                }
            }
        }
        ok(&self.env(Value::Null, 1, "ok", json!({"items": items})))
    }

    fn find_decision(&self, id: &str) -> Option<Value> {
        self.states().into_iter().map(|(_, s)| s["decision"].clone()).find(|d| d["decision_id"] == id)
    }

    fn decision(&self, id: &str) -> Resp {
        let Some(d) = self.find_decision(id) else { return problem("not_found", 404, json!({})) };
        ok(&self.env(
            json!({"kind": "decision", "id": id}), 1, "pending",
            json!({"available_commands": d["available_commands"], "needs_step_up": d["needs_step_up"], "domain_revision": d["domain_revision"]}),
        ))
    }

    /// Validation of the contract's decision command; nothing is executed or stored (read-only by default).
    fn respond(&self, id: &str, r: &Req) -> Resp {
        if r.headers.get("x-csrf-token") != Some(&self.csrf) {
            return problem("csrf_failed", 403, json!({}));
        }
        if r.headers.get("idempotency-key").is_none_or(String::is_empty) {
            return problem("validation_error", 422, json!({}));
        }
        let b: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        if !b["note"].is_string() {
            return field_error("note", "must_be_string");
        }
        let Some(expected) = b["expected_revision"].as_i64() else { return field_error("expected_revision", "required_integer") };
        let Some(d) = self.find_decision(id) else { return problem("not_found", 404, json!({})) };
        let current = d["domain_revision"].as_i64().unwrap_or(0);
        if expected != current {
            return problem("stale_revision", 409, json!({"conflict": {"expected_revision": expected, "current_revision": current, "diff_ref": null}}));
        }
        let offered = b["response"].as_str().is_some_and(|c| d["available_commands"].as_array().is_some_and(|a| a.iter().any(|x| x == c)));
        if !offered {
            return problem("command_not_available", 403, json!({}));
        }
        problem("commands_not_implemented", 501, json!({}))
    }

    /// Admin: append / purge on a run. Does not exist (404) unless an admin token is configured.
    fn admin(&self, r: &Req, tail: &str) -> Resp {
        let Some(token) = &self.cfg.admin_token else { return problem("not_found", 404, json!({})) };
        if !bearer_ok(r, token) {
            return problem("unauthorized", 401, json!({}));
        }
        let parts: Vec<&str> = tail.split('/').collect();
        let body: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        let bad = || problem("validation_error", 422, json!({}));
        match (r.method.as_str(), parts.as_slice()) {
            ("POST", ["runs", run, "events"]) => {
                let Some(kind) = body["kind"].as_str() else { return bad() };
                let mut ev = NewEvent::new(kind, body["entity_kind"].as_str().unwrap_or("run"), body["entity_id"].as_str().unwrap_or(run), body.get("data").cloned().unwrap_or_else(|| Value::Object(Map::new())));
                ev.occurred_at = body["occurred_at"].as_str().map(String::from);
                match self.store.emit(run, ev) {
                    Ok(e) => ok(&e),
                    Err(_) => bad(),
                }
            }
            ("POST", ["runs", run, "purge"]) => match body["through"].as_i64().map(|n| self.store.purge_through(run, n)) {
                Some(Ok(())) => ok(&json!({"ok": true})),
                _ => bad(),
            },
            ("POST", ["ingest", "engine-run"]) => match crate::ingest::engine_run_report(&*self.store, &|id| self.store.head(id).is_some(), &body) {
                Ok(run_id) => ok(&json!({"run_id": run_id})),
                Err(e) if e.contains("already") => problem("already_ingested", 409, json!({})),
                Err(e) => problem("validation_error", 422, json!({"message": e})),
            },
            _ => problem("not_found", 404, json!({})),
        }
    }
}
