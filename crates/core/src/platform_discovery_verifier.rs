//! Independent, deterministic verification for U30 platform Scout evidence.
//!
//! This module verifies a bounded descriptive trend, not a cause, business
//! outcome, opportunity, proposal, or permission to execute a platform change.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::core_task::{CoreTaskOutcome, CoreTaskReceipt, CoreTaskScope};
use crate::model_provider::{ModelOutcome, ModelReceipt};
use crate::platform_discovery::{
    PlatformDiscoveryInput, PlatformScoutCandidate, PlatformScoutCandidateKind,
};
use crate::platform_observations::Layer;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionOfConcern {
    Increase,
    Decrease,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformVerificationPolicy {
    policy_id: String,
    policy_version: u16,
    metric_id: String,
    metric_version: u16,
    layer: Layer,
    population_ref: String,
    metric_mapping_digest: String,
    direction_of_concern: DirectionOfConcern,
    minimum_denominator: u64,
    minimum_coverage_basis_points: u16,
    minimum_directional_change_basis_points: u16,
    digest: String,
}

#[derive(Serialize)]
struct PolicyContent<'a> {
    policy_id: &'a str,
    policy_version: u16,
    metric_id: &'a str,
    metric_version: u16,
    layer: Layer,
    population_ref: &'a str,
    metric_mapping_digest: &'a str,
    direction_of_concern: DirectionOfConcern,
    minimum_denominator: u64,
    minimum_coverage_basis_points: u16,
    minimum_directional_change_basis_points: u16,
}

impl PlatformVerificationPolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        policy_id: impl Into<String>,
        policy_version: u16,
        metric_id: impl Into<String>,
        metric_version: u16,
        layer: Layer,
        population_ref: impl Into<String>,
        metric_mapping_digest: impl Into<String>,
        direction_of_concern: DirectionOfConcern,
        minimum_denominator: u64,
        minimum_coverage_basis_points: u16,
        minimum_directional_change_basis_points: u16,
    ) -> Result<Self, PlatformVerificationError> {
        let mut policy = Self {
            policy_id: policy_id.into(),
            policy_version,
            metric_id: metric_id.into(),
            metric_version,
            layer,
            population_ref: population_ref.into(),
            metric_mapping_digest: metric_mapping_digest.into(),
            direction_of_concern,
            minimum_denominator,
            minimum_coverage_basis_points,
            minimum_directional_change_basis_points,
            digest: String::new(),
        };
        if policy.policy_id.is_empty()
            || policy.policy_version == 0
            || policy.metric_id.is_empty()
            || policy.metric_version == 0
            || policy.layer == Layer::Unknown
            || policy.population_ref.is_empty()
            || !is_sha256(&policy.metric_mapping_digest)
            || policy.minimum_denominator == 0
            || policy.minimum_coverage_basis_points == 0
            || policy.minimum_coverage_basis_points > 10_000
            || policy.minimum_directional_change_basis_points == 0
            || policy.minimum_directional_change_basis_points > 10_000
        {
            return Err(PlatformVerificationError::InvalidPolicy);
        }
        policy.digest = policy.compute_digest();
        Ok(policy)
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    fn compute_digest(&self) -> String {
        let bytes = serde_json::to_vec(&PolicyContent {
            policy_id: &self.policy_id,
            policy_version: self.policy_version,
            metric_id: &self.metric_id,
            metric_version: self.metric_version,
            layer: self.layer,
            population_ref: &self.population_ref,
            metric_mapping_digest: &self.metric_mapping_digest,
            direction_of_concern: self.direction_of_concern,
            minimum_denominator: self.minimum_denominator,
            minimum_coverage_basis_points: self.minimum_coverage_basis_points,
            minimum_directional_change_basis_points: self.minimum_directional_change_basis_points,
        })
        .expect("typed verification policy is serializable");
        format!("sha256:{:x}", Sha256::digest(bytes))
    }
}

