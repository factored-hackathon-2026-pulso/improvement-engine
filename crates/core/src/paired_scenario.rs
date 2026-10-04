//! Deterministic comparison of caller-supplied fixture observations.
//!
//! The observations do not authenticate or prove sandbox execution. This is
//! not an Agent Core evaluator and does not estimate business lift.

use std::collections::BTreeMap;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The two required outcomes in the checked-in C-10 gate envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Safety,
    Improvement,
}

/// Status already computed by the corresponding evaluator; this module only
/// combines the two statuses and does not authenticate their evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Pass,
    Fail,
    NotEvaluable,
}

/// Bounded reason codes keep arbitrary or customer-supplied free text out of
/// the gate envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateReason {
    CandidateRegression,
    InfrastructureUnavailable,
    InsufficientEvidence,
    EvaluationFailed,
}

/// One typed gate outcome, optionally with a bounded diagnostic reason code.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GateObservation {
    gate: GateKind,
    status: GateStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<GateReason>,
}

impl GateObservation {
    #[must_use]
    pub fn new(gate: GateKind, status: GateStatus, reason: Option<GateReason>) -> Self {
        Self {
            gate,
            status,
            reason,
        }
    }
}

/// Result of reducing both required gate outcomes. A pass means only that the
/// paired fixture comparison found no regression and the supplied improvement
/// gate passed. It does not prove sandbox execution, authenticate the paired
/// observation producer, or measure business lift; `quality_claims` is always
/// `forbidden`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CombinedGateResult {
    contract_version: &'static str,
    run_id: String,
    base_ref: String,
    candidate_ref: String,
    suite_digest: String,
    verdict: GateVerdict,
    gates: [GateObservation; 2],
    judge_actor: String,
    quality_claims: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateVerdict {
    Pass,
    Fail,
    NotEvaluable,
}

impl GateVerdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::NotEvaluable => "not_evaluable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateResultError {
    InvalidMetadata,
    MissingGate(GateKind),
    DuplicateGate(GateKind),
    PairIntegrityDenied,
    PairStatusMismatch,
    ReasonStatusMismatch,
    SameArtifact,
}

/// Reduce the paired-scenario safety result and the independently supplied
/// improvement result into the C-10 wire shape. Safety is bound to the sealed
/// pair receipt, so a candidate regression or failed/incomparable pair cannot
/// be overridden by a caller-supplied `pass`. The paired receipt itself still
/// contains caller-supplied fixture observations: it does not authenticate a
/// runner or prove sandbox execution.
pub fn combine_gate_results(
    pair: &PairedEvaluationReceipt,
    suite_digest: impl Into<String>,
    judge_actor: impl Into<String>,
    observations: impl IntoIterator<Item = GateObservation>,
) -> Result<CombinedGateResult, GateResultError> {
    let suite_digest = suite_digest.into();
    let judge_actor = judge_actor.into();
    if !is_digest(&suite_digest) || !valid_c10_id(&judge_actor) {
        return Err(GateResultError::InvalidMetadata);
    }
    if !pair.validate_integrity() {
        return Err(GateResultError::PairIntegrityDenied);
    }
    if pair.baseline_digest == pair.candidate_digest {
        return Err(GateResultError::SameArtifact);
    }

    let (expected_safety, expected_reason) = match pair.verdict {
        PairVerdict::NoRegressionObserved => (GateStatus::Pass, None),
        PairVerdict::CandidateRegression => {
            (GateStatus::Fail, Some(GateReason::CandidateRegression))
        }
        PairVerdict::NotComparable => (
            GateStatus::NotEvaluable,
            Some(GateReason::InsufficientEvidence),
        ),
        PairVerdict::FailedInfra => (
            GateStatus::NotEvaluable,
            Some(GateReason::InfrastructureUnavailable),
        ),
    };

    let (mut safety, mut improvement) = (None, None);
    for observation in observations {
        let kind = observation.gate;
        let slot = match observation.gate {
            GateKind::Safety => &mut safety,
            GateKind::Improvement => &mut improvement,
        };
        if slot.is_some() {
            return Err(GateResultError::DuplicateGate(kind));
        }
        *slot = Some(observation);
    }
    let safety = safety.ok_or(GateResultError::MissingGate(GateKind::Safety))?;
    let improvement = improvement.ok_or(GateResultError::MissingGate(GateKind::Improvement))?;
    if safety.status != expected_safety {
        return Err(GateResultError::PairStatusMismatch);
    }
    if safety.reason != expected_reason || !valid_reason_status(&improvement) {
        return Err(GateResultError::ReasonStatusMismatch);
    }
    let verdict = if safety.status == GateStatus::Fail || improvement.status == GateStatus::Fail {
        GateVerdict::Fail
    } else if safety.status == GateStatus::NotEvaluable
        || improvement.status == GateStatus::NotEvaluable
    {
        GateVerdict::NotEvaluable
    } else {
        GateVerdict::Pass
    };

    Ok(CombinedGateResult {
        contract_version: "engine-steps-pack/0",
        run_id: c10_run_id(&pair.evaluation_id),
        base_ref: c10_artifact_ref(&pair.baseline_digest),
        candidate_ref: c10_artifact_ref(&pair.candidate_digest),
        suite_digest,
        verdict,
        gates: [safety, improvement],
        judge_actor,
        quality_claims: "forbidden",
    })
}

