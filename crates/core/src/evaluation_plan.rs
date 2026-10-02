//! U20 seals typed, repository-attested evaluation inputs before execution.
//!
//! `ScenarioSet` remains the durable storage kind until the artifact schema
//! grows dedicated rows. This module does not trust that generic kind alone:
//! every selected revision must carry the shared `evaluation_contract` and is
//! re-read from the immutable repository before a plan can be frozen.

use crate::workflow_bridge::{LinkGrade, WorkflowBridgeContract};
use crate::{ArtifactKind, ArtifactReference, ArtifactRepository};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvaluationArtifactType {
    Baseline,
    Oracle,
    DevelopmentSuite,
    FinalSuite,
}

impl EvaluationArtifactType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Oracle => "oracle",
            Self::DevelopmentSuite => "development_suite",
            Self::FinalSuite => "final_suite",
        }
    }
    fn expected_partition(self) -> &'static str {
        match self {
            Self::Baseline | Self::Oracle => "shared",
            Self::DevelopmentSuite => "development",
            Self::FinalSuite => "final",
        }
    }
}

/// Typed selection of an immutable revision. Its type is independently
/// re-verified from the immutable artifact payload before use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationArtifactRef {
    reference: ArtifactReference,
    artifact_type: EvaluationArtifactType,
}

impl EvaluationArtifactRef {
    #[must_use]
    pub fn baseline(reference: ArtifactReference) -> Self {
        Self {
            reference,
            artifact_type: EvaluationArtifactType::Baseline,
        }
    }
    #[must_use]
    pub fn oracle(reference: ArtifactReference) -> Self {
        Self {
            reference,
            artifact_type: EvaluationArtifactType::Oracle,
        }
    }
    #[must_use]
    pub fn development_suite(reference: ArtifactReference) -> Self {
        Self {
            reference,
            artifact_type: EvaluationArtifactType::DevelopmentSuite,
        }
    }
    #[must_use]
    pub fn final_suite(reference: ArtifactReference) -> Self {
        Self {
            reference,
            artifact_type: EvaluationArtifactType::FinalSuite,
        }
    }
}

