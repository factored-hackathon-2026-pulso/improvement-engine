//! K3: native evaluation of a frozen proposal. The registry approves only an `evaluated` proposal, so between
//! `freeze_draft` and approve the engine must (1) get the platform to pre-authorise the task binding, (2) admit the
//! evaluation (`/evaluation/admissions`) and (3) run the `pulso-writer` stage in `evaluate_only` mode, whose
//! `pulso_writer_receipts.native_evaluation` carries the verdict. Mirrors `RealCore._evaluate` of
//! `e2e-core/src/claude_standin/core_hooks.py`. No model is involved.
//!
//! Pre-authorisation of the binding is NOT an `/internal/v1` route: the platform (control-api) issues it. Today it is the
//! e2e double's `/_e2e/config` (`BindingPreauthorizer`), the same BRG1 gap candidate as sealing the draft artifact.
use crate::admission::AdmissionRequest;
use crate::canon;
use crate::client::CoreClient;
use crate::dto::{Stage, TaskInvocation, TaskReceipt};
use crate::ops::OpError;
use crate::writer::{EvaluateOnlyCommitment, WriterInput, WRITER_AGENT};
use serde_json::Value;

/// Makes the platform issue (up front) the binding that the writer stage will present.
pub trait BindingPreauthorizer {
    fn preauthorize(&self, tenant: &str, binding_ref: &str) -> Result<(), String>;
}

/// The evaluation suite as the Core knows it: `digest` is the Core's own content hash of the suite entity
/// (`agent_core.registry.entities.content_hash`), supplied by the caller (fixture value), not recomputed here.
#[derive(Debug, Clone, PartialEq)]
pub struct SuiteRef {
    pub id: String,
    pub version: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationRun {
    pub tenant_id: String,
    pub job_id: String,
    pub logical_key: String,
    pub attempt: u32,
    pub pulso_run_ref: String,
    pub lab_grant_ref: String,
    pub writer_release_id: String,
    pub writer_agent_version: String,
    pub budget_ref: String,
    /// UTC RFC3339 `Z`, per-attempt bound of the admission.
    pub deadline: String,
}

/// The stage run of one native evaluation. A run that did not complete is a result (`native` is `None`), never an error:
/// the caller classifies `receipt` (gate failed, quota, candidate changed, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub binding_ref: String,
    pub evaluation_context_ref: String,
    pub receipt: TaskReceipt,
    pub native: Option<Value>,
}

impl Evaluation {
    pub fn verdict(&self) -> Option<&str> {
        self.native.as_ref()?.get("verdict")?.as_str()
    }
}

impl CoreClient {
    /// Admit and run the evaluate-only stage for `frozen`. `Err` only for a request that never ran or a completed stage
    /// that does not equal the commitment (extra writes, no native evaluation, another proposal).
    pub fn evaluate_frozen(
        &self,
        pre: &dyn BindingPreauthorizer,
        frozen: &crate::writer::FrozenProposal,
        suite: &SuiteRef,
        run: &EvaluationRun,
    ) -> Result<Evaluation, OpError> {
        let mut inv = TaskInvocation::new(&run.tenant_id, &run.job_id, Stage::Writer, &run.logical_key);
        inv.attempt = run.attempt;
        inv.agent_id = WRITER_AGENT.into();
        inv.agent_version = run.writer_agent_version.clone();
        inv.release_id = run.writer_release_id.clone();
        inv.pulso_run_ref = run.pulso_run_ref.clone();
        inv.lab_grant_ref = run.lab_grant_ref.clone();
        let key = inv.idempotency_key()?;
        let binding_ref = canon::task_binding_ref(&run.tenant_id, &key)?;
        let adm = AdmissionRequest::new(&binding_ref, &frozen.proposal_id, &frozen.candidate_hash, &suite.id, &suite.version, &suite.digest, run.attempt, &run.budget_ref, &run.deadline);
        adm.validate().map_err(OpError::Invalid)?;
        let ctx = adm.derived_context_ref(&run.tenant_id, &run.job_id)?;
        pre.preauthorize(&run.tenant_id, &binding_ref).map_err(|e| OpError::Invalid(format!("pre-authorising the binding: {e}")))?;
        self.admit_evaluation(&run.tenant_id, &run.job_id, &adm)?;
        inv.input = WriterInput {
            draft_plan_ref: frozen.plan_ref.clone(),
            base_release_id: frozen.base_release_id.clone(),
            evaluate_enabled: true,
            proposal_id: Some(frozen.proposal_id.clone()),
            evaluation_suite: Some((suite.id.clone(), suite.version.clone())),
        }
        .to_json();
        inv.registry_mutation_commitment = Some(
            EvaluateOnlyCommitment { base_release_id: frozen.base_release_id.clone(), proposal_id: frozen.proposal_id.clone(), evaluation_context_ref: ctx.clone() }.to_json(),
        );
        let receipt = self.invoke(&inv)?;
        let native = if receipt.is_success() { Some(verify_stage(&receipt, &frozen.proposal_id)?) } else { None };
        Ok(Evaluation { binding_ref, evaluation_context_ref: ctx, receipt, native })
    }
}

/// A completed evaluate-only stage must have written exactly one verified `evaluate` and report a native evaluation.
fn verify_stage(receipt: &TaskReceipt, proposal_id: &str) -> Result<Value, OpError> {
    let bad = |why: String| Err(OpError::CommitmentMismatch(why));
    let fact = receipt.fact("pulso_writer_receipts").ok_or_else(|| OpError::CommitmentMismatch("no pulso_writer_receipts fact".into()))?;
    let wr = crate::writer::WriterReceipts::from_fact(fact)?;
    let got: Vec<&str> = wr.write_receipts.iter().map(|r| r.op.as_str()).collect();
    if got != ["evaluate"] {
        return bad(format!("an evaluate-only stage must write exactly [\"evaluate\"], wrote {got:?}"));
    }
    if wr.write_receipts.iter().any(|r| !r.verified) {
        return bad("the evaluate write is not verified".into());
    }
    if wr.proposal_id.as_deref() != Some(proposal_id) {
        return bad(format!("the stage evaluated {:?}, not {proposal_id:?}", wr.proposal_id));
    }
    match wr.native_evaluation {
        Some(n) if n.get("verdict").and_then(Value::as_str).is_some() => Ok(n),
        _ => bad("the completed stage reports no native evaluation verdict".into()),
    }
}
