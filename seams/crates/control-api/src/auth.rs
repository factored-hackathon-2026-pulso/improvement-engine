//! A03 service JWT verification (mirrors the Python double `codex_standin.jwtsvc.Verifier` check for check):
//! `typ=JWT`, EdDSA, header exactly {alg, kid, typ}, kid bound to one (iss, aud), `exp - now <= 330`, singular scope,
//! tenant claim required, receiver-owned `jti` replay set.
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use crate::store::Store;
use std::sync::Mutex;

#[derive(Debug, PartialEq, Eq)]
pub struct Denied {
    pub reason: &'static str,
    pub status: u16,
}

fn deny(reason: &'static str) -> Denied {
    Denied { reason, status: 401 }
}

pub struct KeyRing {
    entries: HashMap<String, (String, String, VerifyingKey)>,
}

impl KeyRing {
    /// `{kid: [iss, aud, public_key_b64url]}`.
    pub fn from_json(v: &Value) -> Result<KeyRing, String> {
        let obj = v.as_object().ok_or("ring must be an object")?;
        let mut entries = HashMap::new();
        for (kid, e) in obj {
            let a = e.as_array().filter(|a| a.len() == 3).ok_or("ring entry must be [iss, aud, key]")?;
            let s = |i: usize| a[i].as_str().ok_or("ring entry fields must be strings");
            let raw = B64.decode(s(2)?.trim_end_matches('=')).map_err(|e| e.to_string())?;
            let arr: [u8; 32] = raw.try_into().map_err(|_| "public key must be 32 bytes")?;
            let key = VerifyingKey::from_bytes(&arr).map_err(|e| e.to_string())?;
            entries.insert(kid.clone(), (s(0)?.to_string(), s(1)?.to_string(), key));
        }
        Ok(KeyRing { entries })
    }
}

pub struct Expect<'a> {
    pub aud: &'a str,
    pub scope: Option<&'a str>,
    pub purpose: Option<&'a str>,
}

/// One verifier (one jti store) per receiver.
pub struct Verifier {
    ring: std::sync::Arc<KeyRing>,
    seen: Mutex<HashMap<(String, String), f64>>,
    /// Tokens issued before this instant are refused: the jti set is process memory, so a token captured before a restart
    /// could otherwise be replayed once after it. 0 disables the rule.
    boot_floor: f64,
    /// Durable replay set (`scope` names this verifier in it); `None` keeps the set in process memory.
    durable: Option<(std::sync::Arc<dyn Store>, &'static str)>,
}

impl Verifier {
    pub fn new(ring: std::sync::Arc<KeyRing>) -> Verifier {
        Verifier { ring, seen: Mutex::new(HashMap::new()), boot_floor: 0.0, durable: None }
    }

    /// Refuse (`pre_boot_token`) any token whose `iat` precedes `floor` (epoch seconds, whole): replay protection across restarts.
    pub fn with_boot_floor(mut self, floor: f64) -> Verifier {
        self.boot_floor = floor.floor();
        self
    }

    /// Keep the jti replay set in `store` under `scope` when the store persists it: a restart then forgets nothing, so the
    /// boot floor (which exists only because the in-memory set is lost) is dropped.
    pub fn with_store(mut self, store: std::sync::Arc<dyn Store>, scope: &'static str) -> Verifier {
        if store.durable_replay() {
            self.boot_floor = 0.0;
            self.durable = Some((store, scope));
        }
        self
    }

