//! U20 seals typed, repository-attested evaluation inputs before execution.
//!
//! `ScenarioSet` remains the durable storage kind until the artifact schema
//! grows dedicated rows. This module does not trust that generic kind alone:
//! every selected revision must carry the shared `evaluation_contract` and is
//! re-read from the immutable repository before a plan can be frozen.

use crate::source_validation::SourceSnapshot;
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
    /// U04-B's exact parsed-source identity. This is intentionally distinct
    /// from the repository artifact-content digest in `source_snapshot_ref`.
    source_snapshot_binding_digest: String,
    baseline_ref: ArtifactReference,
    oracle_ref: ArtifactReference,
    development_suite_ref: ArtifactReference,
    final_suite_ref: ArtifactReference,
    semantic: EvaluationSemanticContract,
    commitment: String,
}

/// Crate-private exact U20 plan material required by the E0 safety-oracle
/// composition. It is a read-only binding, not an evaluation result or a
/// capability to run/release a candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvaluationPlanBinding {
    source_snapshot_ref: ArtifactReference,
    source_snapshot_binding_digest: String,
    baseline_ref: ArtifactReference,
    oracle_ref: ArtifactReference,
    development_suite_ref: ArtifactReference,
    final_suite_ref: ArtifactReference,
    commitment: String,
}

#[allow(dead_code)] // Consumed by the crate-private U20-E safety composition.
impl EvaluationPlanBinding {
    #[must_use]
    pub(crate) fn source_snapshot_ref(&self) -> &ArtifactReference {
        &self.source_snapshot_ref
    }
    #[must_use]
    pub(crate) fn source_snapshot_binding_digest(&self) -> &str {
        &self.source_snapshot_binding_digest
    }
    #[must_use]
    pub(crate) fn baseline_ref(&self) -> &ArtifactReference {
        &self.baseline_ref
    }
    #[must_use]
    pub(crate) fn oracle_ref(&self) -> &ArtifactReference {
        &self.oracle_ref
    }
    #[must_use]
    pub(crate) fn development_suite_ref(&self) -> &ArtifactReference {
        &self.development_suite_ref
    }
    #[must_use]
    pub(crate) fn final_suite_ref(&self) -> &ArtifactReference {
        &self.final_suite_ref
    }
    #[must_use]
    pub(crate) fn commitment(&self) -> &str {
        &self.commitment
    }
}

#[cfg(test)]
pub(crate) fn e0_safety_plan_fixture(
    tenant_id: &str,
    source_snapshot_ref: ArtifactReference,
) -> EvaluationPlan {
    let reference = |id: &str, revision: u64, marker: char| ArtifactReference {
        tenant_id: tenant_id.to_owned(),
        id: id.to_owned(),
        revision,
        digest: format!("sha256:{}", marker.to_string().repeat(64)),
    };
    let binding = EvaluationPlanBinding {
        source_snapshot_binding_digest: source_snapshot_ref.digest.clone(),
        source_snapshot_ref,
        baseline_ref: reference("018f0f4e-7bbd-7000-8000-000000000701", 1, 'b'),
        oracle_ref: reference("018f0f4e-7bbd-7000-8000-000000000702", 2, 'c'),
        development_suite_ref: reference("018f0f4e-7bbd-7000-8000-000000000703", 3, 'd'),
        final_suite_ref: reference("018f0f4e-7bbd-7000-8000-000000000704", 4, 'e'),
        commitment: "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            .to_owned(),
    };
    EvaluationPlan {
        bridge_commitment: "sha256:bridge".to_owned(),
        source_snapshot_ref: binding.source_snapshot_ref,
        source_snapshot_binding_digest: binding.source_snapshot_binding_digest,
        baseline_ref: binding.baseline_ref,
        oracle_ref: binding.oracle_ref,
        development_suite_ref: binding.development_suite_ref,
        final_suite_ref: binding.final_suite_ref,
        semantic: EvaluationSemanticContract {
            target_outcome: "identity_check".to_owned(),
            unit_of_analysis: "interaction".to_owned(),
            oracle_measure: "safety_pass_rate".to_owned(),
        },
        commitment: binding.commitment,
    }
}