/// Independently observed comparison window, obtained from the trusted U30
/// projection path rather than copied from a Scout candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformComparisonMeasurement {
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
    metric_mapping_digest: String,
    mapping_resolution_digest: String,
    source_digest: String,
}

impl PlatformComparisonMeasurement {
    /// Builds the comparison only from an immutable, sealed U30 measurement.
    /// Callers cannot supply bare counts or fabricate a source digest.
    pub fn from_discovery_input(
        input: &PlatformDiscoveryInput,
    ) -> Result<Self, PlatformVerificationError> {
        if !input.has_valid_commitment() {
            return Err(PlatformVerificationError::InvalidComparisonEvidence);
        }
        Ok(Self {
            tenant_id: input.tenant_id.clone(),
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
            metric_mapping_digest: input.metric_mapping_digest.clone(),
            mapping_resolution_digest: input.mapping_resolution_digest.clone(),
            source_digest: input.signal_digest.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformVerificationOutcome {
    SupportedDescriptive,
    Refuted,
    Uncertain,
    InsufficientEvidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformVerificationReason {
    CurrentSupportBelowMinimum,
    CurrentCoverageBelowMinimum,
    ComparisonMissing,
    ComparisonSupportBelowMinimum,
    ComparisonCoverageBelowMinimum,
    MetricMappingMismatch,
    ComparisonTenantMismatch,
    ComparisonWindowOverlaps,
    DirectionalChangeSupported,
    OppositeDirectionObserved,
    DirectionalChangeWithinUncertaintyBand,
}

/// A minimal, privacy-safe report. Tenant/job/grant/authority identifiers and
/// raw evidence do not cross into this serializable projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformVerificationReport {
    candidate_digest: String,
    policy_digest: String,
    outcome: PlatformVerificationOutcome,
    reason: PlatformVerificationReason,
    current_rate_basis_points: u16,
    comparison_rate_basis_points: Option<u16>,
    signed_change_basis_points: Option<i32>,
    comparison_evidence: Option<ComparisonEvidenceReference>,
    may_claim_cause: bool,
    may_claim_business_lift: bool,
    may_create_proposal: bool,
    may_execute: bool,
}

/// Safe replay identity for the comparison measurement. Tenant is included in
/// the commitment preimage but omitted from this report projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ComparisonEvidenceReference {
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
    metric_mapping_digest: String,
    mapping_resolution_digest: String,
    source_signal_digest: String,
    evidence_digest: String,
}

#[derive(Serialize)]
struct ComparisonEvidenceContent<'a> {
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
    metric_mapping_digest: &'a str,
    mapping_resolution_digest: &'a str,
    source_signal_digest: &'a str,
}

impl PlatformVerificationReport {
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    pub fn outcome(&self) -> PlatformVerificationOutcome {
        self.outcome
    }
    pub fn reason(&self) -> PlatformVerificationReason {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformVerificationError {
    InvalidCandidate,
    InvalidComparisonEvidence,
    InvalidPolicy,
    ScopeMismatch,
    CandidateEvidenceMismatch,
    CoreReceiptMismatch,
    ModelReceiptMismatch,
}

pub struct PlatformIndependentVerifier;

impl PlatformIndependentVerifier {
    /// Verify the candidate against the exact independently held U30 input,
    /// authenticated invocation scope, typed Core/model receipts, and a
    /// composition-root supplied immutable metric policy.
    pub fn verify(
        candidate: &PlatformScoutCandidate,
        source: &PlatformDiscoveryInput,
        scope: &CoreTaskScope,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
        policy: &PlatformVerificationPolicy,
        comparison: Option<&PlatformComparisonMeasurement>,
    ) -> Result<PlatformVerificationReport, PlatformVerificationError> {
        if !candidate.has_valid_digest()
            || !source.has_valid_commitment()
            || policy.digest != policy.compute_digest()
        {
            return Err(PlatformVerificationError::InvalidCandidate);
        }
        if scope.tenant_id() != candidate.tenant_id
            || scope.job_id() != candidate.job_id
            || scope.grant_id() != candidate.grant_id
            || scope.authority_ref() != candidate.authority_ref
            || source.tenant_id != scope.tenant_id()
        {
            return Err(PlatformVerificationError::ScopeMismatch);
        }
        if !source_matches_candidate(source, candidate)
            || candidate.kind != PlatformScoutCandidateKind::SignalEvidencePacket
            || candidate.eligibility_boundary
                != "not_eligible_for_U13A_U14_requires_platform_independent_verifier"
        {
            return Err(PlatformVerificationError::CandidateEvidenceMismatch);
        }
        if core.scope() != scope
            || core.input_digest() != source.commitment
            || core.attempt_id() != candidate.core_attempt_id
            || core.binding_digest() != candidate.core_binding_digest
            || core.core_run_id() != Some(candidate.core_run_id.as_str())
            || core.output_digest() != Some(candidate.core_output_digest.as_str())
            || core.outcome() != &CoreTaskOutcome::Succeeded
        {
            return Err(PlatformVerificationError::CoreReceiptMismatch);
        }
        if model.scope() != scope
            || model.attempt_id() != candidate.model_attempt_id
            || model.input_commitment() != candidate.model_input_commitment
            || model.policy_digest() != candidate.model_policy_digest
            || model.capability_digest() != candidate.model_capability_digest
            || model.output_digest() != Some(candidate.model_output_digest.as_str())
            || model.outcome() != &ModelOutcome::Succeeded
        {
            return Err(PlatformVerificationError::ModelReceiptMismatch);
        }
        if policy.metric_id != candidate.metric_id
            || policy.metric_version != candidate.metric_version
            || policy.layer != candidate.layer
            || policy.population_ref != candidate.population_ref
            || policy.metric_mapping_digest != candidate.metric_mapping_digest
        {
            return Err(PlatformVerificationError::InvalidPolicy);
        }

        let current_rate = rate_and_coverage(
            candidate.numerator,
            candidate.denominator,
            candidate.missing,
        );
        let Some((current_rate, current_coverage)) = current_rate else {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::InsufficientEvidence,
                PlatformVerificationReason::CurrentSupportBelowMinimum,
                0,
                None,
                comparison,
            ));
        };
        if candidate.denominator < policy.minimum_denominator {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::InsufficientEvidence,
                PlatformVerificationReason::CurrentSupportBelowMinimum,
                current_rate,
                None,
                comparison,
            ));
        }
        if current_coverage < policy.minimum_coverage_basis_points {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::InsufficientEvidence,
                PlatformVerificationReason::CurrentCoverageBelowMinimum,
                current_rate,
                None,
                comparison,
            ));
        }
        let Some(comparison) = comparison else {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::Uncertain,
                PlatformVerificationReason::ComparisonMissing,
                current_rate,
                None,
                comparison,
            ));
        };
        if !comparison_matches_policy(comparison, policy) {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::Uncertain,
                PlatformVerificationReason::MetricMappingMismatch,
                current_rate,
                None,
                Some(comparison),
            ));
        }
        if comparison.tenant_id != candidate.tenant_id {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::Uncertain,
                PlatformVerificationReason::ComparisonTenantMismatch,
                current_rate,
                None,
                Some(comparison),
            ));
        }
        if comparison.window_start_ms >= comparison.window_end_ms
            || comparison.received_as_of_ms < comparison.window_end_ms
        {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::InsufficientEvidence,
                PlatformVerificationReason::ComparisonSupportBelowMinimum,
                current_rate,
                None,
                Some(comparison),
            ));
        }
        if windows_overlap(
            candidate.window_start_ms,
            candidate.window_end_ms,
            comparison.window_start_ms,
            comparison.window_end_ms,
        ) {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::Uncertain,
                PlatformVerificationReason::ComparisonWindowOverlaps,
                current_rate,
                None,
                Some(comparison),
            ));
        }
        let Some((comparison_rate, comparison_coverage)) = rate_and_coverage(
            comparison.numerator,
            comparison.denominator,
            comparison.missing,
        ) else {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::InsufficientEvidence,
                PlatformVerificationReason::ComparisonSupportBelowMinimum,
                current_rate,
                None,
                Some(comparison),
            ));
        };
        if comparison.denominator < policy.minimum_denominator {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::InsufficientEvidence,
                PlatformVerificationReason::ComparisonSupportBelowMinimum,
                current_rate,
                Some(comparison_rate),
                Some(comparison),
            ));
        }
        if comparison_coverage < policy.minimum_coverage_basis_points {
            return Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::InsufficientEvidence,
                PlatformVerificationReason::ComparisonCoverageBelowMinimum,
                current_rate,
                Some(comparison_rate),
                Some(comparison),
            ));
        }

        let raw_change = i32::from(current_rate) - i32::from(comparison_rate);
        let concern_change = match policy.direction_of_concern {
            DirectionOfConcern::Increase => raw_change,
            DirectionOfConcern::Decrease => -raw_change,
        };
        let threshold = i32::from(policy.minimum_directional_change_basis_points);
        if concern_change >= threshold {
            Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::SupportedDescriptive,
                PlatformVerificationReason::DirectionalChangeSupported,
                current_rate,
                Some(comparison_rate),
                Some(comparison),
            ))
        } else if concern_change <= -threshold {
            Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::Refuted,
                PlatformVerificationReason::OppositeDirectionObserved,
                current_rate,
                Some(comparison_rate),
                Some(comparison),
            ))
        } else {
            Ok(report(
                candidate,
                policy,
                PlatformVerificationOutcome::Uncertain,
                PlatformVerificationReason::DirectionalChangeWithinUncertaintyBand,
                current_rate,
                Some(comparison_rate),
                Some(comparison),
            ))
        }
    }
}