fn valid_reason_status(observation: &GateObservation) -> bool {
    match observation.reason {
        None => true,
        Some(GateReason::CandidateRegression) => {
            observation.gate == GateKind::Safety && observation.status == GateStatus::Fail
        }
        Some(GateReason::InfrastructureUnavailable | GateReason::InsufficientEvidence) => {
            observation.status == GateStatus::NotEvaluable
        }
        Some(GateReason::EvaluationFailed) => observation.status == GateStatus::Fail,
    }
}

fn c10_run_id(evaluation_id: &str) -> String {
    // PairPlan already validates this as a full SHA-256 reference. The C-10
    // run-id field allows at most 64 characters, so retain 248 bits of it.
    format!("r-{}", &evaluation_id[7..69])
}

fn c10_artifact_ref(digest: &str) -> String {
    let hex = digest.strip_prefix("sha256:").unwrap_or_default();
    // This reducer has fixture-only authority; make the non-registry alias
    // explicit instead of implying an Agent Core catalogue readback.
    format!("fixture_bundle:sha256-{hex}@1")
}

fn valid_c10_id(value: &str) -> bool {
    (3..=64).contains(&value.len())
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PairError {
    InvalidPlan,
    BindingMismatch { arm: &'static str },
    SharedArm,
    ArtifactDigestMismatch { arm: &'static str },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PairVerdict {
    CandidateRegression,
    /// The supplied synthetic projections match the oracle; this does not
    /// prove that sandbox execution occurred or that business outcomes improve.
    NoRegressionObserved,
    NotComparable,
    FailedInfra,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InfrastructureFailure {
    SandboxUnavailable,
    HarnessUnavailable,
    Timeout,
    DependencyUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComparedArm {
    Baseline,
    Candidate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ObservationStatus {
    Completed,
    FailedInfra(InfrastructureFailure),
}

/// One immutable comparison definition. The oracle and all artifact digests
/// are fixed before either arm is evaluated.
#[derive(Clone, Eq, PartialEq)]
pub struct PairPlan {
    evaluation_id: String,
    scenario_id: String,
    fixture_id: String,
    seed_digest: String,
    baseline_digest: String,
    candidate_digest: String,
    oracle: BTreeMap<ProjectionField, ProjectionValue>,
    oracle_digest: String,
    plan_digest: String,
}

impl PairPlan {
    pub fn new(
        evaluation_id: impl Into<String>,
        scenario_id: impl Into<String>,
        fixture_id: impl Into<String>,
        seed_digest: impl Into<String>,
        baseline_digest: impl Into<String>,
        candidate_digest: impl Into<String>,
        oracle: BTreeMap<ProjectionField, ProjectionValue>,
    ) -> Result<Self, PairError> {
        let evaluation_id = evaluation_id.into();
        let scenario_id = scenario_id.into();
        let fixture_id = fixture_id.into();
        let seed_digest = seed_digest.into();
        let baseline_digest = baseline_digest.into();
        let candidate_digest = candidate_digest.into();
        if !is_digest(&evaluation_id)
            || !is_digest(&scenario_id)
            || !is_digest(&fixture_id)
            || !is_digest(&seed_digest)
            || !is_digest(&baseline_digest)
            || !is_digest(&candidate_digest)
            || oracle.is_empty()
        {
            return Err(PairError::InvalidPlan);
        }
        let oracle_digest = map_digest(&oracle);
        let plan_digest = digest_parts([
            evaluation_id.as_str(),
            scenario_id.as_str(),
            fixture_id.as_str(),
            seed_digest.as_str(),
            baseline_digest.as_str(),
            candidate_digest.as_str(),
            oracle_digest.as_str(),
        ]);
        Ok(Self {
            evaluation_id,
            scenario_id,
            fixture_id,
            seed_digest,
            baseline_digest,
            candidate_digest,
            oracle,
            oracle_digest,
            plan_digest,
        })
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.plan_digest
    }

    #[must_use]
    pub fn seed_digest(&self) -> &str {
        &self.seed_digest
    }
}

/// Caller-provided fixture observation. This constructor does not authenticate
/// a runner or prove sandbox isolation. `state` must be a minimized,
/// allowlisted projection, never a complete bank/customer record. Evidence
/// digests are optional, non-authenticated fixture references; an empty list
/// does not prove that execution occurred.
#[derive(Clone, Eq, PartialEq)]
pub struct ArmObservation {
    binding: ArmBinding,
    /// Exact allowlisted projection compared to the sealed scenario oracle.
    state: Option<BTreeMap<ProjectionField, ProjectionValue>>,
    fixture_evidence_digests: Vec<String>,
    observation_digest: String,
    status: ObservationStatus,
}

/// Immutable identity and content pin returned by one sandbox arm.
#[derive(Clone, Eq, PartialEq)]
pub struct ArmBinding {
    pub evaluation_id: String,
    pub scenario_id: String,
    pub fixture_id: String,
    pub arm_id: String,
    pub seed_digest: String,
    pub artifact_digest: String,
}

impl ArmObservation {
    /// `fixture_evidence_digests` may be empty. These are optional,
    /// non-authenticated fixture references; empty does not prove execution.
    #[must_use]
    pub fn completed(
        binding: ArmBinding,
        state: BTreeMap<ProjectionField, ProjectionValue>,
        fixture_evidence_digests: Vec<String>,
    ) -> Self {
        let state_digest = map_digest(&state);
        let observation_digest = digest_parts(
            [
                binding.evaluation_id.as_str(),
                binding.scenario_id.as_str(),
                binding.fixture_id.as_str(),
                binding.arm_id.as_str(),
                binding.seed_digest.as_str(),
                binding.artifact_digest.as_str(),
                state_digest.as_str(),
            ]
            .into_iter()
            .chain(fixture_evidence_digests.iter().map(String::as_str)),
        );
        Self {
            binding,
            state: Some(state),
            fixture_evidence_digests,
            observation_digest,
            status: ObservationStatus::Completed,
        }
    }

    #[must_use]
    pub fn failed_infra(binding: ArmBinding, reason: InfrastructureFailure) -> Self {
        let reason_code = infra_text(&reason);
        let observation_digest = digest_parts([
            binding.evaluation_id.as_str(),
            binding.scenario_id.as_str(),
            binding.fixture_id.as_str(),
            binding.arm_id.as_str(),
            binding.seed_digest.as_str(),
            binding.artifact_digest.as_str(),
            reason_code,
        ]);
        Self {
            binding,
            state: None,
            fixture_evidence_digests: Vec::new(),
            observation_digest,
            status: ObservationStatus::FailedInfra(reason),
        }
    }
}

/// Receipt identifiers are opaque references. Do not derive `Debug`: even
/// validated digest-shaped IDs could be hashes of sensitive source values.
/// Fields are private so external callers cannot mutate claims while keeping
/// a stale evidence digest; use the read-only accessors and
/// [`Self::validate_integrity`].
#[derive(Clone, Eq, PartialEq)]
pub struct PairedEvaluationReceipt {
    evaluation_id: String,
    scenario_id: String,
    fixture_id: String,
    baseline_arm_id: String,
    candidate_arm_id: String,
    seed_digest: String,
    plan_digest: String,
    baseline_digest: String,
    candidate_digest: String,
    oracle_digest: String,
    baseline_observation_digest: String,
    candidate_observation_digest: String,
    baseline_matches_oracle: Option<bool>,
    candidate_matches_oracle: Option<bool>,
    verdict: PairVerdict,
    infrastructure_failures: Vec<ArmInfrastructureFailure>,
    evidence_digest: String,
    /// This comparison is based on caller-provided fixture observations only.
    authority: ComparisonAuthority,
    /// Always false: scenario-oracle outcomes do not establish business lift.
    business_lift_measured: bool,
}

impl PairedEvaluationReceipt {
    #[must_use]
    pub fn evaluation_id(&self) -> &str {
        &self.evaluation_id
    }
    #[must_use]
    pub fn scenario_id(&self) -> &str {
        &self.scenario_id
    }
    #[must_use]
    pub fn fixture_id(&self) -> &str {
        &self.fixture_id
    }
    #[must_use]
    pub fn baseline_arm_id(&self) -> &str {
        &self.baseline_arm_id
    }
    #[must_use]
    pub fn candidate_arm_id(&self) -> &str {
        &self.candidate_arm_id
    }
    #[must_use]
    pub fn seed_digest(&self) -> &str {
        &self.seed_digest
    }
    #[must_use]
    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
    #[must_use]
    pub fn baseline_digest(&self) -> &str {
        &self.baseline_digest
    }
    #[must_use]
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }
    #[must_use]
    pub fn oracle_digest(&self) -> &str {
        &self.oracle_digest
    }
    #[must_use]
    pub fn baseline_observation_digest(&self) -> &str {
        &self.baseline_observation_digest
    }
    #[must_use]
    pub fn candidate_observation_digest(&self) -> &str {
        &self.candidate_observation_digest
    }
    #[must_use]
    pub fn baseline_matches_oracle(&self) -> Option<bool> {
        self.baseline_matches_oracle
    }
    #[must_use]
    pub fn candidate_matches_oracle(&self) -> Option<bool> {
        self.candidate_matches_oracle
    }
    #[must_use]
    pub fn verdict(&self) -> &PairVerdict {
        &self.verdict
    }
    #[must_use]
    pub fn infrastructure_failures(&self) -> &[ArmInfrastructureFailure] {
        &self.infrastructure_failures
    }
    #[must_use]
    pub fn evidence_digest(&self) -> &str {
        &self.evidence_digest
    }
    #[must_use]
    pub fn authority(&self) -> &ComparisonAuthority {
        &self.authority
    }
    #[must_use]
    pub fn business_lift_measured(&self) -> bool {
        self.business_lift_measured
    }

    /// Recompute a digest over every receipt claim, excluding only the digest
    /// itself. A valid digest binds fixture evidence and does not authenticate
    /// execution or establish business lift.
    #[must_use]
    pub fn validate_integrity(&self) -> bool {
        self.evidence_digest == self.expected_evidence_digest()
    }

    fn expected_evidence_digest(&self) -> String {
        let mut parts = vec![
            self.evaluation_id.clone(),
            self.scenario_id.clone(),
            self.fixture_id.clone(),
            self.baseline_arm_id.clone(),
            self.candidate_arm_id.clone(),
            self.seed_digest.clone(),
            self.plan_digest.clone(),
            self.baseline_digest.clone(),
            self.candidate_digest.clone(),
            self.oracle_digest.clone(),
            self.baseline_observation_digest.clone(),
            self.candidate_observation_digest.clone(),
            option_bool_text(self.baseline_matches_oracle).to_owned(),
            option_bool_text(self.candidate_matches_oracle).to_owned(),
            verdict_text(&self.verdict).to_owned(),
            authority_text(&self.authority).to_owned(),
            bool_text(self.business_lift_measured).to_owned(),
        ];
        for failure in &self.infrastructure_failures {
            parts.push(arm_text(&failure.arm).to_owned());
            parts.push(infra_text(&failure.failure).to_owned());
        }
        digest_parts(parts.iter().map(String::as_str))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComparisonAuthority {
    FixtureOnlyUnverified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArmInfrastructureFailure {
    pub arm: ComparedArm,
    pub failure: InfrastructureFailure,
}

pub fn evaluate_pair(
    plan: &PairPlan,
    baseline: &ArmObservation,
    candidate: &ArmObservation,
) -> Result<PairedEvaluationReceipt, PairError> {
    validate_binding(plan, baseline, "baseline")?;
    validate_binding(plan, candidate, "candidate")?;
    if baseline.binding.arm_id == candidate.binding.arm_id {
        return Err(PairError::SharedArm);
    }
    if baseline.binding.artifact_digest != plan.baseline_digest {
        return Err(PairError::ArtifactDigestMismatch { arm: "baseline" });
    }
    if candidate.binding.artifact_digest != plan.candidate_digest {
        return Err(PairError::ArtifactDigestMismatch { arm: "candidate" });
    }
    let baseline_matches_oracle = matches_oracle(plan, baseline);
    let candidate_matches_oracle = matches_oracle(plan, candidate);
    let mut infrastructure_failures = Vec::new();
    if let ObservationStatus::FailedInfra(failure) = &baseline.status {
        infrastructure_failures.push(ArmInfrastructureFailure {
            arm: ComparedArm::Baseline,
            failure: failure.clone(),
        });
    }
    if let ObservationStatus::FailedInfra(failure) = &candidate.status {
        infrastructure_failures.push(ArmInfrastructureFailure {
            arm: ComparedArm::Candidate,
            failure: failure.clone(),
        });
    }
    let verdict = if !infrastructure_failures.is_empty() {
        PairVerdict::FailedInfra
    } else if baseline_matches_oracle == Some(true) && candidate_matches_oracle == Some(false) {
        PairVerdict::CandidateRegression
    } else if baseline_matches_oracle == Some(true) && candidate_matches_oracle == Some(true) {
        PairVerdict::NoRegressionObserved
    } else {
        PairVerdict::NotComparable
    };
    let mut receipt = PairedEvaluationReceipt {
        evaluation_id: plan.evaluation_id.clone(),
        scenario_id: plan.scenario_id.clone(),
        fixture_id: plan.fixture_id.clone(),
        baseline_arm_id: baseline.binding.arm_id.clone(),
        candidate_arm_id: candidate.binding.arm_id.clone(),
        seed_digest: plan.seed_digest.clone(),
        plan_digest: plan.plan_digest.clone(),
        baseline_digest: plan.baseline_digest.clone(),
        candidate_digest: plan.candidate_digest.clone(),
        oracle_digest: plan.oracle_digest.clone(),
        baseline_observation_digest: baseline.observation_digest.clone(),
        candidate_observation_digest: candidate.observation_digest.clone(),
        baseline_matches_oracle,
        candidate_matches_oracle,
        verdict,
        infrastructure_failures,
        evidence_digest: String::new(),
        authority: ComparisonAuthority::FixtureOnlyUnverified,
        business_lift_measured: false,
    };
    receipt.evidence_digest = receipt.expected_evidence_digest();
    Ok(receipt)
}

fn validate_binding(
    plan: &PairPlan,
    observation: &ArmObservation,
    arm: &'static str,
) -> Result<(), PairError> {
    if !is_digest(&observation.binding.evaluation_id)
        || !is_digest(&observation.binding.scenario_id)
        || !is_digest(&observation.binding.fixture_id)
        || !is_digest(&observation.binding.arm_id)
        || observation.binding.evaluation_id != plan.evaluation_id
        || observation.binding.scenario_id != plan.scenario_id
        || observation.binding.fixture_id != plan.fixture_id
        || observation.binding.seed_digest != plan.seed_digest
        || observation
            .fixture_evidence_digests
            .iter()
            .any(|digest| !is_digest(digest))
    {
        return Err(PairError::BindingMismatch { arm });
    }
    Ok(())
}

fn matches_oracle(plan: &PairPlan, observation: &ArmObservation) -> Option<bool> {
    observation
        .state
        .as_ref()
        .map(|state| state == &plan.oracle)
}

fn option_bool_text(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "not_evaluated",
    }
}

fn is_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// Closed vocabulary of minimized synthetic scenario output fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ProjectionField {
    PaymentStatus,
    CaseOutcome,
    EscalationState,
}

impl ProjectionField {
    fn as_str(self) -> &'static str {
        match self {
            Self::PaymentStatus => "payment_status",
            Self::CaseOutcome => "case_outcome",
            Self::EscalationState => "escalation_state",
        }
    }
}

/// Closed vocabulary of synthetic scenario outputs. Arbitrary strings are
/// not representable as projection values.
///
/// ```compile_fail
/// use std::collections::BTreeMap;
/// use improvement_engine_core::paired_scenario::{
///     ArmBinding, ArmObservation, ProjectionField, ProjectionValue,
/// };
/// let binding = ArmBinding {
///     evaluation_id: "e1".into(), scenario_id: "s1".into(),
///     fixture_id: "f1".into(), arm_id: "a1".into(),
///     seed_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
///     artifact_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
/// };
/// let raw_customer_data = BTreeMap::from([("customer_name".to_owned(), "Ada".to_owned())]);
/// let _observation = ArmObservation::completed(binding, raw_customer_data, vec![]);
/// ```
///
/// `ArmObservation` intentionally has no `Debug` implementation, avoiding
/// accidental formatting of caller-provided values.
///
/// ```compile_fail
/// use std::collections::BTreeMap;
/// use improvement_engine_core::paired_scenario::{ArmBinding, ArmObservation, ProjectionField, ProjectionValue};
/// let binding = ArmBinding {
///     evaluation_id: "e1".into(), scenario_id: "s1".into(), fixture_id: "f1".into(), arm_id: "a1".into(),
///     seed_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
///     artifact_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
/// };
/// let state = BTreeMap::from([(ProjectionField::PaymentStatus, ProjectionValue::Settled)]);
/// let observation = ArmObservation::completed(binding, state, vec![]);
/// println!("{observation:?}");
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionValue {
    Settled,
    Pending,
    Rejected,
    Resolved,
    Unresolved,
    Escalated,
    NotEscalated,
    Unknown,
}

impl ProjectionValue {
    fn as_str(self) -> &'static str {
        match self {
            Self::Settled => "settled",
            Self::Pending => "pending",
            Self::Rejected => "rejected",
            Self::Resolved => "resolved",
            Self::Unresolved => "unresolved",
            Self::Escalated => "escalated",
            Self::NotEscalated => "not_escalated",
            Self::Unknown => "unknown",
        }
    }
}

fn map_digest(values: &BTreeMap<ProjectionField, ProjectionValue>) -> String {
    digest_parts(
        values
            .iter()
            .flat_map(|(key, value)| [key.as_str(), value.as_str()]),
    )
}

fn digest_parts<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}

fn verdict_text(verdict: &PairVerdict) -> &'static str {
    match verdict {
        PairVerdict::CandidateRegression => "candidate_regression",
        PairVerdict::NoRegressionObserved => "no_regression_observed",
        PairVerdict::NotComparable => "not_comparable",
        PairVerdict::FailedInfra => "failed_infra",
    }
}

fn infra_text(failure: &InfrastructureFailure) -> &'static str {
    match failure {
        InfrastructureFailure::SandboxUnavailable => "sandbox_unavailable",
        InfrastructureFailure::HarnessUnavailable => "harness_unavailable",
        InfrastructureFailure::Timeout => "timeout",
        InfrastructureFailure::DependencyUnavailable => "dependency_unavailable",
    }
}

fn arm_text(arm: &ComparedArm) -> &'static str {
    match arm {
        ComparedArm::Baseline => "baseline",
        ComparedArm::Candidate => "candidate",
    }
}

