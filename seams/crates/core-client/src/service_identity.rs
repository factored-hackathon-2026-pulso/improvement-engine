//! The engine's SERVICE identity for agent-core `serve` (registry API and export): short-lived Ed25519 credentials minted from a seed.
//!
//! Contract (agent-core `JwsIdentityVerifier` + registry `who()`): a compact JWS, header exactly `{alg: EdDSA, kid, typ: principal+jws}`,
//! payload a `Principal` (`type=builder`, `id`, `roles`, `scopes`, `attrs`, `auth{level,at}`, `exp`). `serve` verifies the signature
//! against the public key listed under `kid` in its `--staff-keys` file and refuses an expired credential (`exp <= now`). It does not
//! cap the lifetime, so the issuer picks it: the engine mints minutes, never hours, and refreshes before expiry.
//!
//! Secrets: the seed never leaves this module; `Debug` is redacting; errors name the variable, never a value.
use crate::authorizer::{Jws, iso_z};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch. A fake in tests.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0))
}

pub const DEFAULT_TTL_S: i64 = 300;
pub const MIN_TTL_S: i64 = 60;
pub const MAX_TTL_S: i64 = 900;
/// The principal `id` the registry records as `created_by`.
pub const DEFAULT_PRINCIPAL_ID: &str = "pulso-engine";
/// The one role the engine needs on `/v1/registry/*`. Never an approver role.
pub const ROLE_CONSTRUCTOR: &str = "constructor";

/// 64 hex characters -> the 32-byte Ed25519 seed. `Err` carries no part of the input.
pub fn parse_seed_hex(hex: &str) -> Result<[u8; 32], &'static str> {
    if hex.len() != 64 || !hex.is_ascii() {
        return Err("a seed is exactly 64 hex characters");
    }
    let mut s = [0u8; 32];
    for (i, b) in s.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| "a seed is exactly 64 hex characters")?;
    }
    Ok(s)
}

struct Cached {
    jws: Jws,
    exp: i64,
}

pub struct ServiceIdentity {
    kid: String,
    key: SigningKey,
    principal_id: String,
    roles: Vec<String>,
    ttl_s: i64,
    refresh_before_s: i64,
    clock: Clock,
    cache: Mutex<Option<Cached>>,
}

impl fmt::Debug for ServiceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServiceIdentity {{ kid: {:?}, id: {:?}, roles: {:?}, ttl_s: {}, key: <redacted> }}", self.kid, self.principal_id, self.roles, self.ttl_s)
    }
}

impl ServiceIdentity {
    /// `kid` must be the key id published in the Core's staff-keys file. Defaults: id `pulso-engine`, role `constructor`, 300 s.
    pub fn new(kid: &str, seed: [u8; 32]) -> ServiceIdentity {
        ServiceIdentity {
            kid: kid.into(),
            key: SigningKey::from_bytes(&seed),
            principal_id: DEFAULT_PRINCIPAL_ID.into(),
            roles: vec![ROLE_CONSTRUCTOR.into()],
            ttl_s: DEFAULT_TTL_S,
            refresh_before_s: DEFAULT_TTL_S / 5,
            clock: system_clock(),
            cache: Mutex::new(None),
        }
    }

    pub fn with_clock(mut self, clock: Clock) -> ServiceIdentity {
        self.clock = clock;
        self
    }

    pub fn with_principal_id(mut self, id: &str) -> ServiceIdentity {
        self.principal_id = id.into();
        self
    }

    pub fn with_roles(mut self, roles: &[&str]) -> ServiceIdentity {
        self.roles = roles.iter().map(|r| r.to_string()).collect();
        self
    }

    /// Lifetime of each credential, clamped to `MIN_TTL_S..=MAX_TTL_S`; a credential is replaced once less than a fifth of it remains.
    pub fn with_ttl(mut self, ttl_s: i64) -> ServiceIdentity {
        self.ttl_s = ttl_s.clamp(MIN_TTL_S, MAX_TTL_S);
        self.refresh_before_s = self.ttl_s / 5;
        self
    }