fn source_matches_candidate(
    source: &PlatformDiscoveryInput,
    candidate: &PlatformScoutCandidate,
) -> bool {
    source.commitment == candidate.platform_input_commitment
        && source.metric_id == candidate.metric_id
        && source.metric_version == candidate.metric_version
        && source.layer == candidate.layer
        && source.population_ref == candidate.population_ref
        && source.numerator == candidate.numerator
        && source.denominator == candidate.denominator
        && source.missing == candidate.missing
        && source.window_start_ms == candidate.window_start_ms
        && source.window_end_ms == candidate.window_end_ms
        && source.received_as_of_ms == candidate.received_as_of_ms
        && source.source_id == candidate.source_id
        && source.contract_ref == candidate.contract_ref
        && source.batch_digest == candidate.batch_digest
        && source.coverage_evidence_digest == candidate.coverage_evidence_digest
        && source.metric_mapping_digest == candidate.metric_mapping_digest
        && source.mapping_resolution_digest == candidate.mapping_resolution_digest
        && source.projection_digest == candidate.projection_digest
        && source.signal_digest == candidate.platform_signal_digest
}

fn comparison_matches_policy(
    comparison: &PlatformComparisonMeasurement,
    policy: &PlatformVerificationPolicy,
) -> bool {
    comparison.metric_id == policy.metric_id
        && comparison.metric_version == policy.metric_version
        && comparison.layer == policy.layer
        && comparison.population_ref == policy.population_ref
        && comparison.metric_mapping_digest == policy.metric_mapping_digest
        && is_sha256(&comparison.mapping_resolution_digest)
        && is_sha256(&comparison.source_digest)
}

