//! U13 autonomous Scout: turns sealed, treated evidence into *candidate*
//! drafts. It cannot publish a detector, proposal, tool, or platform effect.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::core_task::{CoreTaskOutcome, CoreTaskReceipt, CoreTaskScope};
use crate::deterministic_sensor::DeterministicSignal;
use crate::model_provider::{ModelOutcome, ModelReceipt};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum CandidateKind {
    Signal,
    Claim,
    Opportunity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScoutCandidateDraft {
    pub kind: CandidateKind,
    pub candidate_id: String,
    pub metric_id: String,
    /// Immutable U08 snapshot that supplied the sealed query receipts.
    pub source_snapshot_ref: crate::ArtifactReference,
    /// The scope is retained verbatim so a candidate cannot be replayed into
    /// another tenant, job, grant, or authority context.
    pub tenant_id: String,
    pub job_id: String,
    pub grant_id: String,
    pub authority_ref: String,
    pub source_data_digest: String,
    pub source_contract_digest: String,
    pub transform_digest: String,
    pub cutoff_unix_seconds: u64,
    pub query_receipt_digests: Vec<String>,
    pub signal_commitment: String,
    pub core_input_commitment: String,
    pub model_input_commitment: String,
    pub model_capability_digest: String,
    pub core_binding_digest: String,
    pub core_attempt_id: String,
    pub core_run_id: Option<String>,
    pub core_output_digest: Option<String>,
    pub model_policy_digest: String,
    pub model_attempt_id: String,
    pub model_receipt_evidence: String,
    pub model_output_digest: Option<String>,
    /// Full canonical SHA-256 commitment to all persisted provenance inputs.
    pub provenance_commitment: String,
    pub digest: String,
}

#[derive(Serialize)]
struct CandidateProvenance<'a> {
    tenant_id: &'a str,
    job_id: &'a str,
    grant_id: &'a str,
    authority_ref: &'a str,
    signal: &'a DeterministicSignal,
    core_binding_digest: &'a str,
    core_attempt_id: &'a str,
    core_run_id: Option<&'a str>,
    core_output_digest: Option<&'a str>,
    model_policy_digest: &'a str,
    model_capability_digest: &'a str,
    model_input_commitment: &'a str,
    model_attempt_id: &'a str,
    model_evidence: &'a str,
    model_output_digest: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScoutResult {
    Candidates(Vec<ScoutCandidateDraft>),
    DependencyBlocked { reason: &'static str },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScoutError {
    EvidenceDenied,
    ScopeMismatch,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScoutInvocationExpectation {
    scope: CoreTaskScope,
    signal_digest: String,
    core_binding_digest: String,
    core_attempt_id: String,
    core_run_id: Option<String>,
    core_output_digest: Option<String>,
    model_policy_digest: String,
    model_capability_digest: String,
    model_input_commitment: String,
    model_attempt_id: String,
    model_evidence: String,
    model_output_digest: Option<String>,
}

/// Boundary owned by the improvement-control plane.  It issues an opaque
/// expectation only after it has authenticated the U08/U09/U10 receipts. The
/// Scout deliberately accepts no caller-provided expected strings.
pub trait ScoutInvocationAuthority {
    fn seal(
        &mut self,
        scope: &CoreTaskScope,
        signal: &DeterministicSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<ScoutInvocationExpectation, ScoutError>;
}

/// Explicitly non-production composition used only by the local test harness.
/// Production must provide the control-plane authority through the port above.
#[cfg(feature = "test-support")]
pub struct NonProductionScoutInvocationAuthority;

#[cfg(feature = "test-support")]
impl ScoutInvocationAuthority for NonProductionScoutInvocationAuthority {
    fn seal(
        &mut self,
        scope: &CoreTaskScope,
        signal: &DeterministicSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<ScoutInvocationExpectation, ScoutError> {
        if core.scope() != scope
            || model.scope() != scope
            || !signal.has_valid_digest()
            || signal.tenant_id != scope.tenant_id()
            || signal.grant_id != scope.grant_id()
            || signal.authority_ref != scope.authority_ref()
        {
            return Err(ScoutError::EvidenceDenied);
        }
        Ok(ScoutInvocationExpectation {
            scope: scope.clone(),
            signal_digest: signal.digest.clone(),
            core_binding_digest: core.binding_digest().into(),
            core_attempt_id: core.attempt_id().into(),
            core_run_id: core.core_run_id().map(str::to_owned),
            core_output_digest: core.output_digest().map(str::to_owned),
            model_policy_digest: model.policy_digest().into(),
            model_capability_digest: model.capability_digest().into(),
            model_input_commitment: model.input_commitment().into(),
            model_attempt_id: model.attempt_id().into(),
            model_evidence: model.evidence().into(),
            model_output_digest: model.output_digest().map(str::to_owned),
        })
    }
}

#[cfg(feature = "test-support")]
pub fn sealed_expectation_for_test(
    scope: CoreTaskScope,
    signal: &DeterministicSignal,
    core: &CoreTaskReceipt,
    model: &ModelReceipt,
) -> ScoutInvocationExpectation {
    NonProductionScoutInvocationAuthority
        .seal(&scope, signal, core, model)
        .expect("test fixture is valid")
}

/// Pure, deterministic publication gate. U09/U10 receipts are supplied by
/// their owners; model output is never copied into a candidate or treated as
/// evidence of causality.
pub struct AutonomousScout;
impl AutonomousScout {
    pub fn discover(
        scope: &CoreTaskScope,
        expectation: &ScoutInvocationExpectation,
        signal: &DeterministicSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<ScoutResult, ScoutError> {
        if !signal.has_valid_digest()
            || signal.query_receipts.is_empty()
            || signal.query_receipts.iter().any(|r| {
                !r.has_valid_digest()
                    || r.tenant_id != signal.tenant_id
                    || r.grant_id != signal.grant_id
                    || r.authority_ref != signal.authority_ref
                    || r.source_digest != signal.source_digest
                    || r.source_contract_digest != signal.source_contract_digest
                    || r.transform_digest != signal.transform_digest
                    || r.cutoff_unix_seconds != signal.cutoff_unix_seconds
            })
        {
            return Err(ScoutError::EvidenceDenied);
        }
        if &expectation.scope != scope
            || core.scope() != scope
            || model.scope() != scope
            || signal.tenant_id != scope.tenant_id()
            || signal.grant_id != scope.grant_id()
            || signal.authority_ref != scope.authority_ref()
        {
            return Err(ScoutError::ScopeMismatch);
        }
        if expectation.signal_digest != signal.digest
            || core.input_digest() != signal.digest
            || core.binding_digest() != expectation.core_binding_digest
            || core.attempt_id() != expectation.core_attempt_id
            || core.core_run_id() != expectation.core_run_id.as_deref()
            || core.output_digest() != expectation.core_output_digest.as_deref()
            || model.input_commitment().is_empty()
            || model.policy_digest() != expectation.model_policy_digest
            || model.capability_digest() != expectation.model_capability_digest
            || model.input_commitment() != expectation.model_input_commitment
            || model.attempt_id() != expectation.model_attempt_id
            || model.evidence().is_empty()
            || model.evidence() != expectation.model_evidence
            || model.output_digest() != expectation.model_output_digest.as_deref()
        {
            return Err(ScoutError::EvidenceDenied);
        }
        if core.outcome() != &CoreTaskOutcome::Succeeded {
            return Ok(ScoutResult::DependencyBlocked {
                reason: "core_task_unknown",
            });
        }
        if model.outcome() != &ModelOutcome::Succeeded {
            return Ok(ScoutResult::DependencyBlocked {
                reason: "model_dependency_unavailable",
            });
        }
        let receipts = signal
            .query_receipts
            .iter()
            .map(|r| r.digest.clone())
            .collect::<Vec<_>>();
        let provenance_commitment = digest(&CandidateProvenance {
            tenant_id: scope.tenant_id(),
            job_id: scope.job_id(),
            grant_id: scope.grant_id(),
            authority_ref: scope.authority_ref(),
            signal,
            core_binding_digest: core.binding_digest(),
            core_attempt_id: core.attempt_id(),
            core_run_id: core.core_run_id(),
            core_output_digest: core.output_digest(),
            model_policy_digest: model.policy_digest(),
            model_capability_digest: model.capability_digest(),
            model_input_commitment: model.input_commitment(),
            model_attempt_id: model.attempt_id(),
            model_evidence: model.evidence(),
            model_output_digest: model.output_digest(),
        });
        let mut output = Vec::new();
        for kind in [
            CandidateKind::Signal,
            CandidateKind::Claim,
            CandidateKind::Opportunity,
        ] {
            let mut draft = ScoutCandidateDraft {
                kind,
                candidate_id: format!(
                    "candidate_{}_{}_{}",
                    signal.metric_id,
                    candidate_name(kind),
                    provenance_commitment
                ),
                metric_id: signal.metric_id.clone(),
                source_snapshot_ref: signal.source_snapshot_ref.clone(),
                tenant_id: scope.tenant_id().into(),
                job_id: scope.job_id().into(),
                grant_id: scope.grant_id().into(),
                authority_ref: scope.authority_ref().into(),
                source_data_digest: signal.source_digest.clone(),
                source_contract_digest: signal.source_contract_digest.clone(),
                transform_digest: signal.transform_digest.clone(),
                cutoff_unix_seconds: signal.cutoff_unix_seconds,
                query_receipt_digests: receipts.clone(),
                signal_commitment: signal.digest.clone(),
                core_input_commitment: core.input_digest().to_owned(),
                model_input_commitment: model.input_commitment().to_owned(),
                model_capability_digest: model.capability_digest().to_owned(),
                core_binding_digest: core.binding_digest().to_owned(),
                core_attempt_id: core.attempt_id().to_owned(),
                core_run_id: core.core_run_id().map(str::to_owned),
                core_output_digest: core.output_digest().map(str::to_owned),
                model_policy_digest: model.policy_digest().to_owned(),
                model_attempt_id: model.attempt_id().to_owned(),
                model_receipt_evidence: model.evidence().to_owned(),
                model_output_digest: model.output_digest().map(str::to_owned),
                provenance_commitment: provenance_commitment.clone(),
                digest: String::new(),
            };
            draft.digest = digest(&draft);
            output.push(draft);
        }
        Ok(ScoutResult::Candidates(output))
    }
}
fn candidate_name(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Signal => "signal",
        CandidateKind::Claim => "claim",
        CandidateKind::Opportunity => "opportunity",
    }
}
fn digest<T: Serialize>(value: &T) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("candidate is serializable"))
    )
}