    pub fn kid(&self) -> &str {
        &self.kid
    }

    pub fn ttl_s(&self) -> i64 {
        self.ttl_s
    }

    /// The public key (base64url, 32 bytes) the Core must list under `kid` in its staff-keys file.
    pub fn public_key_b64url(&self) -> String {
        B64.encode(self.key.verifying_key().to_bytes())
    }

    /// A credential valid for at least `refresh_before_s` more seconds: the cached one, or a freshly minted one.
    pub fn credential(&self) -> Jws {
        let now = (self.clock)();
        let mut slot = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = slot.as_ref().filter(|c| c.exp - now > self.refresh_before_s) {
            return c.jws.clone();
        }
        let fresh = self.mint(now);
        let jws = fresh.jws.clone();
        *slot = Some(fresh);
        jws
    }

    /// Drops the cache and mints now (after the Core answered 401: a skewed clock or a rotated key must not wait for the expiry).
    pub fn renew(&self) -> Jws {
        let now = (self.clock)();
        let fresh = self.mint(now);
        let jws = fresh.jws.clone();
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) = Some(fresh);
        jws
    }

    fn mint(&self, now: i64) -> Cached {
        let exp = now + self.ttl_s;
        let payload = json!({
            "type": "builder", "id": self.principal_id, "roles": self.roles, "scopes": [], "attrs": {},
            "auth": {"level": "session", "at": iso_z(now)},
            "exp": iso_z(exp),
        });
        let head = B64.encode(serde_json::to_vec(&json!({"alg": "EdDSA", "kid": self.kid, "typ": "principal+jws"})).expect("json"));
        let body = B64.encode(serde_json::to_vec(&payload).expect("json"));
        let sig = self.key.sign(format!("{head}.{body}").as_bytes());
        Cached { jws: Jws::new(format!("{head}.{body}.{}", B64.encode(sig.to_bytes()))), exp }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};
    use serde_json::Value;
    use std::sync::atomic::{AtomicI64, Ordering};

    const T0: i64 = 1_800_000_000;

    fn fake() -> (Clock, Arc<AtomicI64>) {
        let t = Arc::new(AtomicI64::new(T0));
        let t2 = t.clone();
        (Arc::new(move || t2.load(Ordering::SeqCst)), t)
    }

    fn identity(clock: Clock) -> ServiceIdentity {
        ServiceIdentity::new("pulso-engine-1", [7u8; 32]).with_clock(clock)
    }

    fn parts(j: &Jws) -> (Value, Value, Vec<u8>) {
        let mut it = j.reveal().split('.');
        let (h, p, s) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        assert!(it.next().is_none());
        (serde_json::from_slice(&B64.decode(h).unwrap()).unwrap(), serde_json::from_slice(&B64.decode(p).unwrap()).unwrap(), B64.decode(s).unwrap())
    }

    #[test]
    fn credential_has_the_core_builder_principal_shape_and_verifies_with_the_public_key() {
        let (clock, _) = fake();
        let id = identity(clock);
        let j = id.credential();
        let (head, claims, sig) = parts(&j);
        assert_eq!(head, json!({"alg": "EdDSA", "kid": "pulso-engine-1", "typ": "principal+jws"}));
        assert_eq!(claims["type"], "builder");
        assert_eq!(claims["id"], "pulso-engine");
        assert_eq!(claims["roles"], json!(["constructor"]));
        assert_eq!(claims["scopes"], json!([]));
        assert_eq!(claims["attrs"], json!({}));
        assert_eq!(claims["auth"], json!({"level": "session", "at": iso_z(T0)}));
        assert_eq!(claims["exp"], iso_z(T0 + DEFAULT_TTL_S));
        let (h, p, _) = {
            let mut it = j.reveal().rsplitn(2, '.');
            let _ = it.next();
            let signing_input = it.next().unwrap().to_string();
            let (h, p) = signing_input.split_once('.').map(|(a, b)| (a.to_string(), b.to_string())).unwrap();
            (h, p, ())
        };
        let pk = ed25519_dalek::VerifyingKey::from_bytes(&SigningKey::from_bytes(&[7u8; 32]).verifying_key().to_bytes()).unwrap();
        pk.verify(format!("{h}.{p}").as_bytes(), &Signature::from_slice(&sig).unwrap()).expect("signature verifies");
        assert_eq!(id.public_key_b64url(), B64.encode(pk.to_bytes()));
    }

    #[test]
    fn never_an_approver_role() {
        let (clock, _) = fake();
        let (_, claims, _) = parts(&identity(clock).credential());
        let roles = claims["roles"].as_array().unwrap();
        assert!(roles.iter().all(|r| r == "constructor"));
    }

    #[test]
    fn a_fresh_credential_is_reused_until_the_refresh_window() {
        let (clock, t) = fake();
        let id = identity(clock);
        let a = id.credential();
        t.store(T0 + 100, Ordering::SeqCst);
        assert_eq!(id.credential(), a, "still far from expiry: same credential, no new signature");
        t.store(T0 + DEFAULT_TTL_S - DEFAULT_TTL_S / 5 - 1, Ordering::SeqCst);
        assert_eq!(id.credential(), a, "one second before the refresh window");
    }

    #[test]
    fn it_refreshes_before_expiry_never_serving_a_credential_about_to_expire() {
        let (clock, t) = fake();
        let id = identity(clock);
        let a = id.credential();
        t.store(T0 + DEFAULT_TTL_S - DEFAULT_TTL_S / 5, Ordering::SeqCst);
        let b = id.credential();
        assert_ne!(a, b);
        let (_, claims, _) = parts(&b);
        assert_eq!(claims["exp"], iso_z(T0 + DEFAULT_TTL_S - DEFAULT_TTL_S / 5 + DEFAULT_TTL_S));
        // every credential handed out has at least the refresh margin left
        for step in (0..2000).step_by(7) {
            t.store(T0 + step, Ordering::SeqCst);
            let (_, c, _) = parts(&id.credential());
            let exp = c["exp"].as_str().unwrap().to_string();
            assert!(exp > iso_z(T0 + step + DEFAULT_TTL_S / 5 - 1), "at +{step}s the credential must still have its margin");
        }
    }

    #[test]
    fn renew_mints_now_even_when_the_cache_is_fresh() {
        let (clock, t) = fake();
        let id = identity(clock);
        let a = id.credential();
        t.store(T0 + 5, Ordering::SeqCst);
        let b = id.renew();
        assert_ne!(a, b);
        assert_eq!(id.credential(), b, "the renewed credential is the cached one");
    }

    #[test]
    fn ttl_is_clamped_to_minutes() {
        let (clock, _) = fake();
        assert_eq!(identity(clock.clone()).with_ttl(10 * 3600).ttl_s(), MAX_TTL_S);
        assert_eq!(identity(clock.clone()).with_ttl(1).ttl_s(), MIN_TTL_S);
        assert_eq!(identity(clock).with_ttl(120).ttl_s(), 120);
    }

    #[test]
    fn seed_parsing_refuses_bad_input_without_echoing_it() {
        assert!(parse_seed_hex(&"ab".repeat(32)).is_ok());
        for bad in ["", "zz", &"zz".repeat(32), &"ab".repeat(31), &"é".repeat(32)] {
            let e = parse_seed_hex(bad).unwrap_err();
            assert!(!e.contains("zz") && !e.contains('é'));
        }
    }

    #[test]
    fn debug_never_shows_key_material() {
        let (clock, _) = fake();
        let id = identity(clock);
        let shown = format!("{id:?}");
        assert!(shown.contains("<redacted>") && !shown.contains("07, 07") && !shown.contains(&id.credential().reveal()[..20]));
    }
}
