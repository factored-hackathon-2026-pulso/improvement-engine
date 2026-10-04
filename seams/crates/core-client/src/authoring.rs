//! Authoring DTOs: dry-run (`CoreAuthoringDryRun*`), alias read (`AliasState`) and credential issue.
use crate::canon;
use crate::dto::{DecodeError, err, nullable_str, obj, opt_arr, opt_str, req_str};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;

// ---------------------------------------------------------------------------------------------------------------
// Dry-run
// ---------------------------------------------------------------------------------------------------------------

/// One `EntityDraft` change: `kind`, `content` (object with text `id` and `version`) and `docs` (object).
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub kind: String,
    pub content: Value,
    pub docs: Value,
}

impl Change {
    pub fn new(kind: &str, content: Value, docs: Value) -> Change {
        Change { kind: kind.into(), content, docs }
    }
}

/// `CoreAuthoringDryRunRequest`. `request_digest` is not sent: the answer's digest is checked against the digest of
/// the body we sent (`canon::request_digest`), which is the binding the stand-in (`RealCore`) enforces.
#[derive(Debug, Clone, PartialEq)]
pub struct DryRunRequest {
    pub tenant_id: String,
    pub agent_id: String,
    /// Always serialised (null when `None`): the schema requires the key.
    pub base_release_id: Option<String>,
    pub changes: Vec<Change>,
}

