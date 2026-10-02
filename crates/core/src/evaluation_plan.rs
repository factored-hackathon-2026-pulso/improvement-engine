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
#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};

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

/// Independent authority boundary for evaluation inputs. A coherent payload is
/// insufficient: the exact artifact revision must be attested under the U14/U16
/// grant and capability scope.
pub(crate) trait EvaluationArtifactAuthorityPort {
    fn verify_artifact(
        &mut self,
        scope: &crate::core_task::CoreTaskScope,
        reference: &ArtifactReference,
    ) -> Result<(), EvaluationAuthorityError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvaluationAuthorityError;

/// A policy-issued capability for one immutable evaluation artifact revision.
/// Real deployments resolve it through a policy adapter; the in-memory type is
/// a deterministic contract double for unit tests and local composition.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationArtifactGrant {
    scope: crate::core_task::CoreTaskScope,
    reference: ArtifactReference,
}

#[cfg(test)]
impl EvaluationArtifactGrant {
    #[must_use]
    pub fn for_scope(scope: crate::core_task::CoreTaskScope, reference: ArtifactReference) -> Self {
        Self { scope, reference }
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct InMemoryEvaluationArtifactAuthority {
    grants: BTreeMap<String, EvaluationArtifactGrant>,
    revoked: BTreeSet<String>,
}

#[cfg(test)]
impl InMemoryEvaluationArtifactAuthority {
    pub fn issue(&mut self, grant: EvaluationArtifactGrant) {
        self.grants
            .insert(authority_key(&grant.scope, &grant.reference), grant);
    }
    pub fn revoke(
        &mut self,
        scope: &crate::core_task::CoreTaskScope,
        reference: &ArtifactReference,
    ) {
        self.revoked.insert(authority_key(scope, reference));
    }
}

#[cfg(test)]
impl EvaluationArtifactAuthorityPort for InMemoryEvaluationArtifactAuthority {
    fn verify_artifact(
        &mut self,
        scope: &crate::core_task::CoreTaskScope,
        reference: &ArtifactReference,
    ) -> Result<(), EvaluationAuthorityError> {
        let key = authority_key(scope, reference);
        if self.revoked.contains(&key) {
            return Err(EvaluationAuthorityError);
        }
        let grant = self.grants.get(&key).ok_or(EvaluationAuthorityError)?;
        if grant.scope != *scope {
            return Err(EvaluationAuthorityError);
        }
        if grant.reference != *reference {
            return Err(EvaluationAuthorityError);
        }
        Ok(())
    }
}

#[cfg(test)]
fn authority_key(scope: &crate::core_task::CoreTaskScope, reference: &ArtifactReference) -> String {
    digest(&[
        scope.tenant_id(),
        scope.job_id(),
        scope.grant_id(),
        scope.authority_ref(),
        &reference.tenant_id,
        &reference.id,
        &reference.revision.to_string(),
        &reference.digest,
    ])
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

/// Opaque policy-bound composition. A caller may use an instance handed to it
/// by trusted service wiring, but cannot construct one or substitute an
/// allow-all authority implementation.
pub struct TrustedEvaluationComposer {
    authority: Box<dyn EvaluationArtifactAuthorityPort>,
}

impl TrustedEvaluationComposer {
    /// Only trusted service composition inside this crate can install a policy
    /// adapter. Keeping this constructor crate-private prevents authority
    /// injection by API consumers.
    #[allow(dead_code)] // Called by trusted service composition once its policy adapter lands.
    pub(crate) fn from_policy<A: EvaluationArtifactAuthorityPort + 'static>(authority: A) -> Self {
        Self {
            authority: Box::new(authority),
        }
    }

    pub fn seal_from_bridge<R: ArtifactRepository>(
        &mut self,
        bridge: &WorkflowBridgeContract,
        inputs: EvaluationInputs,
        artifacts: &mut R,
    ) -> Result<EvaluationPlan, EvaluationPlanError> {
        EvaluationPlan::seal_from_bridge(bridge, inputs, artifacts, self.authority.as_mut())
    }
}

/// ```compile_fail
/// use improvement_engine_core::evaluation_plan::EvaluationArtifactAuthorityPort;
/// struct AllowAll;
/// impl EvaluationArtifactAuthorityPort for AllowAll {}
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::evaluation_plan::TrustedEvaluationComposer;
/// let _ = TrustedEvaluationComposer { authority: todo!() };
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::evaluation_plan::TrustedEvaluationComposer;
/// let _ = TrustedEvaluationComposer::from_policy(());
/// ```
///
/// The authority port is deliberately private. An arbitrary consumer cannot
/// implement an allow-all policy, construct [`TrustedEvaluationComposer`], or
/// invoke its trusted-composition constructor.
const _NO_PUBLIC_AUTHORITY_INJECTION: () = ();

impl EvaluationPlan {
    /// Re-reads every immutable evaluation input, validates its sealed shared
    /// contract, and freezes only a comparison semantically identical to U16.
    fn seal_from_bridge<R: ArtifactRepository, A: EvaluationArtifactAuthorityPort + ?Sized>(
        bridge: &WorkflowBridgeContract,
        inputs: EvaluationInputs,
        artifacts: &mut R,
        authority: &mut A,
    ) -> Result<Self, EvaluationPlanError> {
        if bridge.link_grade() != LinkGrade::MechanismProxy
            || !bridge.alternatives().includes_candidate_route()
        {
            return Err(EvaluationPlanError::BridgeNotEvaluable);
        }
        let scope = bridge.scope();
        let snapshot = bridge.source_snapshot_ref();
        let baseline = verified_input(artifacts, authority, scope, snapshot, &inputs.baseline)?;
        let oracle = verified_input(artifacts, authority, scope, snapshot, &inputs.oracle)?;
        let development_suite = verified_input(
            artifacts,
            authority,
            scope,
            snapshot,
            &inputs.development_suite,
        )?;
        let final_suite =
            verified_input(artifacts, authority, scope, snapshot, &inputs.final_suite)?;
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

    /// Internal linkage check for U35. It does not grant any outcome or
    /// release claim; it merely prevents a plan sealed for one bridge from
    /// being reused with another bridge.
    pub(crate) fn is_bound_to_bridge(&self, bridge: &WorkflowBridgeContract) -> bool {
        self.bridge_commitment == bridge.commitment()
            && self.source_snapshot_ref == *bridge.source_snapshot_ref()
            && self.baseline_ref.tenant_id == bridge.scope().tenant_id()
            && self.oracle_ref.tenant_id == bridge.scope().tenant_id()
            && self.development_suite_ref.tenant_id == bridge.scope().tenant_id()
            && self.final_suite_ref.tenant_id == bridge.scope().tenant_id()
    }
}

struct VerifiedInput {
    reference: ArtifactReference,
    semantic: EvaluationSemanticContract,
}

fn verified_input<R: ArtifactRepository, A: EvaluationArtifactAuthorityPort + ?Sized>(
    artifacts: &mut R,
    authority: &mut A,
    scope: &crate::core_task::CoreTaskScope,
    snapshot: &ArtifactReference,
    typed_reference: &EvaluationArtifactRef,
) -> Result<VerifiedInput, EvaluationPlanError> {
    let reference = &typed_reference.reference;
    if reference.tenant_id != scope.tenant_id() {
        return Err(EvaluationPlanError::CrossTenantReference);
    }
    authority
        .verify_artifact(scope, reference)
        .map_err(|_| EvaluationPlanError::AuthorityDenied)?;
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
    AuthorityDenied,
    InvalidInput,
}

#[cfg(test)]
mod tests {
    use super::{
        EvaluationArtifactGrant, EvaluationArtifactRef, EvaluationInputs, EvaluationPlanError,
        InMemoryEvaluationArtifactAuthority, TrustedEvaluationComposer,
    };
    use crate::independent_verifier::VerificationStatus;
    use crate::workflow_bridge::bridge_for_evaluation_plan_test;
    use crate::{
        ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository,
        InMemoryArtifactRepository,
    };
    use serde_json::json;

    fn artifact(
        repo: &mut InMemoryArtifactRepository,
        id: &str,
        artifact_type: &str,
        partition: &str,
        snapshot: ArtifactReference,
    ) -> ArtifactReference {
        repo.append(None, ArtifactDraft::new(
            "tenant_a", id, 1, ArtifactKind::ScenarioSet,
            json!({"evaluation_contract": {
                "artifact_type": artifact_type, "partition": partition,
                "target_outcome": "reduce_repeat_payment_contacts",
                "unit_of_analysis": "customer_episode",
                "oracle_measure": "scenario_oracle/payment_status_resolution_v1",
                "scope": {"tenant_id":"tenant_a","job_id":"job_a","grant_id":"grant_a","authority_ref":"authority_a"}
            }}), Some(snapshot),
        )).unwrap().reference()
    }

    fn inputs(
        repo: &mut InMemoryArtifactRepository,
        snapshot: ArtifactReference,
    ) -> EvaluationInputs {
        EvaluationInputs::new(
            EvaluationArtifactRef::baseline(artifact(
                repo,
                "018f0f4e-7bbd-7000-8000-000000000401",
                "baseline",
                "shared",
                snapshot.clone(),
            )),
            EvaluationArtifactRef::oracle(artifact(
                repo,
                "018f0f4e-7bbd-7000-8000-000000000402",
                "oracle",
                "shared",
                snapshot.clone(),
            )),
            EvaluationArtifactRef::development_suite(artifact(
                repo,
                "018f0f4e-7bbd-7000-8000-000000000403",
                "development_suite",
                "development",
                snapshot.clone(),
            )),
            EvaluationArtifactRef::final_suite(artifact(
                repo,
                "018f0f4e-7bbd-7000-8000-000000000404",
                "final_suite",
                "final",
                snapshot,
            )),
        )
    }

    fn attest_all(
        authority: &mut InMemoryEvaluationArtifactAuthority,
        bridge: &crate::workflow_bridge::WorkflowBridgeContract,
        inputs: &EvaluationInputs,
    ) {
        for reference in [
            &inputs.baseline.reference,
            &inputs.oracle.reference,
            &inputs.development_suite.reference,
            &inputs.final_suite.reference,
        ] {
            authority.issue(EvaluationArtifactGrant::for_scope(
                bridge.scope().clone(),
                reference.clone(),
            ));
        }
    }

    #[test]
    fn four_coherent_self_authored_scenario_sets_cannot_seal_without_authority_attestations() {
        let mut repo = InMemoryArtifactRepository::default();
        let snapshot = repo
            .append(
                None,
                ArtifactDraft::new(
                    "tenant_a",
                    "018f0f4e-7bbd-7000-8000-000000000400",
                    1,
                    ArtifactKind::SourceSnapshot,
                    json!({}),
                    None,
                ),
            )
            .unwrap()
            .reference();
        let bridge =
            bridge_for_evaluation_plan_test(snapshot.clone(), VerificationStatus::Supported);
        let inputs = inputs(&mut repo, snapshot);
        let authority = InMemoryEvaluationArtifactAuthority::default();
        assert_eq!(
            TrustedEvaluationComposer::from_policy(authority).seal_from_bridge(
                &bridge,
                inputs.clone(),
                &mut repo
            ),
            Err(EvaluationPlanError::AuthorityDenied)
        );
        let mut authority = InMemoryEvaluationArtifactAuthority::default();
        attest_all(&mut authority, &bridge, &inputs);
        let mut composer = TrustedEvaluationComposer::from_policy(authority);
        let plan = composer
            .seal_from_bridge(&bridge, inputs.clone(), &mut repo)
            .unwrap();
        assert!(!plan.allows_same_outcome_claim());
        assert!(!plan.eligible_for_proposal());
        let mut authority = InMemoryEvaluationArtifactAuthority::default();
        attest_all(&mut authority, &bridge, &inputs);
        authority.revoke(bridge.scope(), &inputs.oracle.reference);
        assert_eq!(
            TrustedEvaluationComposer::from_policy(authority)
                .seal_from_bridge(&bridge, inputs, &mut repo),
            Err(EvaluationPlanError::AuthorityDenied)
        );
    }

    #[test]
    fn non_proxy_bridge_is_rejected_before_artifact_authority_is_read() {
        let mut repo = InMemoryArtifactRepository::default();
        let snapshot = repo
            .append(
                None,
                ArtifactDraft::new(
                    "tenant_a",
                    "018f0f4e-7bbd-7000-8000-000000000410",
                    1,
                    ArtifactKind::SourceSnapshot,
                    json!({}),
                    None,
                ),
            )
            .unwrap()
            .reference();
        let bridge =
            bridge_for_evaluation_plan_test(snapshot.clone(), VerificationStatus::Uncertain);
        assert_eq!(
            TrustedEvaluationComposer::from_policy(InMemoryEvaluationArtifactAuthority::default())
                .seal_from_bridge(&bridge, inputs(&mut repo, snapshot), &mut repo),
            Err(EvaluationPlanError::BridgeNotEvaluable)
        );
    }
}
