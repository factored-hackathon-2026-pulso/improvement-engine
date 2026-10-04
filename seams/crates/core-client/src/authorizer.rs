//! Human command-authorization for the Core registry (`approve`, `publish`): a CLAUDE-STANDIN for the human.
//!
//! HONEST LABEL: `LocalSimAuthorizer` is a simulated issuer (`auth.simulated=true`), the Rust twin of the sandbox-only
//! Python `local-identity` issuer (`principal.py`: `human_principal`, `binding_digest`). It is NOT a human and NOT the
//! platform issuer; the real thing replaces the `Authorizer` impl, not the registry client.
//! The JWS is the exact Core `Principal` shape (`typ=principal+jws`, EdDSA, header `{alg,kid,typ}`), `auth.level =
//! step_up`, with the binding (operation, proposal, candidate hash, revision) echoed in string `attrs` plus a
//! `binding_digest`. Core ignores those attrs, so the CLIENT must check them (see `registry`).
//! Never logged: `Jws` and `LocalSimAuthorizer` have redacting `Debug`; the key never leaves this module.
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The authorization JWS (a bearer credential). `Debug` never reveals it.
#[derive(Clone, PartialEq, Eq)]
pub struct Jws(String);

impl Jws {
    pub fn new(compact: String) -> Jws {
        Jws(compact)
    }
    /// The exact compact bytes to put in `Authorization: Bearer`. The only way out.
    pub fn reveal(&self) -> &str {
        &self.0
    }
    /// Stable short fingerprint (replay tracking) that does not reveal the token.
    pub fn fingerprint(&self) -> String {
        Sha256::digest(self.0.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
    }
    /// Decoded payload (UNVERIFIED: the signature is Core's to check); used for the client-side binding check.
    pub fn claims(&self) -> Option<Value> {
        let p = self.0.split('.').nth(1)?;
        serde_json::from_slice(&B64.decode(p).ok()?).ok()
    }
}

impl fmt::Debug for Jws {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Jws(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalTarget {
    pub proposal_id: String,
    /// Bare lowercase hex-64.
    pub candidate_hash: String,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Only `approve`, `reject` and `publish` take a proposal target.
    UnsupportedOperation(String),
    InvalidTarget(String),
}

pub trait Authorizer {
    /// One single-use authorization for `operation` on exactly `target`.
    fn authorize(&self, operation: &str, target: &ProposalTarget) -> Result<Jws, AuthError>;
}

static SEQ: AtomicU64 = AtomicU64::new(0);

pub struct LocalSimAuthorizer {
    kid: String,
    key: SigningKey,
    tenant: String,
    actor: String,
    ttl_s: i64,
}

impl fmt::Debug for LocalSimAuthorizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalSimAuthorizer(kid={:?}, tenant={:?}, actor={:?}, key=<redacted>)", self.kid, self.tenant, self.actor)
    }
}

pub fn iso_z(epoch_s: i64) -> String {
    let days = epoch_s.div_euclid(86_400);
    let rem = epoch_s.rem_euclid(86_400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `sha256` of the canonical binding (sorted keys, compact), as `principal.py::binding_digest`.
pub fn binding_digest(binding: &Value) -> String {
    let mut out = String::new();
    canon_py(binding, &mut out);
    Sha256::digest(out.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// `json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=True)`: explicit key sort and `\uXXXX` escapes.
fn canon_py(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canon_str(k, out);
                out.push(':');
                canon_py(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canon_py(x, out);
            }
            out.push(']');
        }
        Value::String(s) => canon_str(s, out),
        other => out.push_str(&other.to_string()),
    }
}

fn canon_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (' '..='\u{7f}').contains(&c) => out.push(c),
            c => {
                let mut b = [0u16; 2];
                for u in c.encode_utf16(&mut b) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
        }
    }
    out.push('"');
}

fn is_ref(s: &str) -> bool {
    (1..=200).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl LocalSimAuthorizer {
    /// `kid` must carry the sandbox prefix `local-sim-human-` (the contract scan rejects it in remote configuration).
    pub fn new(kid: &str, seed: [u8; 32], tenant: &str, actor: &str) -> LocalSimAuthorizer {
        LocalSimAuthorizer { kid: kid.into(), key: SigningKey::from_bytes(&seed), tenant: tenant.into(), actor: actor.into(), ttl_s: 60 }
    }

    /// Sign at an explicit time (tests and `authorize`).
    pub fn authorize_at(&self, operation: &str, target: &ProposalTarget, now: i64) -> Result<Jws, AuthError> {
        if !matches!(operation, "approve" | "reject" | "publish") {
            return Err(AuthError::UnsupportedOperation(operation.into()));
        }
        if !is_ref(&target.proposal_id) || !is_hex64(&target.candidate_hash) {
            return Err(AuthError::InvalidTarget("proposal_id not a valid ref or candidate_hash not hex-64".into()));
        }
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let command_ref = format!("cmd-{operation}-{n}-{now}");
        let challenge_ref = format!("chal-{operation}-{n}-{now}");
        let tgt = json!({"kind": "proposal", "proposal_id": target.proposal_id, "candidate_hash": target.candidate_hash,
                         "expected_revision": target.expected_revision});
        let digest = binding_digest(&json!({"tenant_id": self.tenant, "actor_ref": self.actor, "command_ref": command_ref,
            "operation": operation, "challenge_ref": challenge_ref, "target": tgt}));
        let payload = json!({
            "type": "builder", "id": self.actor, "roles": ["constructor", "aprobador"], "scopes": [],
            "attrs": {"actor": "human", "tenant": self.tenant, "issuer": "local-identity", "command_ref": command_ref,
                      "operation": operation, "challenge_ref": challenge_ref, "target_kind": "proposal",
                      "binding_digest": digest, "proposal_id": target.proposal_id, "candidate_hash": target.candidate_hash,
                      "expected_revision": target.expected_revision.to_string()},
            "auth": {"level": "step_up", "at": iso_z(now), "simulated": true},
            "exp": iso_z(now + self.ttl_s),
        });
        let head = B64.encode(serde_json::to_vec(&json!({"alg": "EdDSA", "kid": self.kid, "typ": "principal+jws"})).expect("json"));
        let body = B64.encode(serde_json::to_vec(&payload).expect("json"));
        let sig = self.key.sign(format!("{head}.{body}").as_bytes());
        Ok(Jws(format!("{head}.{body}.{}", B64.encode(sig.to_bytes()))))
    }
}

impl Authorizer for LocalSimAuthorizer {
    fn authorize(&self, operation: &str, target: &ProposalTarget) -> Result<Jws, AuthError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.authorize_at(operation, target, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};

    const SEED: [u8; 32] = [0x5a; 32];
    fn tgt() -> ProposalTarget {
        ProposalTarget { proposal_id: "prop-1".into(), candidate_hash: "ab".repeat(32), expected_revision: 3 }
    }
    fn auth() -> LocalSimAuthorizer {
        LocalSimAuthorizer::new("local-sim-human-1", SEED, "t1", "local-supervisor")
    }

    #[test]
    fn iso_z_matches_known_instants() {
        assert_eq!(iso_z(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_z(1_709_210_096), "2024-02-29T12:34:56.000Z");
    }

    #[test]
    fn jws_has_the_core_principal_shape_and_a_valid_signature() {
        let j = auth().authorize_at("approve", &tgt(), 1_000_000).unwrap();
        let parts: Vec<&str> = j.reveal().split('.').collect();
        let head: Value = serde_json::from_slice(&B64.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(head, json!({"alg": "EdDSA", "kid": "local-sim-human-1", "typ": "principal+jws"}));
        let c = j.claims().unwrap();
        assert_eq!(c["type"], "builder");
        assert_eq!(c["auth"], json!({"level": "step_up", "at": iso_z(1_000_000), "simulated": true}));
        assert_eq!(c["exp"], iso_z(1_000_060));
        let a = &c["attrs"];
        assert_eq!(
            (a["operation"].as_str(), a["candidate_hash"].as_str(), a["expected_revision"].as_str(), a["proposal_id"].as_str()),
            (Some("approve"), Some("ab".repeat(32).as_str()), Some("3"), Some("prop-1"))
        );
        let vk = SigningKey::from_bytes(&SEED).verifying_key();
        let sig = Signature::from_slice(&B64.decode(parts[2]).unwrap()).unwrap();
        vk.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig).unwrap();
    }

    #[test]
    fn binding_digest_is_recomputable_from_the_attrs_like_the_python_issuer() {
        let c = auth().authorize_at("publish", &tgt(), 5).unwrap().claims().unwrap();
        let a = &c["attrs"];
        let b = json!({"tenant_id": "t1", "actor_ref": "local-supervisor", "command_ref": a["command_ref"], "operation": "publish",
            "challenge_ref": a["challenge_ref"], "target": {"kind": "proposal", "proposal_id": "prop-1", "candidate_hash": "ab".repeat(32), "expected_revision": 3}});
        assert_eq!(a["binding_digest"].as_str().unwrap(), binding_digest(&b));
    }

    #[test]
    fn every_authorization_is_a_fresh_single_use_intention() {
        let (x, y) = (auth().authorize_at("approve", &tgt(), 5).unwrap(), auth().authorize_at("approve", &tgt(), 5).unwrap());
        assert_ne!(x.reveal(), y.reveal());
        assert_ne!(x.fingerprint(), y.fingerprint());
    }

    #[test]
    fn unsupported_operation_and_malformed_target_are_refused_before_signing() {
        assert_eq!(auth().authorize_at("revoke", &tgt(), 5), Err(AuthError::UnsupportedOperation("revoke".into())));
        let mut t = tgt();
        t.candidate_hash = "XYZ".into();
        assert!(matches!(auth().authorize_at("approve", &t, 5), Err(AuthError::InvalidTarget(_))));
    }

    #[test]
    fn debug_never_prints_key_material_or_the_token() {
        let a = auth();
        let j = a.authorize_at("approve", &tgt(), 5).unwrap();
        let shown = format!("{a:?} {j:?} {:?}", (&j, &a));
        let seed_hex: String = SEED.iter().map(|b| format!("{b:02x}")).collect();
        for needle in [seed_hex.as_str(), &B64.encode(SEED), j.reveal(), j.reveal().split('.').nth(2).unwrap(), "5a, 5a"] {
            assert!(!shown.contains(needle), "leaked {needle}");
        }
        assert!(shown.contains("<redacted>"));
    }

    #[test]
    fn binding_digest_matches_python_for_non_ascii_and_unsorted_keys() {
        // python: binding_digest({'tenant_id':'t\u00e9','actor_ref':'a','target':{'kind':'proposal','n':3}})
        let b = json!({"tenant_id": "t\u{e9}", "target": {"n": 3, "kind": "proposal"}, "actor_ref": "a"});
        assert_eq!(binding_digest(&b), "eae3f574b2c20efb8317779e8a8aebfa6453adfe22aab0b1726c4da54a715c41");
    }

    #[test]
    fn proposal_id_must_match_the_python_ref_pattern() {
        for bad in ["a b", "p/1", "p\u{e9}", &"x".repeat(201)] {
            let mut t = tgt();
            t.proposal_id = bad.into();
            assert!(matches!(auth().authorize_at("approve", &t, 5), Err(AuthError::InvalidTarget(_))), "{bad}");
        }
    }
}
