//! K5a reconcile (stub: naive behaviour, RED).
use crate::client::CoreClient;
use crate::dto::{ArmReport, ArmRequest};
use crate::ops::OpError;
use crate::routes;

impl CoreClient {
    pub fn run_arm_reconciled(&self, tenant: &str, job_id: Option<&str>, req: &ArmRequest) -> Result<ArmReport, OpError> {
        let once = |r: &ArmRequest| self.op(&routes::RUN_ARM, tenant, job_id, &[], Some(&r.to_json()), Some(&r.idempotency_key), 1);
        let r = match once(req) {
            Err(OpError::Call(_)) => {
                let mut again = req.clone();
                again.idempotency_key = format!("{}-retry", req.idempotency_key);
                once(&again)?
            }
            other => other?,
        };
        Ok(ArmReport::from_json(&r.body)?)
    }
}
