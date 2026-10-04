//! Typed operations over the K1 transport: one method per `/internal/v1` operation (11 routes).
use crate::canon::CanonError;
use crate::client::{CallError, CoreClient, Response};
use crate::dto::{ArmReport, ArmRequest, DecodeError, TaskInvocation, TaskReceipt, Version};
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
}
