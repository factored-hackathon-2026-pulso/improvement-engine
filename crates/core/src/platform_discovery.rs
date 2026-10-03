//! P1 handoff from a governed U30 platform measurement into discovery.
//!
//! The platform path persists a source-specific Signal evidence packet after
//! bound Core/model attempts. It deliberately does not fabricate U08/U12
//! lineage or create a generic U13-A/U14 candidate or opportunity claim; a
//! platform-specific independent verifier is the required next contract.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::autonomous_scout::AutonomousScout;
use crate::core_task::{
    CoreTaskBinding, CoreTaskInvocation, CoreTaskOutcome, CoreTaskReceipt, CoreTaskScope,
};
use crate::model_provider::{
    ModelInvocation, ModelOutcome, ModelPolicy, ModelReceipt, ProjectionBrokerPort,
};
use crate::platform_observations::Layer;
use crate::platform_sensor::{PlatformLayerSignal, PlatformSignalStatus};
use crate::{ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository, RepositoryError};

#[path = "platform_discovery_verifier.rs"]
pub mod verifier;

/// A measured, provenance-bound platform signal prepared for independent
/// verification. Fields are private and there is no deserializer or public
/// constructor, so callers cannot fabricate U30 evidence by setting digests.
///
/// ```compile_fail
/// use improvement_engine_core::platform_discovery::PlatformDiscoveryInput;
/// let _forged = PlatformDiscoveryInput {
///     metric_id: "tree_handoff_rate".into(), metric_version: 1,
///     layer: improvement_engine_core::platform_observations::Layer::Tree,
///     population_ref: "tree_goals".into(), numerator: 9, denominator: 10,
///     missing: 0, window_start_ms: 1, window_end_ms: 2, received_as_of_ms: 2,
///     projection_digest: "sha256:00".into(), source_id: "fake".into(),
///     contract_ref: "fake".into(), batch_digest: "sha256:00".into(),
///     coverage_evidence_digest: "sha256:00".into(),
///     metric_mapping_digest: "sha256:00".into(),
///     mapping_resolution_digest: "sha256:00".into(), signal_digest: "sha256:00".into(),
///     commitment: "sha256:00".into(),
/// };
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformDiscoveryInput {
    tenant_id: String,
    metric_id: String,
    metric_version: u16,
    layer: Layer,
    population_ref: String,
    numerator: u64,
    denominator: u64,
    missing: u64,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
    projection_digest: String,
    source_id: String,
    contract_ref: String,
    batch_digest: String,
    coverage_evidence_digest: String,
    metric_mapping_digest: String,
    mapping_resolution_digest: String,
    signal_digest: String,
    commitment: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformDiscoveryInputError {
    InsufficientEvidence,
    IncompleteProvenance,
    TenantScopeMismatch,
}

impl PlatformDiscoveryInput {
    /// Copies only a measured U30 result. Partial, ambiguous, inconsistent,
    /// unmapped or otherwise insufficient measurements cannot enter discovery.
    pub fn from_measured_signal(
        signal: &PlatformLayerSignal,
        scope: &CoreTaskScope,
    ) -> Result<Self, PlatformDiscoveryInputError> {
        if signal.tenant_id() != scope.tenant_id() {
            return Err(PlatformDiscoveryInputError::TenantScopeMismatch);
        }
        let PlatformSignalStatus::Measured {
            numerator,
            denominator,
            missing,
        } = signal.status()
        else {
            return Err(PlatformDiscoveryInputError::InsufficientEvidence);
        };

        let (
            Some(source_id),
            Some(contract_ref),
            Some(batch_digest),
            Some(coverage_evidence_digest),
            Some(metric_mapping_digest),
        ) = (
            signal.source_id(),
            signal.contract_ref(),
            signal.batch_digest(),
            signal.coverage_evidence_digest(),
            signal.metric_mapping_digest(),
        )
        else {
            return Err(PlatformDiscoveryInputError::IncompleteProvenance);
        };

        if signal.window_start_ms() >= signal.window_end_ms()
            || signal.received_as_of_ms() < signal.window_end_ms()
            || denominator == &0
            || numerator > denominator
            || !is_sha256(signal.projection_digest())
            || !is_sha256(signal.mapping_resolution_digest())
            || !is_sha256(signal.digest())
            || !is_sha256(batch_digest)
            || !is_sha256(coverage_evidence_digest)
            || !is_sha256(metric_mapping_digest)
        {
            return Err(PlatformDiscoveryInputError::IncompleteProvenance);
        }

        let mut input = Self {
            tenant_id: signal.tenant_id().to_owned(),
            metric_id: signal.metric_id().to_owned(),
            metric_version: signal.metric_version(),
            layer: signal.layer(),
            population_ref: signal.population_ref().to_owned(),
            numerator: *numerator,
            denominator: *denominator,
            missing: *missing,
            window_start_ms: signal.window_start_ms(),
            window_end_ms: signal.window_end_ms(),
            received_as_of_ms: signal.received_as_of_ms(),
            projection_digest: signal.projection_digest().to_owned(),
            source_id: source_id.to_owned(),
            contract_ref: contract_ref.to_owned(),
            batch_digest: batch_digest.to_owned(),
            coverage_evidence_digest: coverage_evidence_digest.to_owned(),
            metric_mapping_digest: metric_mapping_digest.to_owned(),
            mapping_resolution_digest: signal.mapping_resolution_digest().to_owned(),
            signal_digest: signal.digest().to_owned(),
            commitment: String::new(),
        };
        input.commitment = input_digest(&input);
        Ok(input)
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }
    pub fn metric_version(&self) -> u16 {
        self.metric_version
    }
    pub fn layer(&self) -> Layer {
        self.layer
    }
    pub fn population_ref(&self) -> &str {
        &self.population_ref
    }
    pub fn numerator(&self) -> u64 {
        self.numerator
    }
    pub fn denominator(&self) -> u64 {
        self.denominator
    }
    pub fn missing(&self) -> u64 {
        self.missing
    }
    pub fn window_start_ms(&self) -> i64 {
        self.window_start_ms
    }
    pub fn window_end_ms(&self) -> i64 {
        self.window_end_ms
    }
    pub fn received_as_of_ms(&self) -> i64 {
        self.received_as_of_ms
    }
    pub fn projection_digest(&self) -> &str {
        &self.projection_digest
    }
    pub fn source_id(&self) -> &str {
        &self.source_id
    }
    pub fn contract_ref(&self) -> &str {
        &self.contract_ref
    }
    pub fn batch_digest(&self) -> &str {
        &self.batch_digest
    }
    pub fn coverage_evidence_digest(&self) -> &str {
        &self.coverage_evidence_digest
    }
    pub fn metric_mapping_digest(&self) -> &str {
        &self.metric_mapping_digest
    }
    pub fn mapping_resolution_digest(&self) -> &str {
        &self.mapping_resolution_digest
    }
    pub fn signal_digest(&self) -> &str {
        &self.signal_digest
    }
    pub fn commitment(&self) -> &str {
        &self.commitment
    }

    /// Detects alteration after serialization or transport. This proves only
    /// commitment integrity; it does not upgrade the input into U13 authority.
    pub fn has_valid_commitment(&self) -> bool {
        self.commitment == input_digest(self)
    }
}

#[derive(Serialize)]
struct InputCommitment<'a> {
    tenant_id: &'a str,
    metric_id: &'a str,
    metric_version: u16,
    layer: Layer,
    population_ref: &'a str,
    numerator: u64,
    denominator: u64,
    missing: u64,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
    projection_digest: &'a str,
    source_id: &'a str,
    contract_ref: &'a str,
    batch_digest: &'a str,
    coverage_evidence_digest: &'a str,
    metric_mapping_digest: &'a str,
    mapping_resolution_digest: &'a str,
    signal_digest: &'a str,
}

