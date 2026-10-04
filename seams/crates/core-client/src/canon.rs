//! Wire formats and digests of the bridge contract (`contract.json` idempotency + annex D.1):
//! JCS (RFC 8785) canonical JSON, lowercase-hex SHA-256, key/ref derivations, Z-RFC3339 timestamps.
//! Independent re-implementation of `e2e-core/src/codex_standin/dto.py`; tests cross-check the
//! concrete values of the bridge goldens.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonError {
    /// The wire carries integers only (the bridge refuses non-integer numbers).
    NonIntegerNumber,
    /// `|` joins the derivation components; it is outside the key charset and is rejected in any component.
    PipeInComponent(&'static str),
}

impl fmt::Display for CanonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonError::NonIntegerNumber => write!(f, "non-integer JSON number"),
            CanonError::PipeInComponent(c) => write!(f, "`|` is not allowed in {c}"),
        }
    }
}

impl std::error::Error for CanonError {}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn jcs_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{9}' => out.push_str("\\t"),
            '\u{a}' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\u{d}' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn jcs_into(v: &Value, out: &mut String) -> Result<(), CanonError> {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                return Err(CanonError::NonIntegerNumber);
            }
        }
        Value::String(s) => jcs_string(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                jcs_into(x, out)?;
            }
            out.push(']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            // RFC 8785 3.2.3: sort by UTF-16 code units, not UTF-8 bytes.
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                jcs_string(k, out);
                out.push(':');
                jcs_into(&m[k], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// RFC 8785 canonical JSON of `v`.
pub fn jcs(v: &Value) -> Result<String, CanonError> {
    let mut out = String::new();
    jcs_into(v, &mut out)?;
    Ok(out)
}

/// `sha256_hex(JCS(value))`.
pub fn digest_json(v: &Value) -> Result<String, CanonError> {
    Ok(sha256_hex(jcs(v)?.as_bytes()))
}

/// Top-level keys that never enter a request digest.
pub const DIGEST_EXCLUDED: [&str; 3] = ["request_digest", "credentials", "trace"];

/// `sha256_hex(JCS(body minus request_digest, credentials, trace))` (contract.json `idempotency.invoke`).
pub fn request_digest(body: &Value) -> Result<String, CanonError> {
    match body {
        Value::Object(m) => {
            let kept = m.iter().filter(|(k, _)| !DIGEST_EXCLUDED.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
            digest_json(&Value::Object(kept))
        }
        other => digest_json(other),
    }
}

fn joined(parts: &[(&'static str, &str)]) -> Result<String, CanonError> {
    for (name, v) in parts {
        if v.contains('|') {
            return Err(CanonError::PipeInComponent(name));
        }
    }
    Ok(parts.iter().map(|(_, v)| *v).collect::<Vec<_>>().join("|"))
}

/// `Idempotency-Key` of an invoke: `sha256_hex(tenant|job|stage|attempt|logical_key)`.
pub fn idempotency_key(tenant: &str, job: &str, stage: &str, attempt: u32, logical_key: &str) -> Result<String, CanonError> {
    let a = attempt.to_string();
    let text = joined(&[("tenant_id", tenant), ("job_id", job), ("stage", stage), ("attempt", &a), ("logical_key", logical_key)])?;
    Ok(sha256_hex(text.as_bytes()))
}

/// `task_binding_ref = sha256_hex(tenant|idempotency_key)`.
pub fn task_binding_ref(tenant: &str, idempotency_key: &str) -> Result<String, CanonError> {
    Ok(sha256_hex(joined(&[("tenant_id", tenant), ("idempotency_key", idempotency_key)])?.as_bytes()))
}

/// Arm `execution_id = "arm-" + sha256_hex(tenant|key)[:32]`.
pub fn arm_execution_id(tenant: &str, key: &str) -> Result<String, CanonError> {
    let h = sha256_hex(joined(&[("tenant_id", tenant), ("idempotency_key", key)])?.as_bytes());
    Ok(format!("arm-{}", &h[..32]))
}

/// `evaluation_context_ref = "evc-" + sha256_hex(tenant|job|binding_ref|proposal_id|candidate_hash|attempt)[:40]`.
pub fn evaluation_context_ref(
    tenant: &str,
    job: &str,
    binding_ref: &str,
    proposal_id: &str,
    candidate_hash: &str,
    attempt: u32,
) -> Result<String, CanonError> {
    let a = attempt.to_string();
    let text = joined(&[
        ("tenant_id", tenant),
        ("job_id", job),
        ("binding_ref", binding_ref),
        ("proposal_id", proposal_id),
        ("candidate_hash", candidate_hash),
        ("evaluation_attempt", &a),
    ])?;
    Ok(format!("evc-{}", &sha256_hex(text.as_bytes())[..40]))
}

/// UTC RFC3339 with a literal `Z`, whole seconds (annex D.1).
pub fn z_timestamp(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let rem = unix_secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `z_timestamp(now + after)`.
pub fn z_deadline_after(after: Duration) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    z_timestamp(now + after.as_secs() as i64)
}

/// `^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,9})?Z$` with plausible ranges.
pub fn is_z_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 20 || b[b.len() - 1] != b'Z' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    let num = |r: std::ops::Range<usize>| s[r].parse::<u32>().unwrap_or(99);
    if !(digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10) && b[10] == b'T') {
        return false;
    }
    if !(digits(11..13) && b[13] == b':' && digits(14..16) && b[16] == b':' && digits(17..19)) {
        return false;
    }
    if !((1..=12).contains(&num(5..7)) && (1..=31).contains(&num(8..10)) && num(11..13) < 24 && num(14..16) < 60 && num(17..19) < 61) {
        return false;
    }
    match b.len() {
        20 => true,
        n if (22..=30).contains(&n) => b[19] == b'.' && digits(20..n - 1),
        _ => false,
    }
}