#[cfg(test)]
pub(crate) fn e0_safety_plan_fixture_with_oracle_revision(
    tenant_id: &str,
    source_snapshot_ref: ArtifactReference,
    oracle_revision: u64,
) -> EvaluationPlan {
    let mut plan = e0_safety_plan_fixture(tenant_id, source_snapshot_ref);
    plan.oracle_ref.revision = oracle_revision;
    plan
}

/// Crate-private projection for the native-evaluation admission boundary.
/// It intentionally excludes the final suite and oracle: neither may cross
/// into the public/native evaluator request.
#[allow(dead_code)] // Consumed by the next trusted U19 composition.
#[derive(Clone)]
pub(crate) struct NativeEvaluationPlanMaterial {
    pub(crate) plan_commitment: String,
    pub(crate) source_snapshot: ArtifactReference,
    pub(crate) development_suite: ArtifactReference,
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
        let source_snapshot_binding_digest = verified_source_snapshot_binding(artifacts, snapshot)?;
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
            &source_snapshot_binding_digest,
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
            source_snapshot_binding_digest,
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

    /// Exposes exact, already-sealed U20 input references only to another
    /// trusted in-crate composition. Public callers still cannot construct a
    /// plan, alter these references, or treat them as execution authority.
    #[must_use]
    #[allow(dead_code)] // Called by the crate-private U20-E safety composition.
    pub(crate) fn e0_safety_binding(&self) -> EvaluationPlanBinding {
        EvaluationPlanBinding {
            source_snapshot_ref: self.source_snapshot_ref.clone(),
            source_snapshot_binding_digest: self.source_snapshot_binding_digest.clone(),
            baseline_ref: self.baseline_ref.clone(),
            oracle_ref: self.oracle_ref.clone(),
            development_suite_ref: self.development_suite_ref.clone(),
            final_suite_ref: self.final_suite_ref.clone(),
            commitment: self.commitment.clone(),
        }
    }

