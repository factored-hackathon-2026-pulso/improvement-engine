//! Shared helpers of the Rust-only control-api tests: a signing rig (own Ed25519 keys, injected clock) and `App::handle`
//! requests. Nothing here talks to a socket; `App::handle` is the pure request -> response function.
#![allow(dead_code)]
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use control_api::app::{App, Config, Req, Resp};
use control_api::auth::KeyRing;
use control_api::store::MemStore;
use core_client::canon::{jcs, sha256_hex};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub const SUB: &str = "exporter-binding-1";
pub const T0: f64 = 1_800_000_000.0;

pub struct Rig {
    pub app: Arc<App>,
    keys: HashMap<&'static str, SigningKey>,
    clock: Arc<AtomicU64>,
    jti: AtomicU64,
    id: u64,
}

static RIGS: AtomicU64 = AtomicU64::new(0);

fn pubkey(sk: &SigningKey) -> String {
    B64.encode(sk.verifying_key().to_bytes())
}

impl Rig {
    pub fn new() -> Rig {
        Rig::with(|_| {})
    }

    pub fn with(tweak: impl FnOnce(&mut Config)) -> Rig {
        Rig::with_store(tweak, Box::new(MemStore::default()))
    }

    pub fn with_store(tweak: impl FnOnce(&mut Config), store: Box<dyn control_api::store::Store>) -> Rig {
        let keys: HashMap<&'static str, SigningKey> =
            [("cb", 1u8), ("ob", 2), ("ex", 3)].into_iter().map(|(k, n)| (k, SigningKey::from_bytes(&[n; 32]))).collect();
        let ring = json!({
            "cb": ["core-bridge", "control-api", pubkey(&keys["cb"])],
            "ob": ["core-bridge", "control-api", pubkey(&keys["ob"])],
            "ex": ["core-bridge", "lab-broker", pubkey(&keys["ex"])],
        });
        let mut cfg = Config::new(Arc::new(KeyRing::from_json(&ring).unwrap()));
        cfg.upload_pin = Some((SUB.into(), "t1".into()));
        cfg.admin = true;
        tweak(&mut cfg);
        let clock = Arc::new(AtomicU64::new((T0 * 1000.0) as u64));
        let c = clock.clone();
        let app = Arc::new(App::with_clock(cfg, store, Box::new(move || c.load(Ordering::SeqCst) as f64 / 1000.0)));
        Rig { app, keys, clock, jti: AtomicU64::new(0), id: RIGS.fetch_add(1, Ordering::SeqCst) }
    }

    pub fn now(&self) -> f64 {
        self.clock.load(Ordering::SeqCst) as f64 / 1000.0
    }

    pub fn advance(&self, secs: f64) {
        self.clock.fetch_add((secs * 1000.0) as u64, Ordering::SeqCst);
    }

    /// A service JWT; `over` replaces or adds claims.
    pub fn token(&self, kid: &str, aud: &str, scope: &str, tenant: &str, over: Value) -> String {
        let now = self.now() as i64;
        let mut claims = json!({"iss": "core-bridge", "aud": aud, "sub": SUB, "scope": scope, "purpose": scope, "iat": now, "exp": now + 60,
                                "jti": format!("j{}-{}", self.id, self.jti.fetch_add(1, Ordering::SeqCst)), "tenant_id": tenant});
        for (k, v) in over.as_object().into_iter().flatten() {
            claims[k] = v.clone();
        }
        let head = B64.encode(json!({"alg": "EdDSA", "kid": kid, "typ": "JWT"}).to_string());
        let body = B64.encode(claims.to_string());
        let sig = self.keys[kid].sign(format!("{head}.{body}").as_bytes());
        format!("{head}.{body}.{}", B64.encode(sig.to_bytes()))
    }

    pub fn call(&self, method: &str, path: &str, body: Option<&Value>, token: Option<&str>, headers: &[(&str, &str)]) -> (u16, Value) {
        let mut h: HashMap<String, String> = headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.to_string())).collect();
        if let Some(t) = token {
            h.insert("authorization".into(), format!("Bearer {t}"));
        }
        let r: Resp = self.app.handle(&Req { method: method.into(), path: path.into(), query: String::new(), headers: h, body: body.map(|b| b.to_string().into_bytes()).unwrap_or_default() });
        (r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null))
    }

    pub fn admin(&self, cfg: Value) {
        assert_eq!(self.call("POST", "/_e2e/config", Some(&cfg), None, &[]).0, 200);
    }

    // ---- ingest helpers ------------------------------------------------------------------------------------
    pub fn upload_schema(&self) -> Value {
        let content = json!({"schema": "platform_event/1"});
        let digest = sha256_hex(jcs(&content).unwrap().as_bytes());
        let body = json!({"schema_version": "1", "binding_ref": SUB, "source_schema_ref": null, "classification": "treated",
                          "information_partition": "p1", "artifact_kind": "schema", "media_type": "application/json", "encoding": "json",
                          "content": content, "content_digest": digest});
        let tok = self.token("ex", "lab-broker", "artifact_write", "t1", json!({"purpose": "artifact_upload"}));
        let (st, out) = self.call("POST", "/internal/v1/broker/artifacts", Some(&body), Some(&tok), &[("Idempotency-Key", &digest)]);
        assert!(st == 201 || st == 200, "{st} {out}");
        out["artifact_ref"].clone()
    }

    pub fn post_batch(&self, batch: &Value) -> (u16, Value) {
        let tok = self.token("ob", "control-api", "observations", "t1", json!({"purpose": "platform_observations"}));
        self.call("POST", "/internal/v1/platform/observations", Some(batch), Some(&tok), &[("Idempotency-Key", batch["batch_digest"].as_str().unwrap())])
    }

    pub fn cursor(&self) -> Value {
        let tok = self.token("ob", "control-api", "observations", "t1", json!({"purpose": "platform_observations"}));
        let (st, v) = self.call("GET", "/internal/v1/platform/exporters/plat-a.events/partitions/tenant.t1/cursor", None, Some(&tok), &[]);
        assert_eq!(st, 200, "{v}");
        v
    }
}