fn input_digest(input: &PlatformDiscoveryInput) -> String {
    let bytes = serde_json::to_vec(&InputCommitment {
        tenant_id: &input.tenant_id,
        metric_id: &input.metric_id,
        metric_version: input.metric_version,
        layer: input.layer,
        population_ref: &input.population_ref,
        numerator: input.numerator,
        denominator: input.denominator,
        missing: input.missing,
        window_start_ms: input.window_start_ms,
        window_end_ms: input.window_end_ms,
        received_as_of_ms: input.received_as_of_ms,
        projection_digest: &input.projection_digest,
        source_id: &input.source_id,
        contract_ref: &input.contract_ref,
        batch_digest: &input.batch_digest,
        coverage_evidence_digest: &input.coverage_evidence_digest,
        metric_mapping_digest: &input.metric_mapping_digest,
        mapping_resolution_digest: &input.mapping_resolution_digest,
        signal_digest: &input.signal_digest,
    })
    .expect("typed platform discovery input is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// An invocation prepared only from a measured U30 input. The model receives
/// aggregate metric metadata only: no customer text, source rows, or opaque
/// tenant/grant identifiers are included in the prompt.
pub struct PlatformScoutInvocation {
    scope: CoreTaskScope,
    input_commitment: String,
    core: CoreTaskInvocation,
    model: ModelInvocation,
}

#[derive(Serialize)]
struct PlatformScoutPrompt<'a> {
    task: &'static str,
    constraints: [&'static str; 3],
    evidence: PlatformScoutPromptEvidence<'a>,
}

#[derive(Serialize)]
struct PlatformScoutPromptEvidence<'a> {
    metric_id: &'a str,
    metric_version: u16,
    layer: Layer,
    population_ref: &'a str,
    numerator: u64,
    denominator: u64,
    missing: u64,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
}

impl PlatformScoutInvocation {
    /// Creates the exact Core and model inputs for one platform Scout attempt.
    /// The model projection broker is the trusted egress/treatment boundary;
    /// it receives only a compact aggregate, never raw platform events.
    pub fn prepare(
        scope: CoreTaskScope,
        input: &PlatformDiscoveryInput,
        binding: CoreTaskBinding,
        core_attempt_id: impl Into<String>,
        policy: ModelPolicy,
        model_attempt_id: impl Into<String>,
        broker: &mut impl ProjectionBrokerPort,
    ) -> Result<Self, PlatformScoutError> {
        if !input.has_valid_commitment() {
            return Err(PlatformScoutError::InvalidInput);
        }
        if input.tenant_id != scope.tenant_id() {
            return Err(PlatformScoutError::ScopeMismatch);
        }
        let core = CoreTaskInvocation::new(
            scope.clone(),
            binding,
            core_attempt_id,
            input.commitment.clone(),
        )
        .map_err(|_| PlatformScoutError::InvalidInvocation)?;
        let prompt = PlatformScoutPrompt {
            task: "prepare_platform_signal_evidence_for_independent_verification",
            constraints: [
                "Describe only what the aggregate measurement supports.",
                "Do not claim causality, customer harm, or business lift.",
                "Do not propose an executable or publishable change.",
            ],
            evidence: PlatformScoutPromptEvidence {
                metric_id: &input.metric_id,
                metric_version: input.metric_version,
                layer: input.layer,
                population_ref: &input.population_ref,
                numerator: input.numerator,
                denominator: input.denominator,
                missing: input.missing,
                window_start_ms: input.window_start_ms,
                window_end_ms: input.window_end_ms,
                received_as_of_ms: input.received_as_of_ms,
            },
        };
        let treated_input =
            serde_json::to_string(&prompt).map_err(|_| PlatformScoutError::InvalidInput)?;
        let projection = broker
            .authorize_projection(&scope, &policy, treated_input)
            .map_err(|_| PlatformScoutError::ModelProjectionDenied)?;
        let model =
            ModelInvocation::from_verified(scope.clone(), policy, model_attempt_id, projection)
                .map_err(|_| PlatformScoutError::ModelProjectionDenied)?;
        Ok(Self {
            scope,
            input_commitment: input.commitment.clone(),
            core,
            model,
        })
    }

    pub fn core_invocation(&self) -> &CoreTaskInvocation {
        &self.core
    }

    pub fn model_invocation(&self) -> &ModelInvocation {
        &self.model
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformScoutCandidateKind {
    SignalEvidencePacket,
}

/// Platform-specific signal evidence packet from U30 evidence and one
/// successful Scout invocation. It is persisted as an immutable `Signal` artifact, but
/// is explicitly not eligible for U13-A/U14 until a platform-specific
/// independent-verification bridge exists. It is not an opportunity, proposal,
/// release, or causal claim. It retains Core/model attempt IDs and output
/// digests only; no structured result or model free text is included.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformScoutCandidate {
    candidate_id: String,
    kind: PlatformScoutCandidateKind,
    eligibility_boundary: &'static str,
    tenant_id: String,
    job_id: String,
    grant_id: String,
    authority_ref: String,
    metric_id: String,
    metric_version: u16,
    layer: Layer,
    population_ref: String,
    numerator: u64,
    denominator: u64,
    missing: u64,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
    source_id: String,
    contract_ref: String,
    batch_digest: String,
    coverage_evidence_digest: String,
    metric_mapping_digest: String,
    mapping_resolution_digest: String,
    projection_digest: String,
    platform_signal_digest: String,
    platform_input_commitment: String,
    core_attempt_id: String,
    core_run_id: String,
    core_binding_digest: String,
    core_output_digest: String,
    model_attempt_id: String,
    model_policy_digest: String,
    model_capability_digest: String,
    model_input_commitment: String,
    model_output_digest: String,
    provenance_commitment: String,
    digest: String,
}

#[derive(Serialize)]
struct PlatformScoutCandidateProvenance<'a> {
    eligibility_boundary: &'static str,
    tenant_id: &'a str,
    job_id: &'a str,
    grant_id: &'a str,
    authority_ref: &'a str,
    metric_id: &'a str,
    metric_version: u16,
    layer: Layer,
    population_ref: &'a str,
    numerator: u64,
    denominator: u64,
    missing: u64,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
    source_id: &'a str,
    contract_ref: &'a str,
    batch_digest: &'a str,
    coverage_evidence_digest: &'a str,
    metric_mapping_digest: &'a str,
    mapping_resolution_digest: &'a str,
    projection_digest: &'a str,
    platform_signal_digest: &'a str,
    platform_input_commitment: &'a str,
    core_attempt_id: &'a str,
    core_run_id: &'a str,
    core_binding_digest: &'a str,
    core_output_digest: &'a str,
    model_attempt_id: &'a str,
    model_policy_digest: &'a str,
    model_capability_digest: &'a str,
    model_input_commitment: &'a str,
    model_output_digest: &'a str,
}

#[derive(Serialize)]
struct PlatformScoutCandidateContent<'a> {
    candidate_id: &'a str,
    kind: PlatformScoutCandidateKind,
    eligibility_boundary: &'static str,
    provenance_commitment: &'a str,
}

