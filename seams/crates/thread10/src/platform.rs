//! Release correlation through the control-api (P2R) in process: `App::handle` over a MemStore, a throwaway Ed25519 key,
//! a fixed clock. The platform is a DOUBLE (no socket, no real exporter); the correlation code under test is real.
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use control_api::app::{App, Config, Req};
use control_api::auth::KeyRing;
use control_api::correlation::MemSuccessorSink;
use control_api::store::MemStore;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const TENANT: &str = "t1";
const T0_MS: u64 = 1_800_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub release_id: String,
    pub agent_id: String,
    pub alias: String,
    pub candidate_hash: String,
}

pub struct Platform {
    app: App,
    key: SigningKey,
    sink: Arc<MemSuccessorSink>,
    jti: AtomicU64,
}

impl Platform {
    pub fn new() -> Platform {
        let key = SigningKey::from_bytes(&[7u8; 32]); // throwaway, test-only
        let ring = json!({"cb": ["core-bridge", "control-api", B64.encode(key.verifying_key().to_bytes())]});
        let sink = Arc::new(MemSuccessorSink::default());
        let mut cfg = Config::new(Arc::new(KeyRing::from_json(&ring).expect("ring")));
        cfg.admin = true; // the in-process `/_e2e/config` channel records the published release
        cfg.successors = Some(sink.clone());
        let app = App::with_clock(cfg, Box::new(MemStore::default()), Box::new(|| T0_MS as f64 / 1000.0));
        Platform { app, key, sink, jti: AtomicU64::new(0) }
    }

    fn call(&self, path: &str, body: &Value, token: Option<&str>, headers: &[(&str, &str)]) -> (u16, Value) {
        let mut h: HashMap<String, String> = headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.to_string())).collect();
        if let Some(t) = token {
            h.insert("authorization".into(), format!("Bearer {t}"));
        }
        let r = self.app.handle(&Req { method: "POST".into(), path: path.into(), query: String::new(), headers: h, body: body.to_string().into_bytes() });
        (r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null))
    }

    fn token(&self) -> String {
        let now = (T0_MS / 1000) as i64;
        let claims = json!({"iss": "core-bridge", "aud": "control-api", "sub": "thread10-engine", "scope": "release_events", "purpose": "platform_releases",
                            "iat": now, "exp": now + 60, "jti": format!("t10-{}", self.jti.fetch_add(1, Ordering::SeqCst)), "tenant_id": TENANT});
        let head = B64.encode(json!({"alg": "EdDSA", "kid": "cb", "typ": "JWT"}).to_string());
        let body = B64.encode(claims.to_string());
        let sig = self.key.sign(format!("{head}.{body}").as_bytes());
        format!("{head}.{body}.{}", B64.encode(sig.to_bytes()))
    }

    /// The engine records the release it published (the correlation matches events against this record).
    pub fn record_release(&self, r: &Release) -> Result<(), String> {
        let cfg = json!({"published_releases": [{"tenant": TENANT, "release_id": r.release_id, "agent_id": r.agent_id, "alias": r.alias, "candidate_hash": r.candidate_hash}]});
        match self.call("/_e2e/config", &cfg, None, &[]) {
            (200, _) => Ok(()),
            (st, b) => Err(format!("record_release: {st} {b}")),
        }
    }

    /// `release.published` for `r` under `event_id`: (HTTP status, body).
    pub fn post_published(&self, event_id: &str, r: &Release) -> (u16, Value) {
        let ev = json!({"event_id": event_id, "event": {"event_type": "release.published", "entity": "release", "entity_id": r.release_id,
                        "payload": {"release_id": r.release_id, "agent_id": r.agent_id, "alias": r.alias}}});
        self.call("/internal/v1/platform/releases", &ev, Some(&self.token()), &[("Idempotency-Key", event_id)])
    }

    pub fn successor_keys(&self) -> Vec<String> {
        self.sink.runs().into_iter().map(|r| r.unique_key).collect()
    }
}

impl Default for Platform {
    fn default() -> Self {
        Platform::new()
    }
}
