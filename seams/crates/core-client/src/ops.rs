//! Typed operations over the K1 transport: one method per `/internal/v1` operation (11 routes).
use crate::canon::CanonError;
use crate::client::{CallError, CoreClient, Response};
use crate::dto::{
    Admission, AdmissionOutcome, AdmissionRequest, Alias, AliasState, ArmReport, ArmRequest, CredentialIssue, CredentialRequest,
    DecodeError, DryRunRequest, DryRunResult, TaskInvocation, TaskReceipt, Version, Violation,
};
use crate::errors::Disposition;
use crate::routes;
use serde_json::Value;
use std::fmt;

#[derive(Debug)]
pub enum OpError {
    /// Transport, auth or bridge error envelope (see `CallError`).
    Call(CallError),
    /// The 2xx body does not decode into the typed DTO.
    Decode(DecodeError),
    /// A derivation input was unusable (`|`, non-integer number); nothing was sent.
    Canon(CanonError),
    /// A request violated a contract rule client-side; nothing was sent.
    Invalid(String),
    /// The bridge answered something that contradicts the contract (wrong binding, digest not bound to our request).
    Contract(String),
    /// A dry-run answered `valid: false` or with violations: HTTP 200 but NOT success.
    DryRunRefused(Vec<Violation>),
}

impl OpError {
    pub fn disposition(&self) -> Disposition {
        match self {
            OpError::Call(c) => c.disposition(),
            _ => Disposition::Terminal,
        }
    }
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpError::Call(c) => write!(f, "call: {c:?}"),
            OpError::Decode(d) => write!(f, "{d}"),
            OpError::Canon(c) => write!(f, "{c}"),
            OpError::Invalid(s) => write!(f, "invalid request: {s}"),
            OpError::Contract(s) => write!(f, "contract violation: {s}"),
            OpError::DryRunRefused(v) => write!(f, "dry-run refused: {} violation(s): {}", v.len(), v.iter().map(|x| format!("{}: {}", x.rule, x.message)).collect::<Vec<_>>().join("; ")),
        }
    }
}

impl std::error::Error for OpError {}

impl From<CallError> for OpError {
    fn from(e: CallError) -> Self {
        OpError::Call(e)
    }
}
impl From<DecodeError> for OpError {
    fn from(e: DecodeError) -> Self {
        OpError::Decode(e)
    }
}
impl From<CanonError> for OpError {
    fn from(e: CanonError) -> Self {
        OpError::Canon(e)
    }
}