    pub fn verify(&self, token: &str, now: f64, want: &Expect) -> Result<Value, Denied> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(deny("malformed"));
        }
        let dec = |s: &str| B64.decode(s.trim_end_matches('=')).map_err(|_| deny("malformed"));
        let head: Value = serde_json::from_slice(&dec(parts[0])?).map_err(|_| deny("malformed"))?;
        let claims: Value = serde_json::from_slice(&dec(parts[1])?).map_err(|_| deny("malformed"))?;
        let sig = dec(parts[2])?;
        let (head, cl) = match (head.as_object(), claims.as_object()) {
            (Some(h), Some(c)) => (h, c),
            _ => return Err(deny("malformed")),
        };
        let exact: HashSet<&str> = head.keys().map(String::as_str).collect();
        if head.get("alg").and_then(Value::as_str) != Some("EdDSA")
            || head.get("typ").and_then(Value::as_str) != Some("JWT")
            || exact != HashSet::from(["alg", "kid", "typ"])
        {
            return Err(deny("bad_header"));
        }
        let entry = head.get("kid").and_then(Value::as_str).and_then(|k| self.ring.entries.get(k)).ok_or(deny("unknown_kid"))?;
        let signature = Signature::from_slice(&sig).map_err(|_| deny("bad_signature"))?;
        entry.2.verify_strict(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature).map_err(|_| deny("bad_signature"))?;
        let s = |k: &str| cl.get(k).and_then(Value::as_str);
        if s("iss") != Some(entry.0.as_str()) || s("aud") != Some(entry.1.as_str()) || s("aud") != Some(want.aud) {
            return Err(deny("wrong_audience"));
        }
        let exp = cl.get("exp").and_then(Value::as_f64);
        let jti = s("jti").filter(|j| !j.is_empty());
        let (Some(exp), Some(jti)) = (exp, jti) else { return Err(deny("missing_claims")) };
        if exp <= now {
            return Err(deny("expired"));
        }
        if self.boot_floor > 0.0 && cl.get("iat").and_then(Value::as_f64).is_none_or(|iat| iat < self.boot_floor) {
            return Err(deny("pre_boot_token"));
        }
        if exp - now > 330.0 {
            return Err(deny("ttl_too_long"));
        }
        if want.scope.is_some_and(|w| s("scope") != Some(w)) {
            return Err(Denied { reason: "scope_denied", status: 403 });
        }
        if want.purpose.is_some_and(|w| s("purpose") != Some(w)) {
            return Err(Denied { reason: "purpose_denied", status: 403 });
        }
        if jti.chars().count() > 256 || s("iss").is_some_and(|i| i.chars().count() > 256) {
            return Err(deny("malformed")); // the durable replay set bounds both
        }
        if s("tenant_id").is_none_or(|t| t.is_empty() || t.chars().count() > 128 || t.contains('\0')) {
            return Err(Denied { reason: "tenant_required", status: 403 });
        }
        if let Some((store, scope)) = &self.durable {
            if !store.jti_claim(scope, s("iss").unwrap(), jti, exp, now) {
                return Err(deny("jti_replayed"));
            }
            return Ok(claims.clone());
        }
        let mut seen = self.seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // A token past `exp` is rejected before this point, so its entry is dead weight: evict to bound memory.
        seen.retain(|_, e| *e > now);
        if seen.insert((s("iss").unwrap().to_string(), jti.to_string()), exp).is_some() {
            return Err(deny("jti_replayed"));
        }
        Ok(claims.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_client::jwt::{JwtParams, sign};
    use ed25519_dalek::SigningKey;
    use std::sync::Arc;

    fn ring(sk: &SigningKey) -> Arc<KeyRing> {
        let v = serde_json::json!({"k": ["control-api", "core-bridge", B64.encode(sk.verifying_key().to_bytes())]});
        Arc::new(KeyRing::from_json(&v).unwrap())
    }

    #[test]
    fn accepts_a_k1_core_client_token_once_and_then_flags_the_replay() {
        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let v = Verifier::new(ring(&sk));
        let tok = sign(&sk, &JwtParams { kid: "k", worker_id: "w", purpose: "core_task_invoke", tenant_id: Some("t1"), job_id: None, iat: 1000, ttl_s: 60, jti: "j1" });
        let want = Expect { aud: "core-bridge", scope: None, purpose: Some("core_task_invoke") };
        assert_eq!(v.verify(&tok, 1010.0, &want).unwrap()["tenant_id"], "t1");
        assert_eq!(v.verify(&tok, 1010.0, &want), Err(deny("jti_replayed")));
    }

    #[test]
    fn jti_set_evicts_expired_entries() {
        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let v = Verifier::new(ring(&sk));
        let want = Expect { aud: "core-bridge", scope: None, purpose: None };
        for i in 0..50 {
            let j = format!("j{i}");
            let tok = sign(&sk, &JwtParams { kid: "k", worker_id: "w", purpose: "p", tenant_id: Some("t1"), job_id: None, iat: 1000, ttl_s: 60, jti: &j });
            v.verify(&tok, 1010.0, &want).unwrap();
        }
        let tok = sign(&sk, &JwtParams { kid: "k", worker_id: "w", purpose: "p", tenant_id: Some("t1"), job_id: None, iat: 5000, ttl_s: 60, jti: "late" });
        v.verify(&tok, 5010.0, &want).unwrap();
        assert_eq!(v.seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len(), 1);
    }

    #[test]
    fn alg_none_hs256_and_unknown_kid_are_rejected() {
        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let v = Verifier::new(ring(&sk));
        let want = Expect { aud: "core-bridge", scope: None, purpose: None };
        let body = B64.encode(br#"{"iss":"control-api","aud":"core-bridge","exp":1060,"jti":"x","tenant_id":"t1"}"#);
        for h in [r#"{"alg":"none","kid":"k","typ":"JWT"}"#, r#"{"alg":"HS256","kid":"k","typ":"JWT"}"#, r#"{"alg":"EdDSA","kid":"zz","typ":"JWT"}"#] {
            let tok = format!("{}.{}.", B64.encode(h), body);
            assert!(v.verify(&tok, 1010.0, &want).is_err(), "{h}");
        }
    }

    #[test]
    fn tampered_payload_and_expiry_are_rejected() {
        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let v = Verifier::new(ring(&sk));
        let tok = sign(&sk, &JwtParams { kid: "k", worker_id: "w", purpose: "p", tenant_id: Some("t1"), job_id: None, iat: 1000, ttl_s: 60, jti: "j2" });
        let want = Expect { aud: "core-bridge", scope: None, purpose: None };
        assert_eq!(v.verify(&tok, 2000.0, &want), Err(deny("expired")));
        let mut p: Vec<&str> = tok.split('.').collect();
        let forged = B64.encode(br#"{"iss":"control-api","aud":"core-bridge","exp":9999999999,"jti":"j3","tenant_id":"t9"}"#);
        p[1] = &forged;
        assert_eq!(v.verify(&p.join("."), 1010.0, &want), Err(deny("bad_signature")));
    }
}