fn authority_text(authority: &ComparisonAuthority) -> &'static str {
    match authority {
        ComparisonAuthority::FixtureOnlyUnverified => "fixture_only_unverified",
    }
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

#[cfg(test)]
mod receipt_integrity_tests {
    use super::*;

    fn fixture_receipt() -> PairedEvaluationReceipt {
        let id = |digit: char| format!("sha256:{}", digit.to_string().repeat(64));
        let plan = PairPlan::new(
            id('1'),
            id('2'),
            id('3'),
            id('a'),
            id('b'),
            id('c'),
            BTreeMap::from([(ProjectionField::PaymentStatus, ProjectionValue::Settled)]),
        )
        .unwrap();
        let arm = |arm_id: char, artifact: char| {
            ArmObservation::completed(
                ArmBinding {
                    evaluation_id: id('1'),
                    scenario_id: id('2'),
                    fixture_id: id('3'),
                    arm_id: id(arm_id),
                    seed_digest: id('a'),
                    artifact_digest: id(artifact),
                },
                BTreeMap::from([(ProjectionField::PaymentStatus, ProjectionValue::Settled)]),
                vec![],
            )
        };
        evaluate_pair(&plan, &arm('4', 'b'), &arm('5', 'c')).unwrap()
    }

    #[test]
    fn receipt_integrity_digest_rejects_mutation_of_every_claim_class() {
        let original = fixture_receipt();
        assert!(original.validate_integrity());

        let mut tampered = original.clone();
        tampered.evaluation_id.push('x');
        assert!(!tampered.validate_integrity());

        let mut tampered = original.clone();
        tampered.baseline_matches_oracle = Some(false);
        assert!(!tampered.validate_integrity());

        let mut tampered = original.clone();
        tampered.verdict = PairVerdict::CandidateRegression;
        assert!(!tampered.validate_integrity());

        let mut tampered = original.clone();
        tampered
            .infrastructure_failures
            .push(ArmInfrastructureFailure {
                arm: ComparedArm::Candidate,
                failure: InfrastructureFailure::Timeout,
            });
        assert!(!tampered.validate_integrity());

        let mut tampered = original;
        tampered.business_lift_measured = true;
        assert!(!tampered.validate_integrity());
    }
}
