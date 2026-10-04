use crate::errors::{self, ApiError, Disposition};
use crate::http::{self, HttpError};
use crate::jwt::{self, JwtParams};
use crate::pins::BASE_PATH;
use crate::routes::{Idem, Route};
use ed25519_dalek::SigningKey;
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct ClientConfig {
    /// `host:port` of the bridge (plain HTTP).
    pub addr: String,
    pub kid: String,
    pub worker_id: String,
    pub signing_seed: [u8; 32],
    pub ttl_s: i64,
    pub timeout: Duration,
    /// TEST ONLY: accept golden placeholders (`<...>`) in response binding fields. A real bridge never emits them, so
    /// the default (`false`) treats one as a contract violation instead of skipping the binding check.
    pub accept_golden_placeholders: bool,
}

impl ClientConfig {
    pub fn new(addr: &str, kid: &str, signing_seed: [u8; 32], worker_id: &str) -> Self {
        ClientConfig {
            addr: addr.into(),
            kid: kid.into(),
            worker_id: worker_id.into(),
            signing_seed,
            ttl_s: 60,
            timeout: Duration::from_secs(30),
            accept_golden_placeholders: false,
        }
    }
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Value,
}

#[derive(Debug)]
pub enum CallError {
    /// A mandatory `Idempotency-Key` was not supplied; nothing was sent.
    MissingIdempotencyKey,
    /// Invalid key characters (`[A-Za-z0-9_.:-]{1,200}`); nothing was sent.
    InvalidIdempotencyKey,
    /// A path parameter is empty, a dot segment (`.`/`..`), or holds `/`, `\`, `%` or a control character; nothing was sent.
    InvalidPathParam(String),
    /// `sent=false`: connect failed, safe to retry. `sent=true`: outcome UNKNOWN (never "failed").
    Transport { sent: bool, message: String },
    /// Non-2xx with a bridge error envelope.
    Api(ApiError),
    /// Non-2xx without an envelope, or a 2xx body that is not JSON.
    Malformed { status: u16, message: String },
}

impl CallError {
    pub fn disposition(&self) -> Disposition {
        match self {
            CallError::Transport { sent: false, .. } => Disposition::Retry,
            CallError::Api(a) => a.disposition(),
            CallError::Malformed { status, .. } => errors::disposition_for_status(*status),
            _ => Disposition::Terminal,
        }
    }
}

pub struct CoreClient {
    cfg: ClientConfig,
    key: SigningKey,
    attempts: u32,
}

fn key_ok(k: &str) -> bool {
    (1..=200).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}

/// A path parameter is one URL segment: non-empty, not a dot segment, and free of separators, `%` (an encoded
/// `..` would bypass the dot check on a lenient server) and control characters.
fn path_param_ok(p: &str) -> bool {
    !p.is_empty() && p != "." && p != ".." && !p.chars().any(|c| c.is_control() || matches!(c, '/' | '\u{5c}' | '%'))
}

fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

impl CoreClient {
    pub fn new(mut cfg: ClientConfig) -> Self {
        let key = SigningKey::from_bytes(&cfg.signing_seed);
        // Do not keep a second copy of the seed around (SigningKey zeroizes itself on drop).
        for b in cfg.signing_seed.iter_mut() {
            // SAFETY-free volatile-ish wipe: black_box defeats dead-store elimination.
            *b = std::hint::black_box(0);
        }
        CoreClient { cfg, key, attempts: 3 }
    }

    /// Total attempts the typed operations make for retryable failures (default 3, minimum 1).
    pub fn with_attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    pub(crate) fn placeholders_ok(&self) -> bool {
        self.cfg.accept_golden_placeholders
    }

    pub(crate) fn attempts(&self) -> u32 {
        self.attempts
    }

    /// One HTTP attempt: fresh `jti` and token each call, same `idempotency_key` if the caller repeats it.
    pub fn call(
        &self,
        route: &Route,
        tenant_id: &str,
        job_id: Option<&str>,
        path_params: &[&str],
        body: Option<&Value>,
        idempotency_key: Option<&str>,
    ) -> Result<Response, CallError> {
        if route.idem == Idem::Required && idempotency_key.is_none() {
            return Err(CallError::MissingIdempotencyKey);
        }
        if idempotency_key.is_some_and(|k| !key_ok(k)) {
            return Err(CallError::InvalidIdempotencyKey);
        }
        for p in path_params {
            if !path_param_ok(p) {
                return Err(CallError::InvalidPathParam(format!("{p:?}")));
            }
        }
        let mut path = format!("{BASE_PATH}{}", route.path);
        for p in path_params {
            if let (Some(a), Some(b)) = (path.find('{'), path.find('}')) {
                path.replace_range(a..=b, &pct(p));
            }
        }
        let iat = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let jti = jwt::fresh_jti(&self.cfg.worker_id);
        let token = jwt::sign(
            &self.key,
            &JwtParams {
                kid: &self.cfg.kid,
                worker_id: &self.cfg.worker_id,
                purpose: route.purpose,
                tenant_id: route.tenant_required.then_some(tenant_id),
                job_id,
                iat,
                ttl_s: self.cfg.ttl_s,
                jti: &jti,
            },
        );
        let mut headers = vec![("Authorization", format!("Bearer {token}"))];
        if let Some(k) = idempotency_key {
            headers.push(("Idempotency-Key", k.to_string()));
        }
        let bytes = body.map(|b| serde_json::to_vec(b).expect("json"));
        let raw = http::request(&self.cfg.addr, route.method, &path, &headers, bytes.as_deref(), self.cfg.timeout)
            .map_err(|e| match e {
                HttpError::Connect(m) => CallError::Transport { sent: false, message: m },
                HttpError::Io(m) | HttpError::Protocol(m) => CallError::Transport { sent: true, message: m },
            })?;
        if (200..300).contains(&raw.status) {
            return serde_json::from_slice(&raw.body)
                .map(|body| Response { status: raw.status, body })
                .map_err(|e| CallError::Malformed { status: raw.status, message: e.to_string() });
        }
        Err(match errors::parse_envelope(raw.status, &raw.body) {
            Some(a) => CallError::Api(a),
            None => CallError::Malformed { status: raw.status, message: "not an error envelope".into() },
        })
    }

    /// Repeats retryable failures (same idempotency key, fresh jti) up to `max_attempts` total attempts.
    /// A `Transport{sent:true}` (unknown outcome) is repeated only when a key makes the replay safe.
    #[allow(clippy::too_many_arguments)]
    pub fn call_with_retry(
        &self,
        route: &Route,
        tenant_id: &str,
        job_id: Option<&str>,
        path_params: &[&str],
        body: Option<&Value>,
        idempotency_key: Option<&str>,
        max_attempts: u32,
    ) -> Result<Response, CallError> {
        let mut attempt = 1;
        loop {
            match self.call(route, tenant_id, job_id, path_params, body, idempotency_key) {
                Err(e) if attempt < max_attempts && Self::retryable(&e, idempotency_key.is_some()) => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(20 * u64::from(attempt)));
                }
                other => return other,
            }
        }
    }

    fn retryable(e: &CallError, has_key: bool) -> bool {
        match e {
            CallError::Transport { sent: true, .. } => has_key,
            other => other.disposition() == Disposition::Retry,
        }
    }
}
