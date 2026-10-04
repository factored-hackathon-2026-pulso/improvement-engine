//! K5a reconcile: lost response, throttling and crash-after-write against a Core that is idempotent by key.
//!
//! Rule: a `Transport{sent:true}` is an UNKNOWN outcome, never a failure. The client never re-executes blindly:
//! it first reads the effect back by its idempotency key (`reconcile_arm`); only when the Core reports it absent
//! does it send again, always with the SAME key (a replay under the same key can never create a second effect).
//! All waiting goes through a `Clock` so tests never sleep.
use crate::client::{CallError, CoreClient};
use crate::dto::{ArmReport, ArmRequest};
use crate::errors::Disposition;
use crate::ops::OpError;
use crate::routes;
use std::time::Duration;

/// Injected time source for backoff (tests record the requested waits instead of sleeping).
pub trait Clock {
    fn sleep(&self, d: Duration);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Total HTTP attempts for the write (default 3, minimum 1).
    pub max_attempts: u32,
    pub base: Duration,
    pub cap: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy { max_attempts: 3, base: Duration::from_millis(100), cap: Duration::from_secs(5) }
    }
}

impl RetryPolicy {
    /// Exponential backoff with deterministic jitter derived from (key, attempt): up to +50%, capped.
    pub fn backoff(&self, key: &str, attempt: u32) -> Duration {
        let exp = self.base.saturating_mul(1u32 << (attempt.saturating_sub(1)).min(16));
        let h = key.bytes().fold(0xcbf29ce484222325u64 ^ u64::from(attempt), |a, b| (a ^ u64::from(b)).wrapping_mul(0x100000001b3));
        let jitter = exp.as_millis() as u64 / 2;
        let extra = if jitter == 0 { 0 } else { h % (jitter + 1) };
        (exp + Duration::from_millis(extra)).min(self.cap)
    }
}

/// Outcome of reading an effect back by key.
#[derive(Debug, Clone, PartialEq)]
pub enum Reconciled<T> {
    /// The Core holds the effect for this key: adopt it, do not re-execute.
    Found(T),
    /// The Core has no effect for this key (safe to execute with the same key).
    Absent,
}

/// Hook for resume paths. The engine executor returns `ExecError::NeedsReconciliation(i)` for a handler with
/// `effectful() == true` whose `eff/{i}` is `UnknownPendingReconciliation`. A live handler implements this trait
/// over `CoreClient::reconcile_arm` (key = the handler's stable idempotency key): `Found` -> record the output and
/// clear the effect; `Absent` -> the effect never happened, the handler may run (same key); `Err` -> stay blocked.
pub trait EffectReconciler {
    type Output;
    fn reconcile(&self, key: &str) -> Result<Reconciled<Self::Output>, OpError>;
}

fn is_absent(e: &OpError) -> bool {
    matches!(e, OpError::Call(CallError::Api(a)) if a.status == 404 && a.code == "pulso:not_found")
}

impl CoreClient {
    /// Reads an arm effect back by its idempotency key. `Absent` only on a definite `pulso:not_found`; any other
    /// failure is an error (the outcome is still unknown, stay blocked).
    pub fn reconcile_arm(&self, tenant: &str, job_id: Option<&str>, key: &str) -> Result<Reconciled<ArmReport>, OpError> {
        match self.read_arm_by_key(tenant, job_id, key) {
            Ok(r) => {
                self.check_found(tenant, key, &r)?;
                Ok(Reconciled::Found(r))
            }
            Err(e) if is_absent(&e) => Ok(Reconciled::Absent),
            Err(e) => Err(e),
        }
    }

    /// A read-back must be the run of THIS key (execution id derives from tenant+key).
    fn check_found(&self, tenant: &str, key: &str, r: &ArmReport) -> Result<(), OpError> {
        let expected = crate::canon::arm_execution_id(tenant, key)?;
        if !(self.placeholders_ok() && r.execution_id.starts_with('<')) && r.execution_id != expected {
            return Err(OpError::Contract(format!("read-back execution_id {} is not the id of this key ({expected})", r.execution_id)));
        }
        Ok(())
    }

    /// `run_arm` with reconcile semantics: bounded attempts, same key on every attempt, an unknown outcome is
    /// read back before anything is sent again.
    pub fn run_arm_reconciled(
        &self,
        tenant: &str,
        job_id: Option<&str>,
        req: &ArmRequest,
        policy: &RetryPolicy,
        clock: &dyn Clock,
    ) -> Result<ArmReport, OpError> {
        req.validate().map_err(OpError::Invalid)?;
        let key = req.idempotency_key.as_str();
        let expected = crate::canon::arm_execution_id(tenant, key)?;
        let body = req.to_json();
        let mut attempt = 1;
        loop {
            let err = match self.call(&routes::RUN_ARM, tenant, job_id, &[], Some(&body), Some(key)) {
                Ok(r) => return self.decode_arm(&expected, &r),
                Err(e) => e,
            };
            if let CallError::Transport { sent: true, .. } = &err {
                // Unknown outcome: the effect may exist. Read it back before deciding anything.
                match self.reconcile_arm(tenant, job_id, key) {
                    Ok(Reconciled::Found(r)) => {
                        let same = r.arm.as_deref().is_none_or(|a| a == req.arm)
                            && r.case_ref.as_deref().is_none_or(|c| c == req.case_ref)
                            && r.repetition.is_none_or(|n| n == i64::from(req.repetition));
                        if !same {
                            return Err(OpError::Contract("read-back run for this key has a different body (key reuse)".into()));
                        }
                        return Ok(r);
                    }
                    Ok(Reconciled::Absent) => {}
                    // Still unknown: never resend blindly.
                    Err(e) => return Err(e),
                }
            }
            let retry = matches!(err, CallError::Transport { .. }) || err.disposition() == Disposition::Retry;
            if !retry || attempt >= policy.max_attempts {
                return Err(err.into());
            }
            let wait = match &err {
                CallError::Api(a) if a.retry_after.is_some() => a.retry_after.unwrap().min(policy.cap),
                _ => policy.backoff(key, attempt),
            };
            clock.sleep(wait);
            attempt += 1;
        }
    }
}