impl CoreClient {
    /// Every typed call goes through here: retries only what K1 deems safe (same key, fresh jti).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn op(
        &self,
        route: &routes::Route,
        tenant: &str,
        job_id: Option<&str>,
        path_params: &[&str],
        body: Option<&Value>,
        key: Option<&str>,
        attempts: u32,
    ) -> Result<Response, OpError> {
        Ok(self.call_with_retry(route, tenant, job_id, path_params, body, key, attempts)?)
    }

    /// `GET /version`: the bridge's pin and runtime profile. Use `Version::check_pin` to compare with ours.
    pub fn version(&self) -> Result<Version, OpError> {
        let r = self.op(&routes::VERSION, "", None, &[], None, None, self.attempts())?;
        Ok(Version::from_json(&r.body)?)
    }

    /// `POST /core-tasks/invoke`. The `Idempotency-Key` is derived from the invocation; an identical replay returns
    /// the stored run (no second Core run), another body under the same key is `pulso:digest_conflict`. A 202 is a
    /// non-terminal state, not a failure; a terminal_failed run is returned, not raised.
    pub fn invoke(&self, inv: &TaskInvocation) -> Result<TaskReceipt, OpError> {
        inv.validate().map_err(OpError::Invalid)?;
        let key = inv.idempotency_key()?;
        let body = inv.to_json();
        let r = self.op(&routes::INVOKE, &inv.tenant_id, Some(&inv.job_id), &[], Some(&body), Some(&key), self.attempts())?;
        if r.status != 200 && r.status != 202 {
            return Err(OpError::Contract(format!("invoke answered {}", r.status)));
        }
        let receipt = TaskReceipt::from_response(r.status, &r.body)?;
        let expected = crate::canon::task_binding_ref(&inv.tenant_id, &key)?;
        if receipt.task_binding_ref != expected {
            return Err(OpError::Contract(format!("task_binding_ref {} is not the binding of this request ({expected})", receipt.task_binding_ref)));
        }
        Ok(receipt)
    }

    /// `GET /core-tasks/{task_id}`.
    pub fn read_task(&self, tenant: &str, job_id: Option<&str>, task_id: &str) -> Result<TaskReceipt, OpError> {
        let r = self.op(&routes::READ_TASK, tenant, job_id, &[task_id], None, None, self.attempts())?;
        Ok(TaskReceipt::from_response(r.status, &r.body)?)
    }

    /// `POST /evaluation/arms/run`. The key rides in the `Idempotency-Key` header and in the body (equal). A replay
    /// returns the stored report; another body under the same key is `pulso:idempotency_conflict`. An in-run failure
    /// (`failed_infra`, `candidate_failed`, ...) is a decoded report, not an error.
    pub fn run_arm(&self, tenant: &str, job_id: Option<&str>, req: &ArmRequest) -> Result<ArmReport, OpError> {
        req.validate().map_err(OpError::Invalid)?;
        let expected = crate::canon::arm_execution_id(tenant, &req.idempotency_key)?;
        let body = req.to_json();
        let r = self.op(&routes::RUN_ARM, tenant, job_id, &[], Some(&body), Some(&req.idempotency_key), self.attempts())?;
        let rep = ArmReport::from_json(&r.body)?;
        if !rep.execution_id_well_formed() {
            return Err(OpError::Contract(format!("execution_id {:?} is not arm-<32 hex>", rep.execution_id)));
        }
        if !rep.execution_id.starts_with('<') && rep.execution_id != expected {
            return Err(OpError::Contract(format!("execution_id {} is not the id of this key ({expected})", rep.execution_id)));
        }
        Ok(rep)
    }

    /// `POST /evaluation/arms/{arm_id}/run` (same body and key rules as `run_arm`).
    pub fn run_arm_by_id(&self, tenant: &str, job_id: Option<&str>, arm_id: &str, req: &ArmRequest) -> Result<ArmReport, OpError> {
        req.validate().map_err(OpError::Invalid)?;
        let body = req.to_json();
        let r = self.op(&routes::RUN_ARM_BY_ID, tenant, job_id, &[arm_id], Some(&body), Some(&req.idempotency_key), self.attempts())?;
        Ok(ArmReport::from_json(&r.body)?)
    }

    /// `GET /evaluation/arms/{arm_id}`.
    pub fn read_arm(&self, tenant: &str, job_id: Option<&str>, arm_id: &str) -> Result<ArmReport, OpError> {
        let r = self.op(&routes::READ_ARM, tenant, job_id, &[arm_id], None, None, self.attempts())?;
        Ok(ArmReport::from_json(&r.body)?)
    }

    /// `GET /evaluation/arms/by-key/{key}`.
    pub fn read_arm_by_key(&self, tenant: &str, job_id: Option<&str>, key: &str) -> Result<ArmReport, OpError> {
        let r = self.op(&routes::READ_ARM_BY_KEY, tenant, job_id, &[key], None, None, self.attempts())?;
        Ok(ArmReport::from_json(&r.body)?)
    }

    /// `POST /evaluation/admissions`. 201 = first admission, 200 = identical replay (same derived ref). The bridge
    /// derives `evaluation_context_ref` from the JWT `job_id` (`job_id` here); the answer must carry that ref.
    pub fn admit_evaluation(&self, tenant: &str, job_id: &str, req: &AdmissionRequest) -> Result<AdmissionOutcome, OpError> {
        req.validate().map_err(OpError::Invalid)?;
        let derived = req.derived_context_ref(tenant, job_id)?;
        let body = req.to_json();
        let r = self.op(&routes::ADMIT_EVALUATION, tenant, Some(job_id), &[], Some(&body), None, self.attempts())?;
        let admission = Admission::from_json(&r.body)?;
        // Golden placeholders (`<...>`) can never come from a real bridge.
        if !admission.evaluation_context_ref.starts_with('<') && admission.evaluation_context_ref != derived {
            return Err(OpError::Contract(format!("evaluation_context_ref {} is not the derived {derived}", admission.evaluation_context_ref)));
        }
        Ok(AdmissionOutcome { admission, created: r.status == 201 })
    }

    /// `GET /core-state/aliases/{agent_id}/{alias}`.
    pub fn read_alias(&self, tenant: &str, job_id: Option<&str>, agent_id: &str, alias: Alias) -> Result<AliasState, OpError> {
        let r = self.op(&routes::READ_ALIAS, tenant, job_id, &[agent_id, alias.as_str()], None, None, self.attempts())?;
        let st = AliasState::from_json(&r.body)?;
        if st.agent_id != agent_id || st.alias != alias {
            return Err(OpError::Contract(format!("alias answer is for {}/{}, asked {agent_id}/{}", st.agent_id, st.alias.as_str(), alias.as_str())));
        }
        Ok(st)
    }

    /// `POST /core-authoring/dry-run` WITHOUT acceptance checks: decodes any 200 answer, including `valid: false`.
    /// Prefer `dry_run`.
    pub fn dry_run_raw(&self, job_id: &str, req: &DryRunRequest) -> Result<DryRunResult, OpError> {
        self.dry_run_as_tenant(&req.tenant_id, job_id, req)
    }

    /// Like `dry_run_raw` but with an explicit JWT tenant (diagnostics: provokes `pulso:tenant_mismatch`).
    pub fn dry_run_as_tenant(&self, claim_tenant: &str, job_id: &str, req: &DryRunRequest) -> Result<DryRunResult, OpError> {
        req.validate().map_err(OpError::Invalid)?;
        let body = req.to_json();
        let r = self.op(&routes::AUTHORING_DRY_RUN, claim_tenant, Some(job_id), &[], Some(&body), None, self.attempts())?;
        Ok(DryRunResult::from_json(&r.body)?)
    }

    /// `POST /core-authoring/dry-run`, strict. `Ok` guarantees: `valid`, no violations, no proposal created, the
    /// answer's `request_digest` is the digest of the body we sent, and `candidate_hash` is bare lowercase hex-64
    /// (a `sha256:` prefix is stripped). `valid: false` is `OpError::DryRunRefused`, never success.
    pub fn dry_run(&self, job_id: &str, req: &DryRunRequest) -> Result<DryRunResult, OpError> {
        let mut res = self.dry_run_raw(job_id, req)?;
        if res.valid != Some(true) || !res.violations.is_empty() {
            return Err(OpError::DryRunRefused(res.violations));
        }
        let ours = crate::canon::request_digest(&req.to_json())?;
        if res.request_digest.as_deref() != Some(ours.as_str()) {
            return Err(OpError::Contract(format!("request_digest {:?} does not match the request sent ({ours})", res.request_digest)));
        }
        if res.proposal_created != Some(false) {
            return Err(OpError::Contract("proposal_created != false (a dry-run must write nothing)".into()));
        }
        let h = res.candidate_hash.as_deref().map(|h| h.strip_prefix("sha256:").unwrap_or(h).to_string());
        match h {
            Some(h) if h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => res.candidate_hash = Some(h),
            other => return Err(OpError::Contract(format!("valid answer without a well-formed candidate_hash ({other:?})"))),
        }
        Ok(res)
    }

    /// `POST /core-credentials/issue`. Never retried: a repeat would mint a second credential.
    pub fn issue_credential(&self, job_id: &str, req: &CredentialRequest) -> Result<CredentialIssue, OpError> {
        self.issue_credential_as_tenant(&req.tenant_id, job_id, req)
    }

    /// Like `issue_credential` but with an explicit JWT tenant (diagnostics: provokes `pulso:tenant_mismatch`).
    pub fn issue_credential_as_tenant(&self, claim_tenant: &str, job_id: &str, req: &CredentialRequest) -> Result<CredentialIssue, OpError> {
        req.validate().map_err(OpError::Invalid)?;
        let body = req.to_json();
        let r = self.op(&routes::ISSUE_CREDENTIAL, claim_tenant, Some(job_id), &[], Some(&body), None, 1)?;
        Ok(CredentialIssue::from_json(&r.body)?)
    }
}
