//! Error classifier for the closed `error_codes.wire` list of `bridge-contract/contract.json`.
use crate::generated;
use serde_json::Value;

pub use generated::{AUTH_REASONS, TABLE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Safe to repeat with the SAME idempotency key (fresh jti).
    Retry,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Auth,
    Denied,
    Invalid,
    NotFound,
    Conflict,
    Capacity,
    Unavailable,
    Internal,
    /// Status 200 "errors" (`task_in_progress`, `task_unknown`): a state, not a failure.
    Progress,
}

#[derive(Debug, Clone, Copy)]
pub struct ErrorKind {
    pub code: &'static str,
    pub kind: Kind,
    pub retryable: bool,
    pub disposition: Disposition,
    pub statuses: &'static [u16],
}

pub fn classify(code: &str) -> Option<&'static ErrorKind> {
    TABLE.iter().find(|e| e.code == code)
}

pub fn auth_reason_status(reason: &str) -> Option<u16> {
    AUTH_REASONS.iter().find(|(r, _)| *r == reason).map(|(_, s)| *s)
}

/// Fallback for responses without a parsable envelope (proxy errors, crashes).
pub fn disposition_for_status(status: u16) -> Disposition {
    match status {
        429 | 500 | 502 | 503 | 504 => Disposition::Retry,
        _ => Disposition::Terminal,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub retryable: bool,
    pub reason: Option<String>,
    pub trace_id: Option<String>,
    pub details: Value,
    /// True when `code` is in the contract table.
    pub known: bool,
}

impl ApiError {
    pub fn disposition(&self) -> Disposition {
        if self.retryable { Disposition::Retry } else { Disposition::Terminal }
    }
}

/// Parses the private error envelope (annex D.1). `None` when the body is not an envelope.
pub fn parse_envelope(status: u16, body: &[u8]) -> Option<ApiError> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let code = v.get("code")?.as_str()?.to_string();
    let known = classify(&code);
    // The contract table is authoritative for retryability; unknown codes fail closed (never retried).
    let retryable = known.map(|k| k.retryable).unwrap_or(false);
    let details = v.get("details").cloned().unwrap_or(Value::Null);
    Some(ApiError {
        status,
        reason: details.get("reason").and_then(Value::as_str).map(str::to_string),
        trace_id: v.get("trace_id").and_then(Value::as_str).map(str::to_string),
        known: known.is_some(),
        code,
        retryable,
        details,
    })
}