impl DryRunRequest {
    pub fn new(tenant_id: &str, agent_id: &str, base_release_id: Option<&str>, changes: Vec<Change>) -> DryRunRequest {
        DryRunRequest { tenant_id: tenant_id.into(), agent_id: agent_id.into(), base_release_id: base_release_id.map(str::to_string), changes }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.tenant_id.is_empty() || crate::dto::cp_len(&self.tenant_id) > 255 {
            return Err("tenant_id must be 1..=255 chars".into());
        }
        if self.agent_id.is_empty() || crate::dto::cp_len(&self.agent_id) > 255 {
            return Err("agent_id must be 1..=255 chars".into());
        }
        if self.base_release_id.as_deref().is_some_and(|b| crate::dto::cp_len(b) > 255) {
            return Err("base_release_id is capped at 255 chars".into());
        }
        for (i, c) in self.changes.iter().enumerate() {
            if c.kind.is_empty() || crate::dto::cp_len(&c.kind) > 64 {
                return Err(format!("changes[{i}].kind must be 1..=64 chars"));
            }
            for (n, v) in [("content", &c.content), ("docs", &c.docs)] {
                if !v.is_object() {
                    return Err(format!("changes[{i}].{n} must be a JSON object"));
                }
                canon::jcs(v).map_err(|e| format!("changes[{i}].{n}: {e}"))?;
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("schema_version".into(), Value::String("1".into()));
        m.insert("tenant_id".into(), Value::String(self.tenant_id.clone()));
        m.insert("agent_id".into(), Value::String(self.agent_id.clone()));
        m.insert("base_release_id".into(), self.base_release_id.clone().map_or(Value::Null, Value::String));
        m.insert(
            "changes".into(),
            Value::Array(
                self.changes
                    .iter()
                    .map(|c| serde_json::json!({"kind": c.kind, "content": c.content, "docs": c.docs}))
                    .collect(),
            ),
        );
        Value::Object(m)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub rule: String,
    pub message: String,
    pub flow: Option<String>,
    pub node_id: Option<String>,
    pub path: Option<String>,
}

/// Dry-run result. HTTP 200 with non-empty `violations` (or `valid: false`) is NOT success: use `is_valid()` or the
/// strict `CoreClient::dry_run`.
#[derive(Debug, Clone, PartialEq)]
pub struct DryRunResult {
    pub valid: Option<bool>,
    pub violations: Vec<Violation>,
    pub candidate_hash: Option<String>,
    pub release_hash: Option<String>,
    pub release_id_preview: Option<String>,
    pub request_digest: Option<String>,
    pub proposal_created: Option<bool>,
    pub content_hashes: BTreeMap<String, String>,
    pub auto_bumped: Vec<Value>,
    pub new_versions: Vec<Value>,
    pub runtime_profile: Option<String>,
    pub raw: Value,
}

impl DryRunResult {
    pub fn from_json(body: &Value) -> Result<DryRunResult, DecodeError> {
        const W: &str = "CoreAuthoringDryRun";
        let m = obj(W, body)?;
        if m.get("schema_version").and_then(Value::as_str) != Some("1") {
            return err(W, "schema_version is not \"1\"");
        }
        let mut violations = Vec::new();
        for v in m.get("violations").and_then(Value::as_array).ok_or_else(|| DecodeError(format!("{W}: missing violations")))? {
            let vm = obj("violation", v)?;
            violations.push(Violation {
                rule: req_str("violation", vm, "rule")?,
                message: req_str("violation", vm, "message")?,
                flow: opt_str(vm, "flow"),
                node_id: opt_str(vm, "node_id"),
                path: opt_str(vm, "path"),
            });
        }
        let mut content_hashes = BTreeMap::new();
        if let Some(Value::Object(h)) = m.get("content_hashes") {
            for (k, v) in h {
                content_hashes.insert(k.clone(), v.as_str().unwrap_or_default().to_string());
            }
        }
        Ok(DryRunResult {
            valid: m.get("valid").and_then(Value::as_bool),
            violations,
            candidate_hash: nullable_str(W, m, "candidate_hash")?,
            release_hash: opt_str(m, "release_hash"),
            release_id_preview: opt_str(m, "release_id_preview"),
            request_digest: opt_str(m, "request_digest"),
            proposal_created: m.get("proposal_created").and_then(Value::as_bool),
            content_hashes,
            auto_bumped: opt_arr(m, "auto_bumped"),
            new_versions: opt_arr(m, "new_versions"),
            runtime_profile: opt_str(m, "runtime_profile"),
            raw: body.clone(),
        })
    }

    /// `valid` and no violations and a candidate hash.
    pub fn is_valid(&self) -> bool {
        self.valid == Some(true) && self.violations.is_empty() && self.candidate_hash.is_some()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Alias read
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alias {
    Prod,
    Staging,
}

impl Alias {
    pub fn as_str(self) -> &'static str {
        match self {
            Alias::Prod => "prod",
            Alias::Staging => "staging",
        }
    }
    pub fn parse(s: &str) -> Option<Alias> {
        match s {
            "prod" => Some(Alias::Prod),
            "staging" => Some(Alias::Staging),
            _ => None,
        }
    }
}

/// `AliasState`: read-only observation of an alias (there is no write path).
#[derive(Debug, Clone, PartialEq)]
pub struct AliasState {
    pub agent_id: String,
    pub alias: Alias,
    pub release_id: Option<String>,
    pub observed_at: Option<String>,
    pub runtime_profile: Option<String>,
    pub source: Option<String>,
    pub status: Option<String>,
    pub raw: Value,
}

impl AliasState {
    pub fn from_json(body: &Value) -> Result<AliasState, DecodeError> {
        const W: &str = "AliasState";
        let m = obj(W, body)?;
        let alias_s = req_str(W, m, "alias")?;
        Ok(AliasState {
            agent_id: req_str(W, m, "agent_id")?,
            alias: Alias::parse(&alias_s).ok_or_else(|| DecodeError(format!("{W}: unknown alias {alias_s:?}")))?,
            release_id: nullable_str(W, m, "release_id")?,
            observed_at: opt_str(m, "observed_at"),
            runtime_profile: opt_str(m, "runtime_profile"),
            source: opt_str(m, "source"),
            status: opt_str(m, "status"),
            raw: body.clone(),
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Credential issue
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct CredentialRequest {
    pub tenant_id: String,
    pub role: String,
    pub purpose: String,
}

impl CredentialRequest {
    pub fn new(tenant_id: &str, role: &str, purpose: &str) -> CredentialRequest {
        CredentialRequest { tenant_id: tenant_id.into(), role: role.into(), purpose: purpose.into() }
    }

    pub fn validate(&self) -> Result<(), String> {
        for (n, v, max) in [("tenant_id", &self.tenant_id, 128), ("role", &self.role, 64), ("purpose", &self.purpose, 64)] {
            if v.is_empty() || crate::dto::cp_len(v) > max {
                return Err(format!("{n} must be 1..={max} chars"));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        serde_json::json!({"tenant_id": self.tenant_id, "role": self.role, "purpose": self.purpose})
    }
}

/// A short-lived principal JWS (TTL <= 900 s, `Cache-Control: no-store`). Never persist or log it: `Debug` redacts.
#[derive(Clone, PartialEq)]
pub struct CredentialIssue {
    jws: String,
    pub kid: String,
    exp: Value,
}

impl CredentialIssue {
    pub fn from_json(body: &Value) -> Result<CredentialIssue, DecodeError> {
        const W: &str = "CoreCredentialIssue";
        let m = obj(W, body)?;
        Ok(CredentialIssue {
            jws: req_str(W, m, "jws")?,
            kid: req_str(W, m, "kid")?,
            exp: m.get("exp").cloned().ok_or_else(|| DecodeError(format!("{W}: missing exp")))?,
        })
    }

    /// The secret. Pass it straight to the caller that needs it; do not store it.
    pub fn jws(&self) -> &str {
        &self.jws
    }

    /// Unix expiry; `None` when the bridge sent a non-integer (only golden placeholders do).
    pub fn exp(&self) -> Option<i64> {
        self.exp.as_i64()
    }
}

impl fmt::Debug for CredentialIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CredentialIssue {{ jws: <redacted>, kid: {:?}, exp: {} }}", self.kid, self.exp)
    }
}
