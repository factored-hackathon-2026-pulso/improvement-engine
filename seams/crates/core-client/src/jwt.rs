//! Service JWT (class i): header exactly `{alg: EdDSA, kid, typ: JWT}`, `iss=control-api`,
//! `aud=core-bridge`, `sub=worker:<id>`, per-route `purpose`, `exp - iat <= 300`, fresh `jti` per attempt.
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique per attempt (the receiver rejects replays); not a secret.
pub fn fresh_jti(worker: &str) -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut h = Sha256::new();
    h.update(format!("{worker}|{}|{nanos}|{n}", std::process::id()));
    let d = h.finalize();
    d[..16].iter().map(|b| format!("{b:02x}")).collect()
}

pub struct JwtParams<'a> {
    pub kid: &'a str,
    pub worker_id: &'a str,
    pub purpose: &'a str,
    /// `None` only for `version_probe`.
    pub tenant_id: Option<&'a str>,
    pub job_id: Option<&'a str>,
    pub iat: i64,
    pub ttl_s: i64,
    pub jti: &'a str,
}

pub fn sign(key: &SigningKey, p: &JwtParams) -> String {
    let header = json!({"alg": "EdDSA", "kid": p.kid, "typ": "JWT"});
    let mut c = Map::new();
    c.insert("iss".into(), json!(crate::pins::JWT_ISS));
    c.insert("aud".into(), json!(crate::pins::JWT_AUD));
    c.insert("sub".into(), json!(format!("worker:{}", p.worker_id)));
    c.insert("purpose".into(), json!(p.purpose));
    c.insert("iat".into(), json!(p.iat));
    c.insert("exp".into(), json!(p.iat + p.ttl_s.min(crate::pins::JWT_MAX_TTL_SECONDS)));
    c.insert("jti".into(), json!(p.jti));
    if let Some(t) = p.tenant_id {
        c.insert("tenant_id".into(), json!(t));
    }
    if let Some(j) = p.job_id {
        c.insert("job_id".into(), json!(j));
    }
    let enc = |v: &Value| B64.encode(serde_json::to_vec(v).expect("json"));
    let signing_input = format!("{}.{}", enc(&header), enc(&Value::Object(c)));
    let sig = key.sign(signing_input.as_bytes());
    format!("{signing_input}.{}", B64.encode(sig.to_bytes()))
}