impl PlatformScoutCandidate {
    pub fn candidate_id(&self) -> &str {
        &self.candidate_id
    }
    pub fn kind(&self) -> PlatformScoutCandidateKind {
        self.kind
    }
    pub fn eligibility_boundary(&self) -> &'static str {
        self.eligibility_boundary
    }
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }
    pub fn numerator(&self) -> u64 {
        self.numerator
    }
    pub fn denominator(&self) -> u64 {
        self.denominator
    }
    pub fn batch_digest(&self) -> &str {
        &self.batch_digest
    }
    pub fn platform_signal_digest(&self) -> &str {
        &self.platform_signal_digest
    }
    pub fn projection_digest(&self) -> &str {
        &self.projection_digest
    }
    pub fn core_run_id(&self) -> &str {
        &self.core_run_id
    }
    pub fn provenance_commitment(&self) -> &str {
        &self.provenance_commitment
    }

    pub fn has_valid_digest(&self) -> bool {
        self.provenance_commitment == platform_candidate_provenance_digest(self)
            && self.digest == platform_candidate_digest(self)
    }

    /// Appends this source-specific signal candidate as an immutable artifact.
    /// A repeated identical write is idempotent; the same artifact ID with
    /// different content fails closed. U30 aggregate provenance is carried in
    /// the payload and is never represented as a U08 source snapshot reference.
    /// Callers recovering from an ambiguous storage error must retry with the
    /// same stable artifact ID; the initial readback then resolves a committed
    /// write without creating another revision.
    pub fn append_immutable(
        &self,
        artifacts: &mut impl ArtifactRepository,
        artifact_id: impl Into<String>,
    ) -> Result<ArtifactReference, PlatformScoutError> {
        if !self.has_valid_digest() {
            return Err(PlatformScoutError::InvalidInput);
        }
        let artifact_id = artifact_id.into();
        let payload = serde_json::to_value(self).map_err(|_| PlatformScoutError::InvalidInput)?;
        if let Some(existing) = artifacts
            .get(&self.tenant_id, &artifact_id, 1)
            .map_err(PlatformScoutError::ArtifactRepository)?
        {
            if is_same_platform_signal_artifact(&existing, &payload) {
                return Ok(existing.reference());
            }
            return Err(PlatformScoutError::ArtifactConflict);
        }
        let draft = ArtifactDraft::new(
            self.tenant_id.clone(),
            artifact_id.clone(),
            1,
            ArtifactKind::Signal,
            payload.clone(),
            None,
        );
        match artifacts.append(None, draft) {
            Ok(appended) => Ok(appended.reference()),
            Err(error @ RepositoryError::RevisionConflict { .. }) => {
                // For a CAS conflict, read back and accept only an exact
                // content match. Storage errors are returned; callers recover
                // an ambiguous commit by retrying the same stable artifact ID.
                if let Some(existing) = artifacts
                    .get(&self.tenant_id, &artifact_id, 1)
                    .map_err(PlatformScoutError::ArtifactRepository)?
                {
                    if is_same_platform_signal_artifact(&existing, &payload) {
                        return Ok(existing.reference());
                    }
                }
                Err(PlatformScoutError::ArtifactRepository(error))
            }
            Err(error) => Err(PlatformScoutError::ArtifactRepository(error)),
        }
    }
}