    #[allow(dead_code)] // Consumed by the next trusted U19 composition.
    pub(crate) fn native_evaluation_material(&self) -> NativeEvaluationPlanMaterial {
        NativeEvaluationPlanMaterial {
            plan_commitment: self.commitment.clone(),
            source_snapshot: self.source_snapshot_ref.clone(),
            development_suite: self.development_suite_ref.clone(),
        }
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

/// Resolves the U16 source-snapshot artifact into the exact U04-B parsed
/// source identity. Repository artifact content and a `SourceSnapshot` byte
/// seal are different domains: a plan keeps both and never compares them as
/// though they were interchangeable.
fn verified_source_snapshot_binding<R: ArtifactRepository>(
    artifacts: &mut R,
    reference: &ArtifactReference,
) -> Result<String, EvaluationPlanError> {
    let artifact = artifacts
        .get(&reference.tenant_id, &reference.id, reference.revision)
        .map_err(|_| EvaluationPlanError::ReferenceUnavailable)?
        .ok_or(EvaluationPlanError::ReferenceUnavailable)?;
    if artifact.reference() != *reference
        || artifact.kind != ArtifactKind::SourceSnapshot
        || artifact.source_snapshot_ref.is_some()
    {
        return Err(EvaluationPlanError::ReferenceMismatch);
    }
    let raw = artifact
        .payload
        .get("source_snapshot_json")
        .and_then(Value::as_str)
        .ok_or(EvaluationPlanError::InvalidInput)?;
    let snapshot = SourceSnapshot::from_json(raw).map_err(|_| EvaluationPlanError::InvalidInput)?;
    if snapshot.tenant_id() != reference.tenant_id {
        return Err(EvaluationPlanError::CrossTenantReference);
    }
    Ok(snapshot.binding_digest())
}

#[cfg(test)]
pub(crate) fn plan_for_native_evaluation_test(
    source_snapshot: ArtifactReference,
    development_suite: ArtifactReference,
) -> EvaluationPlan {
    let source_snapshot_binding_digest = source_snapshot.digest.clone();
    EvaluationPlan {
        bridge_commitment: "sha256:bridge".to_owned(),
        source_snapshot_ref: source_snapshot.clone(),
        source_snapshot_binding_digest,
        baseline_ref: source_snapshot.clone(),
        oracle_ref: source_snapshot.clone(),
        development_suite_ref: development_suite.clone(),
        final_suite_ref: source_snapshot,
        semantic: EvaluationSemanticContract {
            target_outcome: "resolved".to_owned(),
            unit_of_analysis: "case".to_owned(),
            oracle_measure: "resolution".to_owned(),
        },
        commitment: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .to_owned(),
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
pub(crate) mod tests {
    use super::{
        EvaluationArtifactGrant, EvaluationArtifactRef, EvaluationInputs, EvaluationPlan,
        EvaluationPlanError, InMemoryEvaluationArtifactAuthority, TrustedEvaluationComposer,
    };
    use crate::independent_verifier::VerificationStatus;
    use crate::source_validation::SourceSnapshot;
    use crate::workflow_bridge::bridge_for_evaluation_plan_test;
    use crate::{
        ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository,
        InMemoryArtifactRepository,
    };
    use serde_json::json;

    pub(crate) fn source_snapshot_payload(tenant_id: &str) -> serde_json::Value {
        let raw = json!({
            "contract_version": {"major": 1, "minor": 0},
            "tenant_id": tenant_id,
            "source_namespace": "platform_history",
            "world_ref": "world_a",
            "observed_cutoff": "1970-01-01T00:01:40Z",
            "sources": [{
                "table": "case",
                "uri": "file://fixture.csv",
                "file_digest": format!("sha256:{}", "a".repeat(64)),
                "header_digest": format!("sha256:{}", "b".repeat(64)),
                "row_count": 1,
                "source_contract_ref": {
                    "id": "case", "version": "v1",
                    "digest": format!("sha256:{}", "c".repeat(64))
                }
            }]
        })
        .to_string();
        json!({"source_snapshot_json": raw})
    }

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

    /// Cross-slice fixture: a real U20 plan backed by a persisted source
    /// snapshot whose raw bytes can also feed U04-B. Test-only callers cannot
    /// manufacture the production composer or its authority.
    pub(crate) fn real_e0_plan_and_snapshot() -> (EvaluationPlan, SourceSnapshot) {
        let raw = source_snapshot_payload("tenant_a")["source_snapshot_json"]
            .as_str()
            .unwrap()
            .to_owned();
        let parsed = SourceSnapshot::from_json(&raw).unwrap();
        let mut repo = InMemoryArtifactRepository::default();
        let snapshot = repo
            .append(
                None,
                ArtifactDraft::new(
                    "tenant_a",
                    "018f0f4e-7bbd-7000-8000-000000000420",
                    1,
                    ArtifactKind::SourceSnapshot,
                    json!({"source_snapshot_json": raw}),
                    None,
                ),
            )
            .unwrap()
            .reference();
        let bridge =
            bridge_for_evaluation_plan_test(snapshot.clone(), VerificationStatus::Supported);
        let inputs = inputs(&mut repo, snapshot);
        let mut authority = InMemoryEvaluationArtifactAuthority::default();
        attest_all(&mut authority, &bridge, &inputs);
        let plan = TrustedEvaluationComposer::from_policy(authority)
            .seal_from_bridge(&bridge, inputs, &mut repo)
            .unwrap();
        (plan, parsed)
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
                    source_snapshot_payload("tenant_a"),
                    None,
                ),
            )
            .unwrap()
            .reference();
        let bridge =
            bridge_for_evaluation_plan_test(snapshot.clone(), VerificationStatus::Supported);
        let inputs = inputs(&mut repo, snapshot.clone());
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
        let snapshot_artifact = repo
            .get(&snapshot.tenant_id, &snapshot.id, snapshot.revision)
            .unwrap()
            .unwrap();
        let raw = snapshot_artifact.payload["source_snapshot_json"]
            .as_str()
            .unwrap();
        let parsed = SourceSnapshot::from_json(raw).unwrap();
        assert_eq!(
            plan.e0_safety_binding().source_snapshot_binding_digest(),
            parsed.binding_digest()
        );
        assert_ne!(
            plan.e0_safety_binding().source_snapshot_ref().digest,
            parsed.binding_digest(),
            "artifact content and source-byte sealing remain distinct digest domains"
        );
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
                    source_snapshot_payload("tenant_a"),
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
