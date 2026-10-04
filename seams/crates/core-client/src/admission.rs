//! Evaluation admission DTOs (annex D.4): `AdmissionRequest` and `Admission`.
use crate::canon;
use crate::dto::{DecodeError, obj, req_str};
use serde_json::{Map, Value};

/// Admission request. `evaluation_context_ref` is deprecated (ADR 0011): the bridge derives it, so it is never sent;
/// `derived_context_ref` computes it for the writer commitment and for checking the answer.
#[derive(Debug, Clone, PartialEq)]
pub struct AdmissionRequest {
    pub binding_ref: String,
    pub proposal_id: String,
    pub candidate_hash: String,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_digest: String,
    pub evaluation_attempt: u32,
    pub budget_ref: String,
    /// UTC RFC3339 `Z`. A per-attempt bound: excluded from the request digest, so a retry may recompute it.
    pub deadline: String,
    /// Test hook: send this digest instead of the computed one (a digest not bound to the fields conflicts).
    pub request_digest_override: Option<String>,
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl AdmissionRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        binding_ref: &str,
        proposal_id: &str,
        candidate_hash: &str,
        suite_id: &str,
        suite_version: &str,
        suite_digest: &str,
        evaluation_attempt: u32,
        budget_ref: &str,
        deadline: &str,
    ) -> AdmissionRequest {
        AdmissionRequest {
            binding_ref: binding_ref.into(),
            proposal_id: proposal_id.into(),
            candidate_hash: candidate_hash.into(),
            suite_id: suite_id.into(),
            suite_version: suite_version.into(),
            suite_digest: suite_digest.into(),
            evaluation_attempt,
            budget_ref: budget_ref.into(),
            deadline: deadline.into(),
            request_digest_override: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for (n, v, max) in [
            ("binding_ref", &self.binding_ref, 200),
            ("proposal_id", &self.proposal_id, 200),
            ("candidate_hash", &self.candidate_hash, 128),
            ("suite_id", &self.suite_id, 200),
            ("suite_version", &self.suite_version, 64),
            ("suite_digest", &self.suite_digest, 128),
            ("budget_ref", &self.budget_ref, 200),
        ] {
            if v.is_empty() || v.len() > max {
                return Err(format!("{n} must be 1..={max} chars"));
            }
        }
        if self.evaluation_attempt < 1 {
            return Err("evaluation_attempt must be >= 1".into());
        }
        if !canon::is_z_timestamp(&self.deadline) {
            return Err("deadline must be UTC RFC3339 with a literal Z".into());
        }
        if self.request_digest_override.as_deref().is_some_and(|d| !hex64(d)) {
            return Err("request_digest must be lowercase hex-64".into());
        }
        Ok(())
    }

    fn fields(&self) -> Map<String, Value> {
        let s = |v: &str| Value::String(v.into());
        let mut m = Map::new();
        m.insert("schema_version".into(), s("1"));
        m.insert("binding_ref".into(), s(&self.binding_ref));
        m.insert("proposal_id".into(), s(&self.proposal_id));
        m.insert("candidate_hash".into(), s(&self.candidate_hash));
        m.insert("suite_id".into(), s(&self.suite_id));
        m.insert("suite_version".into(), s(&self.suite_version));
        m.insert("suite_digest".into(), s(&self.suite_digest));
        m.insert("evaluation_attempt".into(), Value::from(self.evaluation_attempt));
        m.insert("budget_ref".into(), s(&self.budget_ref));
        m
    }

    /// `sha256_hex(JCS(body minus deadline and request_digest))` (the stand-in's `admission()` rule).
    pub fn computed_request_digest(&self) -> String {
        canon::digest_json(&Value::Object(self.fields())).expect("only strings and integers")
    }

    /// The exact wire body.
    pub fn to_json(&self) -> Value {
        let mut m = self.fields();
        m.insert("deadline".into(), Value::String(self.deadline.clone()));
        let digest = self.request_digest_override.clone().unwrap_or_else(|| self.computed_request_digest());
        m.insert("request_digest".into(), Value::String(digest));
        Value::Object(m)
    }

    /// `evc-` + sha256_hex(tenant|job|binding_ref|proposal_id|candidate_hash|attempt)[:40]; `job` is the JWT `job_id`.
    pub fn derived_context_ref(&self, tenant: &str, job: &str) -> Result<String, canon::CanonError> {
        canon::evaluation_context_ref(tenant, job, &self.binding_ref, &self.proposal_id, &self.candidate_hash, self.evaluation_attempt)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionState {
    Admitted,
    Consumed,
    Expired,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Admission {
    pub evaluation_context_ref: String,
    pub state: AdmissionState,
}

impl Admission {
    pub fn from_json(body: &Value) -> Result<Admission, DecodeError> {
        const W: &str = "EvaluationAdmission";
        let m = obj(W, body)?;
        let st = req_str(W, m, "state")?;
        let state = match st.as_str() {
            "admitted" => AdmissionState::Admitted,
            "consumed" => AdmissionState::Consumed,
            "expired" => AdmissionState::Expired,
            "unknown" => AdmissionState::Unknown,
            other => return Err(DecodeError(format!("{W}: unknown state {other:?}"))),
        };
        Ok(Admission { evaluation_context_ref: req_str(W, m, "evaluation_context_ref")?, state })
    }
}

/// 201 = first admission (`created`), 200 = identical replay.
#[derive(Debug, Clone, PartialEq)]
pub struct AdmissionOutcome {
    pub admission: Admission,
    pub created: bool,
}