fn is_same_platform_signal_artifact(artifact: &ArtifactDraft, payload: &serde_json::Value) -> bool {
    artifact.kind == ArtifactKind::Signal
        && artifact.payload == *payload
        && artifact.source_snapshot_ref.is_none()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformScoutResult {
    Candidate(Box<PlatformScoutCandidate>),
    DependencyBlocked { reason: &'static str },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformScoutError {
    InvalidInput,
    ScopeMismatch,
    InvalidInvocation,
    ModelProjectionDenied,
    ReceiptMismatch,
    ArtifactConflict,
    ArtifactRepository(RepositoryError),
}

impl AutonomousScout {
    /// Admits a measured U30 input through the Scout invocation boundary.
    /// This returns only a bounded platform-specific signal candidate; it
    /// never creates a generic U13-A candidate, U14-verified candidate,
    /// opportunity, or proposal.
    pub fn discover_platform(
        input: &PlatformDiscoveryInput,
        invocation: &PlatformScoutInvocation,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<PlatformScoutResult, PlatformScoutError> {
        if !input.has_valid_commitment() {
            return Err(PlatformScoutError::InvalidInput);
        }
        if input.tenant_id != invocation.scope.tenant_id() {
            return Err(PlatformScoutError::ScopeMismatch);
        }
        if invocation.input_commitment != input.commitment
            || invocation.core.scope() != &invocation.scope
            || invocation.core.input_digest() != input.commitment
            || invocation.model.scope() != &invocation.scope
            || core.scope() != &invocation.scope
            || model.scope() != &invocation.scope
            || core.input_digest() != input.commitment
            || core.input_digest() != invocation.core.input_digest()
            || core.binding_digest() != invocation.core.binding().digest()
            || core.attempt_id() != invocation.core.attempt_id()
            || model.input_commitment() != invocation.model.input_commitment()
            || model.policy_digest() != invocation.model.policy().digest()
            || model.capability_digest() != invocation.model.policy().capability().digest()
            || model.attempt_id() != invocation.model.attempt_id()
        {
            return Err(PlatformScoutError::ReceiptMismatch);
        }
        if core.outcome() != &CoreTaskOutcome::Succeeded {
            return Ok(PlatformScoutResult::DependencyBlocked {
                reason: "core_task_unknown_or_unsuccessful",
            });
        }
        if model.outcome() != &ModelOutcome::Succeeded {
            return Ok(PlatformScoutResult::DependencyBlocked {
                reason: "model_dependency_unavailable_or_unsuccessful",
            });
        }
        let (Some(core_run_id), Some(core_output_digest), Some(model_output_digest)) = (
            core.core_run_id(),
            core.output_digest(),
            model.output_digest(),
        ) else {
            return Err(PlatformScoutError::ReceiptMismatch);
        };
        let mut candidate = PlatformScoutCandidate {
            candidate_id: String::new(),
            kind: PlatformScoutCandidateKind::SignalEvidencePacket,
            eligibility_boundary: "not_eligible_for_U13A_U14_requires_platform_independent_verifier",
            tenant_id: invocation.scope.tenant_id().to_owned(),
            job_id: invocation.scope.job_id().to_owned(),
            grant_id: invocation.scope.grant_id().to_owned(),
            authority_ref: invocation.scope.authority_ref().to_owned(),
            metric_id: input.metric_id.clone(),
            metric_version: input.metric_version,
            layer: input.layer,
            population_ref: input.population_ref.clone(),
            numerator: input.numerator,
            denominator: input.denominator,
            missing: input.missing,
            window_start_ms: input.window_start_ms,
            window_end_ms: input.window_end_ms,
            received_as_of_ms: input.received_as_of_ms,
            source_id: input.source_id.clone(),
            contract_ref: input.contract_ref.clone(),
            batch_digest: input.batch_digest.clone(),
            coverage_evidence_digest: input.coverage_evidence_digest.clone(),
            metric_mapping_digest: input.metric_mapping_digest.clone(),
            mapping_resolution_digest: input.mapping_resolution_digest.clone(),
            projection_digest: input.projection_digest.clone(),
            platform_signal_digest: input.signal_digest.clone(),
            platform_input_commitment: input.commitment.clone(),
            core_attempt_id: core.attempt_id().to_owned(),
            core_run_id: core_run_id.to_owned(),
            core_binding_digest: core.binding_digest().to_owned(),
            core_output_digest: core_output_digest.to_owned(),
            model_attempt_id: model.attempt_id().to_owned(),
            model_policy_digest: model.policy_digest().to_owned(),
            model_capability_digest: model.capability_digest().to_owned(),
            model_input_commitment: model.input_commitment().to_owned(),
            model_output_digest: model_output_digest.to_owned(),
            provenance_commitment: String::new(),
            digest: String::new(),
        };
        candidate.provenance_commitment = platform_candidate_provenance_digest(&candidate);
        candidate.candidate_id = format!("platform-signal-{}", candidate.provenance_commitment);
        candidate.digest = platform_candidate_digest(&candidate);
        Ok(PlatformScoutResult::Candidate(Box::new(candidate)))
    }
}

fn platform_candidate_provenance_digest(candidate: &PlatformScoutCandidate) -> String {
    let bytes = serde_json::to_vec(&PlatformScoutCandidateProvenance {
        eligibility_boundary: candidate.eligibility_boundary,
        tenant_id: &candidate.tenant_id,
        job_id: &candidate.job_id,
        grant_id: &candidate.grant_id,
        authority_ref: &candidate.authority_ref,
        metric_id: &candidate.metric_id,
        metric_version: candidate.metric_version,
        layer: candidate.layer,
        population_ref: &candidate.population_ref,
        numerator: candidate.numerator,
        denominator: candidate.denominator,
        missing: candidate.missing,
        window_start_ms: candidate.window_start_ms,
        window_end_ms: candidate.window_end_ms,
        received_as_of_ms: candidate.received_as_of_ms,
        source_id: &candidate.source_id,
        contract_ref: &candidate.contract_ref,
        batch_digest: &candidate.batch_digest,
        coverage_evidence_digest: &candidate.coverage_evidence_digest,
        metric_mapping_digest: &candidate.metric_mapping_digest,
        mapping_resolution_digest: &candidate.mapping_resolution_digest,
        projection_digest: &candidate.projection_digest,
        platform_signal_digest: &candidate.platform_signal_digest,
        platform_input_commitment: &candidate.platform_input_commitment,
        core_attempt_id: &candidate.core_attempt_id,
        core_run_id: &candidate.core_run_id,
        core_binding_digest: &candidate.core_binding_digest,
        core_output_digest: &candidate.core_output_digest,
        model_attempt_id: &candidate.model_attempt_id,
        model_policy_digest: &candidate.model_policy_digest,
        model_capability_digest: &candidate.model_capability_digest,
        model_input_commitment: &candidate.model_input_commitment,
        model_output_digest: &candidate.model_output_digest,
    })
    .expect("typed platform Scout provenance is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn platform_candidate_digest(candidate: &PlatformScoutCandidate) -> String {
    let bytes = serde_json::to_vec(&PlatformScoutCandidateContent {
        candidate_id: &candidate.candidate_id,
        kind: candidate.kind,
        eligibility_boundary: candidate.eligibility_boundary,
        provenance_commitment: &candidate.provenance_commitment,
    })
    .expect("typed platform Scout candidate is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use crate::model_provider::{
        HmacProjectionBroker, ModelBudgetLimits, ModelCapability, ModelProvider, RedactionPolicy,
        VerifiedProjection,
    };

    struct CountingBroker {
        inner: HmacProjectionBroker,
        calls: usize,
    }

    impl ProjectionBrokerPort for CountingBroker {
        fn authorize_projection(
            &mut self,
            scope: &CoreTaskScope,
            policy: &ModelPolicy,
            treated_input: String,
        ) -> Result<VerifiedProjection, crate::model_provider::ModelProviderError> {
            self.calls += 1;
            self.inner
                .authorize_projection(scope, policy, treated_input)
        }
    }

    #[test]
    fn tampered_platform_input_is_rejected_before_egress_projection() {
        let mut input = PlatformDiscoveryInput {
            tenant_id: "bank_demo".into(),
            metric_id: "tree_handoff_rate".into(),
            metric_version: 1,
            layer: Layer::Tree,
            population_ref: "tree_goals".into(),
            numerator: 1,
            denominator: 4,
            missing: 0,
            window_start_ms: 10,
            window_end_ms: 20,
            received_as_of_ms: 21,
            projection_digest: format!("sha256:{}", "a".repeat(64)),
            source_id: "attention-platform".into(),
            contract_ref: "contract:attention-v1".into(),
            batch_digest: format!("sha256:{}", "b".repeat(64)),
            coverage_evidence_digest: format!("sha256:{}", "c".repeat(64)),
            metric_mapping_digest: format!("sha256:{}", "d".repeat(64)),
            mapping_resolution_digest: format!("sha256:{}", "e".repeat(64)),
            signal_digest: format!("sha256:{}", "f".repeat(64)),
            commitment: String::new(),
        };
        input.commitment = input_digest(&input);
        input.numerator = 2;

        let scope = CoreTaskScope::new(
            "bank_demo",
            "job-platform",
            "grant-platform",
            "authority-platform",
        )
        .unwrap();
        let capability = ModelCapability::new(
            ModelProvider::OpenRouter,
            "https://openrouter.ai/api/v1",
            "fixture/scout-model",
            "secret://fixture/model-key",
            "fixture-revision-1",
        )
        .unwrap();
        let policy = ModelPolicy::with_budget(
            "platform-scout-policy",
            capability,
            "platform_signal_investigation",
            RedactionPolicy::RejectMarkedInput,
            0,
            1_000,
            ModelBudgetLimits::new(4_096, 1_024, 100_000).unwrap(),
        )
        .unwrap();
        let mut broker = CountingBroker {
            inner: HmacProjectionBroker::new_for_test(
                b"test-only-platform-scout-projection-key-32-bytes",
            )
            .unwrap(),
            calls: 0,
        };

        let result = PlatformScoutInvocation::prepare(
            scope,
            &input,
            CoreTaskBinding::new(
                "scout",
                "scout-release",
                "0.5.0",
                "53e729d624c8284e906249df84c1a1df84cc8d40",
            )
            .unwrap(),
            "core-attempt-tampered",
            policy,
            "model-attempt-tampered",
            &mut broker,
        );

        assert!(matches!(result, Err(PlatformScoutError::InvalidInput)));
        assert_eq!(broker.calls, 0);
    }
}