pub fn domain(seq: i64, event_type: &str, schema: &Value) -> Value {
    json!({"kind": "platform_event", "level": null,
           "source_event": {"kind": "domain_event", "event_type": event_type, "event_id": format!("EVT-{seq}"), "payload": {"secret_marker": "DO-NOT-STORE"}},
           "native_event_id": format!("EVT-{seq}"), "source_event_digest": sha256_hex(format!("evt{seq}{event_type}").as_bytes()),
           "source_event_ref": null, "source_schema_ref": schema, "source_run_ref": null, "source_sequence": seq, "episode_ref": null, "goal_ref": null, "layer_mapping_ref": null,
           "observed_at": "2026-03-01T10:00:00Z", "trace_refs": [], "coverage_marker": null})
}

pub fn finding(native: &str, code: &str, details: Value, schema: &Value) -> Value {
    json!({"kind": "platform_event", "level": null,
           "source_event": {"kind": "exporter_finding", "finding_code": code, "severity": "warning", "details": details},
           "native_event_id": native, "source_event_digest": sha256_hex(native.as_bytes()), "source_event_ref": null,
           "source_schema_ref": schema, "source_run_ref": null, "source_sequence": null, "episode_ref": null, "goal_ref": null, "layer_mapping_ref": null, "observed_at": "2026-03-01T10:00:00Z",
           "trace_refs": [], "coverage_marker": null})
}

pub fn late(mut e: Value) -> Value {
    e["coverage_marker"] = json!("late");
    e
}

pub struct B {
    pub revision: Option<i64>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub cursor: String,
    pub events: Vec<Value>,
}

pub fn batch(b: B) -> Value {
    let mut v = json!({"contract_version": "pulso-observations-2", "source_id": "plat-a.events", "tenant_id": "t1", "partition": "tenant.t1",
                       "scan_mode": if b.revision.is_some() { "fast_poll" } else { "rescan" }, "expected_cursor_revision": b.revision,
                       "from_seq": b.from, "to_seq": b.to, "cursor": b.cursor, "cut_ref": null, "events": b.events, "verification_receipts": []});
    v["batch_digest"] = json!(sha256_hex(jcs(&v).unwrap().as_bytes()));
    v
}

// ---- lab / grants ------------------------------------------------------------------------------------------------------
pub const BROKER: &str = "/internal/v1/broker";

/// A sqlite lab in the ED0L shape (`lab_rows`, `lab_meta`); rows are `(metric, window, g_group, numerator, count)`.
pub fn make_lab(name: &str, k: i64, rows: &[(&str, &str, &str, i64, i64)]) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("control-api-lab-{}-{name}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let con = rusqlite::Connection::open(&path).unwrap();
    con.execute_batch(
        "create table lab_rows (evidence_ref text primary key, metric_id text, window_id text, g_group text, numerator integer, count integer, digest text);
         create table lab_meta (key text primary key, value text);",
    )
    .unwrap();
    for (i, (m, w, g, n, c)) in rows.iter().enumerate() {
        con.execute("insert into lab_rows values (?,?,?,?,?,?,?)", rusqlite::params![format!("ev_{i:016}"), m, w, g, n, c, "d"]).unwrap();
    }
    con.execute("insert into lab_meta values ('k', ?)", [k.to_string()]).unwrap();
    path
}

impl Rig {
    pub fn lab_token(&self, tenant: &str, scope: &str) -> String {
        self.token("ex", "lab-broker", scope, tenant, json!({}))
    }

    pub fn bind_ref(&self, binding_ref: &str, tenant: &str) {
        self.admin(json!({"preauthorized_bindings": [{"binding_ref": binding_ref, "tenant": tenant}]}));
    }

    pub fn issue_grant(&self, tenant: &str, binding_ref: &str, scope: &str, ttl: i64) -> (u16, Value) {
        let tok = self.lab_token(tenant, "grant_issue");
        self.call("POST", &format!("{BROKER}/grants"), Some(&json!({"binding_ref": binding_ref, "scope": scope, "ttl_seconds": ttl})), Some(&tok), &[])
    }

    pub fn lab(&self, method: &str, path: &str, body: Option<&Value>, tenant: &str, grant: Option<&str>) -> (u16, Value) {
        let tok = self.lab_token(tenant, "lab");
        let mut h = Vec::new();
        if let Some(g) = grant {
            h.push(("X-Grant-Ref", g));
        }
        self.call(method, &format!("{BROKER}{path}"), body, Some(&tok), &h)
    }

    /// Issues a lab grant for `binding_ref` and opens a session with it; returns `(grant_ref, session_ref)`.
    pub fn open_session(&self, tenant: &str, binding_ref: &str, ttl: i64) -> (String, String) {
        let (st, g) = self.issue_grant(tenant, binding_ref, "lab", ttl);
        assert_eq!(st, 201, "{g}");
        let grant = g["grant_ref"].as_str().unwrap().to_string();
        let (st, s) = self.lab("POST", "/lab/sessions", Some(&json!({"binding_ref": binding_ref})), tenant, Some(&grant));
        assert_eq!(st, 200, "{s}");
        (grant, s["session_ref"].as_str().unwrap().to_string())
    }
}