fn rate_and_coverage(numerator: u64, denominator: u64, missing: u64) -> Option<(u16, u16)> {
    let population = denominator.checked_add(missing)?;
    if denominator == 0 || numerator > denominator || population == 0 {
        return None;
    }
    let rate = (u128::from(numerator) * 10_000 / u128::from(denominator)) as u16;
    let coverage = (u128::from(denominator) * 10_000 / u128::from(population)) as u16;
    Some((rate, coverage))
}

fn windows_overlap(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

fn report(
    candidate: &PlatformScoutCandidate,
    policy: &PlatformVerificationPolicy,
    outcome: PlatformVerificationOutcome,
    reason: PlatformVerificationReason,
    current_rate_basis_points: u16,
    comparison_rate_basis_points: Option<u16>,
    comparison: Option<&PlatformComparisonMeasurement>,
) -> PlatformVerificationReport {
    PlatformVerificationReport {
        candidate_digest: candidate.digest.clone(),
        policy_digest: policy.digest.clone(),
        outcome,
        reason,
        current_rate_basis_points,
        comparison_rate_basis_points,
        signed_change_basis_points: comparison_rate_basis_points
            .map(|baseline| i32::from(current_rate_basis_points) - i32::from(baseline)),
        comparison_evidence: comparison.map(comparison_evidence_reference),
        may_claim_cause: false,
        may_claim_business_lift: false,
        may_create_proposal: false,
        may_execute: false,
    }
}

fn comparison_evidence_reference(
    comparison: &PlatformComparisonMeasurement,
) -> ComparisonEvidenceReference {
    ComparisonEvidenceReference {
        metric_id: comparison.metric_id.clone(),
        metric_version: comparison.metric_version,
        layer: comparison.layer,
        population_ref: comparison.population_ref.clone(),
        numerator: comparison.numerator,
        denominator: comparison.denominator,
        missing: comparison.missing,
        window_start_ms: comparison.window_start_ms,
        window_end_ms: comparison.window_end_ms,
        received_as_of_ms: comparison.received_as_of_ms,
        metric_mapping_digest: comparison.metric_mapping_digest.clone(),
        mapping_resolution_digest: comparison.mapping_resolution_digest.clone(),
        source_signal_digest: comparison.source_digest.clone(),
        evidence_digest: comparison_evidence_digest(comparison),
    }
}

fn comparison_evidence_digest(comparison: &PlatformComparisonMeasurement) -> String {
    let bytes = serde_json::to_vec(&ComparisonEvidenceContent {
        tenant_id: &comparison.tenant_id,
        metric_id: &comparison.metric_id,
        metric_version: comparison.metric_version,
        layer: comparison.layer,
        population_ref: &comparison.population_ref,
        numerator: comparison.numerator,
        denominator: comparison.denominator,
        missing: comparison.missing,
        window_start_ms: comparison.window_start_ms,
        window_end_ms: comparison.window_end_ms,
        received_as_of_ms: comparison.received_as_of_ms,
        metric_mapping_digest: &comparison.metric_mapping_digest,
        mapping_resolution_digest: &comparison.mapping_resolution_digest,
        source_signal_digest: &comparison.source_digest,
    })
    .expect("typed comparison evidence commitment is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use crate::core_task::{
        CoreTaskBinding, CoreTaskBindingRegistry, CoreTaskPort, CoreTaskSimulator,
    };
    use crate::model_provider::{
        HmacProjectionBroker, ModelBudgetLimits, ModelCapability, ModelPolicy, ModelPort,
        ModelProvider, ModelProviderSimulator, RedactionPolicy,
    };
    use crate::platform_discovery::{AutonomousScout, PlatformScoutInvocation};

    const TENANT: &str = "bank_demo";
    const METRIC: &str = "attention_run_handoff_rate";
    const POPULATION: &str = "attention_source_runs";
    const CONTRACT_SHA: &str = "53e729d624c8284e906249df84c1a1df84cc8d40";
    const MAPPING: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const RESOLUTION: &str =
        "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    struct Fixture {
        candidate: crate::platform_discovery::PlatformScoutCandidate,
        input: PlatformDiscoveryInput,
        scope: CoreTaskScope,
        core: CoreTaskReceipt,
        model: ModelReceipt,
        policy: PlatformVerificationPolicy,
    }

    fn fixture() -> Fixture {
        fixture_with_attempt("1")
    }

    fn fixture_with_attempt(suffix: &str) -> Fixture {
        let mut input = PlatformDiscoveryInput {
            tenant_id: TENANT.into(),
            metric_id: METRIC.into(),
            metric_version: 1,
            layer: Layer::Tree,
            population_ref: POPULATION.into(),
            numerator: 60,
            denominator: 100,
            missing: 0,
            window_start_ms: 100,
            window_end_ms: 200,
            received_as_of_ms: 201,
            projection_digest: format!("sha256:{}", "a".repeat(64)),
            source_id: "attention-platform".into(),
            contract_ref: "contract:attention-v1".into(),
            batch_digest: format!("sha256:{}", "b".repeat(64)),
            coverage_evidence_digest: format!("sha256:{}", "c".repeat(64)),
            metric_mapping_digest: MAPPING.into(),
            mapping_resolution_digest: RESOLUTION.into(),
            signal_digest: format!("sha256:{}", "f".repeat(64)),
            commitment: String::new(),
        };
        input.commitment = super::super::input_digest(&input);

        let scope = CoreTaskScope::new(TENANT, "job-1", "grant-1", "authority-1").unwrap();
        let binding = CoreTaskBinding::new(
            "platform-scout",
            "platform-scout-release",
            "0.5.0",
            CONTRACT_SHA,
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
        let model_policy = ModelPolicy::with_budget(
            "platform-scout-policy",
            capability,
            "platform_signal_investigation",
            RedactionPolicy::RejectMarkedInput,
            0,
            1_000,
            ModelBudgetLimits::new(4_096, 1_024, 100_000).unwrap(),
        )
        .unwrap();
        let mut broker =
            HmacProjectionBroker::new_for_test(b"test-only-platform-scout-projection-key-32-bytes")
                .unwrap();
        let invocation = PlatformScoutInvocation::prepare(
            scope.clone(),
            &input,
            binding.clone(),
            format!("core-attempt-{suffix}"),
            model_policy,
            format!("model-attempt-{suffix}"),
            &mut broker,
        )
        .unwrap();
        let mut core_adapter =
            CoreTaskSimulator::new(CoreTaskBindingRegistry::new(vec![binding]).unwrap());
        core_adapter
            .script_success(
                format!("core-run-{suffix}"),
                format!("sha256:{}", "1".repeat(64)),
            )
            .unwrap();
        let core = core_adapter
            .invoke(invocation.core_invocation().clone())
            .unwrap();
        let mut model_adapter =
            ModelProviderSimulator::new(invocation.model_invocation().policy().clone());
        model_adapter.script_success("aggregate observation", "request-1");
        let model = model_adapter
            .invoke(invocation.model_invocation().clone())
            .unwrap();
        let crate::platform_discovery::PlatformScoutResult::Candidate(candidate) =
            AutonomousScout::discover_platform(&input, &invocation, &core, &model).unwrap()
        else {
            panic!("fixture must generate a candidate");
        };
        let policy = PlatformVerificationPolicy::new(
            "handoff-rate-trend",
            1,
            METRIC,
            1,
            Layer::Tree,
            POPULATION,
            MAPPING,
            DirectionOfConcern::Increase,
            20,
            9_000,
            500,
        )
        .unwrap();
        Fixture {
            candidate: *candidate,
            input,
            scope,
            core,
            model,
            policy,
        }
    }

    fn comparison(
        numerator: u64,
        denominator: u64,
        start: i64,
        end: i64,
    ) -> PlatformComparisonMeasurement {
        comparison_with_identity(
            TENANT,
            METRIC,
            MAPPING,
            numerator,
            denominator,
            0,
            start,
            end,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn comparison_with_identity(
        tenant_id: &str,
        metric_id: &str,
        mapping_digest: &str,
        numerator: u64,
        denominator: u64,
        missing: u64,
        start: i64,
        end: i64,
    ) -> PlatformComparisonMeasurement {
        let mut input = PlatformDiscoveryInput {
            tenant_id: tenant_id.into(),
            metric_id: metric_id.into(),
            metric_version: 1,
            layer: Layer::Tree,
            population_ref: POPULATION.into(),
            numerator,
            denominator,
            missing,
            window_start_ms: start,
            window_end_ms: end,
            received_as_of_ms: end + 1,
            projection_digest: format!("sha256:{}", "a".repeat(64)),
            source_id: "attention-platform".into(),
            contract_ref: "contract:attention-v1".into(),
            batch_digest: format!("sha256:{}", "b".repeat(64)),
            coverage_evidence_digest: format!("sha256:{}", "c".repeat(64)),
            metric_mapping_digest: mapping_digest.into(),
            mapping_resolution_digest: format!("sha256:{}", "e".repeat(64)),
            signal_digest: format!("sha256:{}", "9".repeat(64)),
            commitment: String::new(),
        };
        input.commitment = super::super::input_digest(&input);
        PlatformComparisonMeasurement::from_discovery_input(&input).unwrap()
    }

    #[test]
    fn verifies_only_scope_bound_non_overlapping_descriptive_trends() {
        let f = fixture();
        let previous = comparison(40, 100, 0, 100);
        let report = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            Some(&previous),
        )
        .unwrap();
        assert_eq!(
            report.outcome(),
            PlatformVerificationOutcome::SupportedDescriptive
        );
        assert_eq!(
            report.reason(),
            PlatformVerificationReason::DirectionalChangeSupported
        );
        assert_eq!(report.signed_change_basis_points, Some(2_000));
        assert!(!report.may_claim_cause);
        assert!(!report.may_claim_business_lift);
        assert!(!report.may_create_proposal);
        assert!(!report.may_execute);
        let encoded = serde_json::to_value(&report).unwrap();
        let comparison = &encoded["comparison_evidence"];
        assert_eq!(comparison["window_start_ms"], 0);
        assert_eq!(comparison["window_end_ms"], 100);
        assert_eq!(comparison["received_as_of_ms"], 101);
        assert_eq!(comparison["numerator"], 40);
        assert_eq!(comparison["denominator"], 100);
        assert!(
            comparison["evidence_digest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );

        let mut later_receipt = previous.clone();
        later_receipt.received_as_of_ms += 1;
        assert_ne!(
            comparison_evidence_digest(&previous),
            comparison_evidence_digest(&later_receipt),
            "as-of provenance must be included in the immutable comparison digest"
        );
    }

    #[test]
    fn missing_or_overlapping_comparison_never_confirms_a_trend() {
        let f = fixture();
        let absent = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            None,
        )
        .unwrap();
        assert_eq!(absent.outcome(), PlatformVerificationOutcome::Uncertain);
        assert_eq!(
            absent.reason(),
            PlatformVerificationReason::ComparisonMissing
        );

        let overlap = comparison(40, 100, 150, 250);
        let report = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            Some(&overlap),
        )
        .unwrap();
        assert_eq!(report.outcome(), PlatformVerificationOutcome::Uncertain);
        assert_eq!(
            report.reason(),
            PlatformVerificationReason::ComparisonWindowOverlaps
        );
    }

    #[test]
    fn opposite_or_small_changes_are_refuted_or_uncertain_not_supported() {
        let f = fixture();
        let opposite = comparison(80, 100, 0, 100);
        let refuted = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            Some(&opposite),
        )
        .unwrap();
        assert_eq!(refuted.outcome(), PlatformVerificationOutcome::Refuted);
        assert_eq!(
            refuted.reason(),
            PlatformVerificationReason::OppositeDirectionObserved
        );

        let within_band = comparison(58, 100, 0, 100);
        let uncertain = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            Some(&within_band),
        )
        .unwrap();
        assert_eq!(uncertain.outcome(), PlatformVerificationOutcome::Uncertain);
        assert_eq!(
            uncertain.reason(),
            PlatformVerificationReason::DirectionalChangeWithinUncertaintyBand
        );
    }

    #[test]
    fn mapping_mismatch_and_cross_tenant_comparison_cannot_support_the_claim() {
        let f = fixture();
        let other_metric =
            comparison_with_identity(TENANT, "different_metric", MAPPING, 40, 100, 0, 0, 100);
        let mismatched = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            Some(&other_metric),
        )
        .unwrap();
        assert_eq!(mismatched.outcome(), PlatformVerificationOutcome::Uncertain);
        assert_eq!(
            mismatched.reason(),
            PlatformVerificationReason::MetricMappingMismatch
        );

        let foreign_tenant =
            comparison_with_identity("other_bank", METRIC, MAPPING, 40, 100, 0, 0, 100);
        let foreign = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            Some(&foreign_tenant),
        )
        .unwrap();
        assert_eq!(foreign.outcome(), PlatformVerificationOutcome::Uncertain);
        assert_eq!(
            foreign.reason(),
            PlatformVerificationReason::ComparisonTenantMismatch
        );
    }

    #[test]
    fn weak_comparison_coverage_and_wrong_receipts_fail_closed() {
        let f = fixture();
        let weak = comparison_with_identity(TENANT, METRIC, MAPPING, 40, 100, 20, 0, 100);
        let low_coverage = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            Some(&weak),
        )
        .unwrap();
        assert_eq!(
            low_coverage.outcome(),
            PlatformVerificationOutcome::InsufficientEvidence
        );
        assert_eq!(
            low_coverage.reason(),
            PlatformVerificationReason::ComparisonCoverageBelowMinimum
        );

        let another_attempt = fixture_with_attempt("2");
        assert_eq!(
            PlatformIndependentVerifier::verify(
                &f.candidate,
                &f.input,
                &f.scope,
                &another_attempt.core,
                &f.model,
                &f.policy,
                None,
            ),
            Err(PlatformVerificationError::CoreReceiptMismatch)
        );
        assert_eq!(
            PlatformIndependentVerifier::verify(
                &f.candidate,
                &f.input,
                &f.scope,
                &f.core,
                &another_attempt.model,
                &f.policy,
                None,
            ),
            Err(PlatformVerificationError::ModelReceiptMismatch)
        );
    }

    #[test]
    fn scope_and_tampering_are_rejected_before_report_creation() {
        let f = fixture();
        let foreign = CoreTaskScope::new("other_bank", "job-1", "grant-1", "authority-1").unwrap();
        assert_eq!(
            PlatformIndependentVerifier::verify(
                &f.candidate,
                &f.input,
                &foreign,
                &f.core,
                &f.model,
                &f.policy,
                None,
            ),
            Err(PlatformVerificationError::ScopeMismatch)
        );
        let mut tampered = f.candidate.clone();
        tampered.numerator += 1;
        assert_eq!(
            PlatformIndependentVerifier::verify(
                &tampered, &f.input, &f.scope, &f.core, &f.model, &f.policy, None,
            ),
            Err(PlatformVerificationError::InvalidCandidate)
        );
    }

    #[test]
    fn insufficient_denominator_and_coverage_are_not_reported_as_refutation() {
        let mut f = fixture();
        f.policy.minimum_denominator = 101;
        f.policy.digest = f.policy.compute_digest();
        let report = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            None,
        )
        .unwrap();
        assert_eq!(
            report.outcome(),
            PlatformVerificationOutcome::InsufficientEvidence
        );
        assert_eq!(
            report.reason(),
            PlatformVerificationReason::CurrentSupportBelowMinimum
        );
    }

    #[test]
    fn a_zero_coverage_policy_is_rejected() {
        assert_eq!(
            PlatformVerificationPolicy::new(
                "handoff-rate-trend",
                1,
                METRIC,
                1,
                Layer::Tree,
                POPULATION,
                MAPPING,
                DirectionOfConcern::Increase,
                1,
                0,
                500,
            ),
            Err(PlatformVerificationError::InvalidPolicy)
        );
    }

    #[test]
    fn privacy_safe_report_omits_tenant_scope_and_model_text() {
        let f = fixture();
        let report = PlatformIndependentVerifier::verify(
            &f.candidate,
            &f.input,
            &f.scope,
            &f.core,
            &f.model,
            &f.policy,
            None,
        )
        .unwrap();
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains(TENANT));
        assert!(!serialized.contains("grant-1"));
        assert!(!serialized.contains("authority-1"));
        assert!(!serialized.contains("aggregate observation"));
    }
}