/// The four sealed inputs of one comparison. Development and final suites are
/// separate typed slots: final evidence cannot become iterative development input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationInputs {
    baseline: EvaluationArtifactRef,
    oracle: EvaluationArtifactRef,
    development_suite: EvaluationArtifactRef,
    final_suite: EvaluationArtifactRef,
}
impl EvaluationInputs {
    #[must_use]
    pub fn new(
        baseline: EvaluationArtifactRef,
        oracle: EvaluationArtifactRef,
        development_suite: EvaluationArtifactRef,
        final_suite: EvaluationArtifactRef,
    ) -> Self {
        Self {
            baseline,
            oracle,
            development_suite,
            final_suite,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EvaluationSemanticContract {
    target_outcome: String,
    unit_of_analysis: String,
    oracle_measure: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationPlan {
    bridge_commitment: String,
    source_snapshot_ref: ArtifactReference,
    baseline_ref: ArtifactReference,
    oracle_ref: ArtifactReference,
    development_suite_ref: ArtifactReference,
    final_suite_ref: ArtifactReference,
    semantic: EvaluationSemanticContract,
    commitment: String,
}

impl EvaluationPlan {
    /// Re-reads every immutable evaluation input, validates its sealed shared
    /// contract, and freezes only a comparison semantically identical to U16.
    pub fn seal_from_bridge<R: ArtifactRepository>(
        bridge: &WorkflowBridgeContract,
        inputs: EvaluationInputs,
        artifacts: &mut R,
    ) -> Result<Self, EvaluationPlanError> {
        if bridge.link_grade() != LinkGrade::MechanismProxy
            || !bridge.alternatives().includes_candidate_route()
        {
            return Err(EvaluationPlanError::BridgeNotEvaluable);
        }
        let scope = bridge.scope();
        let snapshot = bridge.source_snapshot_ref();
        let baseline = verified_input(artifacts, scope, snapshot, &inputs.baseline)?;
        let oracle = verified_input(artifacts, scope, snapshot, &inputs.oracle)?;
        let development_suite =
            verified_input(artifacts, scope, snapshot, &inputs.development_suite)?;
        let final_suite = verified_input(artifacts, scope, snapshot, &inputs.final_suite)?;
        let references = [
            &baseline.reference,
            &oracle.reference,
            &development_suite.reference,
            &final_suite.reference,
        ];
        if references.iter().enumerate().any(|(index, reference)| {
            references
                .iter()
                .skip(index + 1)
                .any(|other| *reference == *other)
        }) {
            return Err(EvaluationPlanError::DuplicateInputReference);
        }
        let expected = EvaluationSemanticContract {
            target_outcome: bridge.input().target_outcome().to_owned(),
            unit_of_analysis: bridge.input().unit_of_analysis().to_owned(),
            oracle_measure: bridge.input().oracle_measure().to_owned(),
        };
        if [
            &baseline.semantic,
            &oracle.semantic,
            &development_suite.semantic,
            &final_suite.semantic,
        ]
        .iter()
        .any(|semantic| **semantic != expected)
        {
            return Err(EvaluationPlanError::SemanticMismatch);
        }
        let commitment = digest(&[
            bridge.commitment(),
            &snapshot.tenant_id,
            &snapshot.id,
            &snapshot.revision.to_string(),
            &snapshot.digest,
            &baseline.reference.tenant_id,
            &baseline.reference.id,
            &baseline.reference.revision.to_string(),
            &baseline.reference.digest,
            &oracle.reference.tenant_id,
            &oracle.reference.id,
            &oracle.reference.revision.to_string(),
            &oracle.reference.digest,
            &development_suite.reference.tenant_id,
            &development_suite.reference.id,
            &development_suite.reference.revision.to_string(),
            &development_suite.reference.digest,
            &final_suite.reference.tenant_id,
            &final_suite.reference.id,
            &final_suite.reference.revision.to_string(),
            &final_suite.reference.digest,
            &expected.target_outcome,
            &expected.unit_of_analysis,
            &expected.oracle_measure,
        ]);
        Ok(Self {
            bridge_commitment: bridge.commitment().to_owned(),
            source_snapshot_ref: snapshot.clone(),
            baseline_ref: baseline.reference,
            oracle_ref: oracle.reference,
            development_suite_ref: development_suite.reference,
            final_suite_ref: final_suite.reference,
            semantic: expected,
            commitment,
        })
    }
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }
    #[must_use]
    pub fn allows_same_outcome_claim(&self) -> bool {
        false
    }
    #[must_use]
    pub fn eligible_for_proposal(&self) -> bool {
        false
    }
}

struct VerifiedInput {
    reference: ArtifactReference,
    semantic: EvaluationSemanticContract,
}

fn verified_input<R: ArtifactRepository>(
    artifacts: &mut R,
    scope: &crate::core_task::CoreTaskScope,
    snapshot: &ArtifactReference,
    typed_reference: &EvaluationArtifactRef,
) -> Result<VerifiedInput, EvaluationPlanError> {
    let reference = &typed_reference.reference;
    if reference.tenant_id != scope.tenant_id() {
        return Err(EvaluationPlanError::CrossTenantReference);
    }
    let artifact = artifacts
        .get(&reference.tenant_id, &reference.id, reference.revision)
        .map_err(|_| EvaluationPlanError::ReferenceUnavailable)?
        .ok_or(EvaluationPlanError::ReferenceUnavailable)?;
    if artifact.reference() != *reference
        || artifact.kind != ArtifactKind::ScenarioSet
        || artifact.source_snapshot_ref.as_ref() != Some(snapshot)
    {
        return Err(EvaluationPlanError::ReferenceMismatch);
    }
    let contract = artifact
        .payload
        .get("evaluation_contract")
        .and_then(Value::as_object)
        .ok_or(EvaluationPlanError::InvalidInput)?;
    if contract.get("artifact_type").and_then(Value::as_str)
        != Some(typed_reference.artifact_type.as_str())
        || contract.get("partition").and_then(Value::as_str)
            != Some(typed_reference.artifact_type.expected_partition())
    {
        return Err(EvaluationPlanError::ReferenceMismatch);
    }
    let scoped = contract
        .get("scope")
        .and_then(Value::as_object)
        .ok_or(EvaluationPlanError::ScopeMismatch)?;
    for (key, expected) in [
        ("tenant_id", scope.tenant_id()),
        ("job_id", scope.job_id()),
        ("grant_id", scope.grant_id()),
        ("authority_ref", scope.authority_ref()),
    ] {
        if scoped.get(key).and_then(Value::as_str) != Some(expected) {
            return Err(EvaluationPlanError::ScopeMismatch);
        }
    }
    Ok(VerifiedInput {
        reference: reference.clone(),
        semantic: EvaluationSemanticContract {
            target_outcome: contract_field(contract, "target_outcome")?,
            unit_of_analysis: contract_field(contract, "unit_of_analysis")?,
            oracle_measure: contract_field(contract, "oracle_measure")?,
        },
    })
}

fn contract_field(
    contract: &serde_json::Map<String, Value>,
    name: &str,
) -> Result<String, EvaluationPlanError> {
    contract
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or(EvaluationPlanError::InvalidInput)
}
fn digest(values: &[&str]) -> String {
    let mut h = Sha256::new();
    for value in values {
        h.update(value.len().to_be_bytes());
        h.update(value.as_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationPlanError {
    BridgeNotEvaluable,
    CrossTenantReference,
    ReferenceUnavailable,
    ReferenceMismatch,
    ScopeMismatch,
    SemanticMismatch,
    DuplicateInputReference,
    InvalidInput,
}
