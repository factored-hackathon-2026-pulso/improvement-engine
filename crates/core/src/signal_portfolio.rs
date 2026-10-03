//! Deterministic portfolio construction from authenticated measured signals.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::ArtifactReference;
use crate::e0_deterministic_sensor::{E0DiagnosticSignal, E0DiagnosticWindow};
use crate::e0_query_lab::VerifiedE0QueryResult;

const QUALIFICATION_POLICY_ID: &str = "positive_observed_cases";
const QUALIFICATION_POLICY_VERSION: u16 = 1;

/// Why a configured signal could not be measured. These states carry no count
/// fields, so callers cannot mistake unavailable evidence for a measured zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalUnavailableReason {
    SourceFieldUnavailable,
    EvidenceIncomplete,
    MeasurementFailed,
    NotConfigured,
}

/// A non-measured metric outcome scoped by an authenticated signal from the
/// same run. It deliberately accepts no numerator, denominator, or synthetic
/// evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SignalUnavailable {
    metric_id: String,
    policy_id: String,
    policy_version: u16,
    reason: SignalUnavailableReason,
    scope: SignalEvidenceScope,
}

impl SignalUnavailable {
    pub fn new(
        metric_id: impl Into<String>,
        policy_id: impl Into<String>,
        policy_version: u16,
        reason: SignalUnavailableReason,
        scope_witness: &VerifiedE0QueryResult,
    ) -> Result<Self, SignalPortfolioError> {
        let metric_id = metric_id.into();
        let policy_id = policy_id.into();
        if !valid_identifier(&metric_id) || !valid_identifier(&policy_id) || policy_version == 0 {
            return Err(SignalPortfolioError::InvalidUnavailableSignal);
        }
        let scope = SignalEvidenceScope::from_verified_query(scope_witness)?;
        Ok(Self {
            metric_id,
            policy_id,
            policy_version,
            reason,
            scope,
        })
    }

    #[must_use]
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }

    #[must_use]
    pub fn policy_id(&self) -> &str {
        &self.policy_id
    }

    #[must_use]
    pub fn policy_version(&self) -> u16 {
        self.policy_version
    }

    #[must_use]
    pub fn reason(&self) -> SignalUnavailableReason {
        self.reason
    }
}

/// Inputs are either opaque sensor output or explicit non-measurement states.
/// Raw rows, caller-provided counts, and deserialized signal claims are not accepted.
#[derive(Clone, Debug)]
pub enum SignalOutcome<'a> {
    Measured(&'a E0DiagnosticSignal),
    Unsupported(SignalUnavailable),
    Unknown(SignalUnavailable),
}

/// A signal with positive known support, suitable for independent hypothesis
/// investigation. This is descriptive eligibility, not a business opportunity
/// conclusion, causal claim, or authorization to execute a change.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SignalCandidate {
    candidate_id: String,
    metric_id: String,
    metric_policy_id: String,
    metric_policy_version: u16,
    metric_spec_commitment: String,
    qualification_policy_id: String,
    qualification_policy_version: u16,
    signal_digest: String,
    numerator: u64,
    denominator: u64,
    missing: u64,
    coverage_basis_points: u16,
    window: E0DiagnosticWindow,
    cutoff_unix_seconds: u64,
    tenant_id: String,
    run_id: String,
    grant_id: String,
    authority_ref: String,
    source_snapshot_ref: ArtifactReference,
    source_snapshot_binding: String,
    availability_profile_digest: String,
    table: String,
    source_contract_digest: String,
    source_digest: String,
    transform_digest: String,
    replay_projection_digest: String,
    source_evidence_digest: String,
    query_receipt_digests: Vec<String>,
}

impl SignalCandidate {
    #[must_use]
    pub fn candidate_id(&self) -> &str {
        &self.candidate_id
    }

    #[must_use]
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }

    #[must_use]
    pub fn metric_policy_id(&self) -> &str {
        &self.metric_policy_id
    }

    #[must_use]
    pub fn policy_id(&self) -> &str {
        &self.metric_policy_id
    }

    #[must_use]
    pub fn policy_version(&self) -> u16 {
        self.metric_policy_version
    }

    #[must_use]
    pub fn metric_spec_commitment(&self) -> &str {
        &self.metric_spec_commitment
    }

    #[must_use]
    pub fn qualification_policy_id(&self) -> &str {
        &self.qualification_policy_id
    }

    #[must_use]
    pub fn qualification_policy_version(&self) -> u16 {
        self.qualification_policy_version
    }

    #[must_use]
    pub fn signal_digest(&self) -> &str {
        &self.signal_digest
    }

    #[must_use]
    pub fn numerator(&self) -> u64 {
        self.numerator
    }

    #[must_use]
    pub fn denominator(&self) -> u64 {
        self.denominator
    }

    #[must_use]
    pub fn missing(&self) -> u64 {
        self.missing
    }

    #[must_use]
    pub fn coverage_basis_points(&self) -> u16 {
        self.coverage_basis_points
    }

    #[must_use]
    pub fn window(&self) -> E0DiagnosticWindow {
        self.window
    }

    #[must_use]
    pub fn cutoff_unix_seconds(&self) -> u64 {
        self.cutoff_unix_seconds
    }

    #[must_use]
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    #[must_use]
    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }

    #[must_use]
    pub fn authority_ref(&self) -> &str {
        &self.authority_ref
    }

    #[must_use]
    pub fn source_snapshot_ref(&self) -> &ArtifactReference {
        &self.source_snapshot_ref
    }

    #[must_use]
    pub fn source_snapshot_binding(&self) -> &str {
        &self.source_snapshot_binding
    }

    #[must_use]
    pub fn availability_profile_digest(&self) -> &str {
        &self.availability_profile_digest
    }

    #[must_use]
    pub fn table(&self) -> &str {
        &self.table
    }

    #[must_use]
    pub fn source_contract_digest(&self) -> &str {
        &self.source_contract_digest
    }

    #[must_use]
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    #[must_use]
    pub fn transform_digest(&self) -> &str {
        &self.transform_digest
    }

    #[must_use]
    pub fn replay_projection_digest(&self) -> &str {
        &self.replay_projection_digest
    }

    #[must_use]
    pub fn source_evidence_digest(&self) -> &str {
        &self.source_evidence_digest
    }

    #[must_use]
    pub fn query_receipt_digests(&self) -> &[String] {
        &self.query_receipt_digests
    }
}

/// A deterministic finding for a measured metric that did not qualify.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SignalNotQualified {
    metric_id: String,
    signal_digest: String,
    numerator: u64,
    denominator: u64,
    missing: u64,
    reason: NotQualifiedReason,
}

impl SignalNotQualified {
    #[must_use]
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }

    #[must_use]
    pub fn signal_digest(&self) -> &str {
        &self.signal_digest
    }

    #[must_use]
    pub fn numerator(&self) -> u64 {
        self.numerator
    }

    #[must_use]
    pub fn denominator(&self) -> u64 {
        self.denominator
    }

    #[must_use]
    pub fn missing(&self) -> u64 {
        self.missing
    }

    #[must_use]
    pub fn reason(&self) -> NotQualifiedReason {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotQualifiedReason {
    NoKnownDenominator,
    NoPositiveKnownObservations,
}

/// Every input is retained as one of these outcomes, including unsupported
/// and unknown states; those states never acquire fabricated count fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum SignalObservation {
    Candidate {
        candidate_id: String,
        metric_id: String,
        signal_digest: String,
    },
    NotQualified(SignalNotQualified),
    Unsupported(SignalUnavailable),
    Unknown(SignalUnavailable),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioStatus {
    CandidatesReady,
    NoQualifyingSignals,
    InsufficientEvidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoOpportunityReason {
    NoSignalsProvided,
    NoQualifyingSignals,
    InsufficientEvidence,
}

/// Immutable, deterministically ordered signal candidates and full dispositions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SignalPortfolio {
    status: PortfolioStatus,
    no_opportunity_reason: Option<NoOpportunityReason>,
    candidates: Vec<SignalCandidate>,
    observations: Vec<SignalObservation>,
}

impl SignalPortfolio {
    pub fn build(outcomes: &[SignalOutcome<'_>]) -> Result<Self, SignalPortfolioError> {
        let mut candidates = Vec::new();
        let mut observations = Vec::with_capacity(outcomes.len());
        let mut digests = BTreeSet::new();
        let mut scope: Option<SignalEvidenceScope> = None;

        for outcome in outcomes {
            match outcome {
                SignalOutcome::Measured(signal) => {
                    if !signal.has_valid_digest() {
                        return Err(SignalPortfolioError::InvalidSignalDigest);
                    }
                    let binding = signal.scout_binding();
                    validate_binding(&binding)?;
                    let current_scope = SignalEvidenceScope::from_binding(&binding);
                    validate_scope(&mut scope, &current_scope)?;
                    if !digests.insert(binding.signal_digest.clone()) {
                        return Err(SignalPortfolioError::DuplicateSignal);
                    }
                    if binding.denominator == 0 || binding.numerator == 0 {
                        let reason = if binding.denominator == 0 {
                            NotQualifiedReason::NoKnownDenominator
                        } else {
                            NotQualifiedReason::NoPositiveKnownObservations
                        };
                        observations.push(SignalObservation::NotQualified(SignalNotQualified {
                            metric_id: binding.metric_id,
                            signal_digest: binding.signal_digest,
                            numerator: binding.numerator,
                            denominator: binding.denominator,
                            missing: binding.missing,
                            reason,
                        }));
                        continue;
                    }

                    let candidate = SignalCandidate::from_binding(binding);
                    observations.push(SignalObservation::Candidate {
                        candidate_id: candidate.candidate_id.clone(),
                        metric_id: candidate.metric_id.clone(),
                        signal_digest: candidate.signal_digest.clone(),
                    });
                    candidates.push(candidate);
                }
                SignalOutcome::Unsupported(unavailable) => {
                    validate_scope(&mut scope, &unavailable.scope)?;
                    observations.push(SignalObservation::Unsupported(unavailable.clone()));
                }
                SignalOutcome::Unknown(unavailable) => {
                    validate_scope(&mut scope, &unavailable.scope)?;
                    observations.push(SignalObservation::Unknown(unavailable.clone()));
                }
            }
        }

        candidates.sort_by(|left, right| {
            (
                left.metric_id.as_str(),
                left.window.start_unix_seconds(),
                left.window.end_unix_seconds(),
                left.signal_digest.as_str(),
            )
                .cmp(&(
                    right.metric_id.as_str(),
                    right.window.start_unix_seconds(),
                    right.window.end_unix_seconds(),
                    right.signal_digest.as_str(),
                ))
        });
        observations
            .sort_by(|left, right| observation_sort_key(left).cmp(&observation_sort_key(right)));

        let has_measured_outcome = outcomes
            .iter()
            .any(|outcome| matches!(outcome, SignalOutcome::Measured(_)));
        let has_unavailable_outcome = outcomes.iter().any(|outcome| {
            matches!(
                outcome,
                SignalOutcome::Unsupported(_) | SignalOutcome::Unknown(_)
            )
        });
        let has_uncovered_measured_outcome = observations.iter().any(|observation| {
            matches!(
                observation,
                SignalObservation::NotQualified(signal)
                    if signal.reason == NotQualifiedReason::NoKnownDenominator
            )
        });
        let status = if !candidates.is_empty() {
            PortfolioStatus::CandidatesReady
        } else if outcomes.is_empty()
            || !has_measured_outcome
            || has_unavailable_outcome
            || has_uncovered_measured_outcome
        {
            PortfolioStatus::InsufficientEvidence
        } else {
            PortfolioStatus::NoQualifyingSignals
        };
        let no_opportunity_reason = if candidates.is_empty() {
            Some(if outcomes.is_empty() {
                NoOpportunityReason::NoSignalsProvided
            } else if !has_measured_outcome
                || has_unavailable_outcome
                || has_uncovered_measured_outcome
            {
                NoOpportunityReason::InsufficientEvidence
            } else {
                NoOpportunityReason::NoQualifyingSignals
            })
        } else {
            None
        };
        Ok(Self {
            status,
            no_opportunity_reason,
            candidates,
            observations,
        })
    }

    #[must_use]
    pub fn status(&self) -> PortfolioStatus {
        self.status
    }

    #[must_use]
    pub fn no_opportunity_reason(&self) -> Option<NoOpportunityReason> {
        self.no_opportunity_reason
    }

    #[must_use]
    pub fn candidates(&self) -> &[SignalCandidate] {
        &self.candidates
    }

    #[must_use]
    pub fn observations(&self) -> &[SignalObservation] {
        &self.observations
    }
}

impl SignalCandidate {
    fn from_binding(binding: crate::e0_deterministic_sensor::E0ScoutSignalBinding) -> Self {
        let candidate_id = candidate_id(&binding.signal_digest);
        Self {
            candidate_id,
            metric_id: binding.metric_id,
            metric_policy_id: binding.metric_policy_id,
            metric_policy_version: binding.metric_policy_version,
            metric_spec_commitment: binding.metric_spec_commitment,
            qualification_policy_id: QUALIFICATION_POLICY_ID.to_owned(),
            qualification_policy_version: QUALIFICATION_POLICY_VERSION,
            signal_digest: binding.signal_digest,
            numerator: binding.numerator,
            denominator: binding.denominator,
            missing: binding.missing,
            coverage_basis_points: binding.coverage_basis_points,
            window: binding.window,
            cutoff_unix_seconds: binding.cutoff_unix_seconds,
            tenant_id: binding.tenant_id,
            run_id: binding.run_id,
            grant_id: binding.grant_id,
            authority_ref: binding.authority_ref,
            source_snapshot_ref: binding.source_snapshot_ref,
            source_snapshot_binding: binding.source_snapshot_binding,
            availability_profile_digest: binding.availability_profile_digest,
            table: binding.table,
            source_contract_digest: binding.source_contract_digest,
            source_digest: binding.source_digest,
            transform_digest: binding.transform_digest,
            replay_projection_digest: binding.replay_projection_digest,
            source_evidence_digest: binding.source_evidence_digest,
            query_receipt_digests: binding.query_receipt_digests,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignalPortfolioError {
    InvalidUnavailableSignal,
    InvalidSignalDigest,
    InvalidSignalProvenance,
    MixedEvidenceScope,
    DuplicateSignal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SignalEvidenceScope {
    tenant_id: String,
    run_id: String,
    grant_id: String,
    authority_ref: String,
    source_snapshot_ref: ArtifactReference,
    source_snapshot_binding: String,
    cutoff_unix_seconds: u64,
}

fn validate_scope(
    expected: &mut Option<SignalEvidenceScope>,
    current: &SignalEvidenceScope,
) -> Result<(), SignalPortfolioError> {
    if let Some(expected) = expected {
        if expected != current {
            return Err(SignalPortfolioError::MixedEvidenceScope);
        }
    } else {
        *expected = Some(current.clone());
    }
    Ok(())
}

impl SignalEvidenceScope {
    fn from_binding(binding: &crate::e0_deterministic_sensor::E0ScoutSignalBinding) -> Self {
        Self {
            tenant_id: binding.tenant_id.clone(),
            run_id: binding.run_id.clone(),
            grant_id: binding.grant_id.clone(),
            authority_ref: binding.authority_ref.clone(),
            source_snapshot_ref: binding.source_snapshot_ref.clone(),
            source_snapshot_binding: binding.source_snapshot_binding.clone(),
            cutoff_unix_seconds: binding.cutoff_unix_seconds,
        }
    }

    fn from_verified_query(
        verified_query: &VerifiedE0QueryResult,
    ) -> Result<Self, SignalPortfolioError> {
        let receipt = verified_query.result().receipt();
        let commitments = verified_query.commitments();
        if !receipt.has_valid_digest()
            || receipt.tenant_id.is_empty()
            || receipt.run_id.is_empty()
            || receipt.grant_id.is_empty()
            || receipt.authority_ref.is_empty()
            || receipt.source_snapshot_ref.tenant_id != receipt.tenant_id
            || receipt.source_snapshot_ref.id.is_empty()
            || receipt.source_snapshot_ref.revision == 0
            || !is_digest(&receipt.source_snapshot_ref.digest)
            || receipt.cutoff_unix_seconds != commitments.cutoff_unix_seconds()
            || !is_digest(commitments.source_snapshot_binding())
        {
            return Err(SignalPortfolioError::InvalidSignalProvenance);
        }
        Ok(Self {
            tenant_id: receipt.tenant_id.clone(),
            run_id: receipt.run_id.clone(),
            grant_id: receipt.grant_id.clone(),
            authority_ref: receipt.authority_ref.clone(),
            source_snapshot_ref: receipt.source_snapshot_ref.clone(),
            source_snapshot_binding: commitments.source_snapshot_binding().to_owned(),
            cutoff_unix_seconds: receipt.cutoff_unix_seconds,
        })
    }
}

fn validate_binding(
    binding: &crate::e0_deterministic_sensor::E0ScoutSignalBinding,
) -> Result<(), SignalPortfolioError> {
    if !valid_identifier(&binding.metric_id)
        || !valid_identifier(&binding.metric_policy_id)
        || binding.metric_policy_version == 0
        || binding.metric_spec_commitment.is_empty()
        || binding.tenant_id.is_empty()
        || binding.run_id.is_empty()
        || binding.grant_id.is_empty()
        || binding.authority_ref.is_empty()
        || binding.source_snapshot_ref.tenant_id != binding.tenant_id
        || !is_digest(&binding.signal_digest)
        || !is_digest(&binding.metric_spec_commitment)
        || !is_digest(&binding.source_snapshot_binding)
        || !is_digest(&binding.availability_profile_digest)
        || !is_digest(&binding.source_contract_digest)
        || !is_digest(&binding.source_digest)
        || !is_digest(&binding.transform_digest)
        || !is_digest(&binding.replay_projection_digest)
        || !is_digest(&binding.source_evidence_digest)
        || binding.query_receipt_digests.is_empty()
        || binding
            .query_receipt_digests
            .iter()
            .any(|digest| !is_digest(digest))
        || binding.denominator == 0 && binding.numerator != 0
        || binding.numerator > binding.denominator
        || binding.coverage_basis_points > 10_000
        || binding.window.end_unix_seconds() > binding.cutoff_unix_seconds
    {
        return Err(SignalPortfolioError::InvalidSignalProvenance);
    }
    Ok(())
}

fn observation_sort_key(observation: &SignalObservation) -> (&str, u8, &str, u16, u8) {
    match observation {
        SignalObservation::Candidate {
            metric_id,
            signal_digest,
            ..
        } => (metric_id, 0, signal_digest, 0, 0),
        SignalObservation::NotQualified(signal) => {
            (&signal.metric_id, 1, &signal.signal_digest, 0, 0)
        }
        SignalObservation::Unsupported(signal) => (
            &signal.metric_id,
            2,
            &signal.policy_id,
            signal.policy_version,
            unavailable_reason_sort_key(signal.reason),
        ),
        SignalObservation::Unknown(signal) => (
            &signal.metric_id,
            3,
            &signal.policy_id,
            signal.policy_version,
            unavailable_reason_sort_key(signal.reason),
        ),
    }
}

fn unavailable_reason_sort_key(reason: SignalUnavailableReason) -> u8 {
    match reason {
        SignalUnavailableReason::SourceFieldUnavailable => 0,
        SignalUnavailableReason::EvidenceIncomplete => 1,
        SignalUnavailableReason::MeasurementFailed => 2,
        SignalUnavailableReason::NotConfigured => 3,
    }
}

fn candidate_id(signal_digest: &str) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("pulso-signal-candidate-v1:{signal_digest}").as_bytes())
    )
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use crate::e0_deterministic_sensor::{
        DiagnosticMetricPolicy, DiagnosticMetricSpec, E0DiagnosticSensor, E0DiagnosticWindow,
    };
    use crate::e0_query_lab::E0QueryLab;
    use crate::enriched_history::{
        AvailabilityClockMode, AvailabilityProfile, EnrichedHistoryAdapter,
        EnrichedHistoryManifest, PackageFile, ProvenanceDigests, ReplayRowAvailability, TableInput,
        replay_projection_digest,
    };
    use crate::local_lab::{
        InMemoryLabGrantAuthority, InMemoryLabSourceAuthority, LabAccess, LabDataClassification,
        LabGrant, LabQuery, LabSource, LabSourceApprovalPort, LabSourceManifest, LabTable,
        LocalInvestigationLab,
    };
    use crate::source_validation::{SourceSnapshot, resolve_source_snapshot_artifact};
    use crate::{ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository};

    fn digest(seed: char) -> String {
        format!("sha256:{}", seed.to_string().repeat(64))
    }

    fn row(value: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
            ("technical_error".to_owned(), value.to_owned()),
        ])
    }

    fn authenticated_signal(
        run_id: &str,
        seed: char,
        values: &[&str],
        window_start: u64,
    ) -> super::super::e0_deterministic_sensor::E0DiagnosticSignal {
        authenticated_signal_for_tenant(run_id, seed, values, window_start, "tenant_a")
    }

    fn authenticated_signal_for_tenant(
        run_id: &str,
        seed: char,
        values: &[&str],
        window_start: u64,
        tenant_id: &str,
    ) -> super::super::e0_deterministic_sensor::E0DiagnosticSignal {
        let evidence = authenticated_query_evidence_for_tenant(run_id, seed, values, tenant_id);
        E0DiagnosticSensor::measure(
            &DiagnosticMetricSpec::from_policy(DiagnosticMetricPolicy::TechnicalErrorRateV1),
            E0DiagnosticWindow::new(window_start, 100).unwrap(),
            std::slice::from_ref(&evidence),
        )
        .unwrap()
    }

    fn authenticated_query_scope(
        run_id: &str,
        seed: char,
        values: &[&str],
        tenant_id: &str,
    ) -> crate::e0_query_lab::VerifiedE0QueryResult {
        authenticated_query_evidence_for_tenant(run_id, seed, values, tenant_id)
    }

    fn authenticated_query_evidence_for_tenant(
        run_id: &str,
        seed: char,
        values: &[&str],
        tenant_id: &str,
    ) -> crate::e0_query_lab::VerifiedE0QueryResult {
        let raw_snapshot = format!(
            r#"{{"world_ref":"world_a","sources":[{{"row_count":{},"header_digest":"{}","table":"contacts","source_contract_ref":{{"version":"v1","digest":"{}","id":"contacts"}},"uri":"file://contacts.csv","file_digest":"{}"}}],"tenant_id":"{}","observed_cutoff":"1970-01-01T00:01:40Z","contract_version":{{"minor":0,"major":1}},"source_namespace":"platform_history"}}"#,
            values.len(),
            digest('b'),
            digest('c'),
            digest(seed),
            tenant_id,
        );
        let snapshot = SourceSnapshot::from_json(&raw_snapshot).unwrap();
        let rows = values
            .iter()
            .map(|value| json!({"event_time":"1970-01-01T00:01:40Z","technical_error":value}))
            .collect::<Vec<_>>();
        let availability = values
            .iter()
            .map(|_| {
                ReplayRowAvailability::new(BTreeMap::from([
                    ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                    (
                        "technical_error".to_owned(),
                        "1970-01-01T00:01:40Z".to_owned(),
                    ),
                ]))
            })
            .collect::<Vec<_>>();
        let manifest = EnrichedHistoryManifest::new_replay(
            "platform_history",
            "world_a",
            "1970-01-01T00:01:40Z",
            AvailabilityProfile::new(
                "e0_replay",
                1,
                AvailabilityClockMode::replay_at_event_time("e0_zero_lag"),
                tenant_id,
                snapshot.binding_digest(),
            ),
            vec![
                PackageFile::new(
                    "contacts",
                    ProvenanceDigests::new(digest(seed), digest('b'), digest('c'), digest('d')),
                    "1970-01-01T00:01:40Z",
                )
                .with_field_availability(BTreeMap::from([
                    ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                    (
                        "technical_error".to_owned(),
                        "1970-01-01T00:01:40Z".to_owned(),
                    ),
                ]))
                .with_replay_projection_digest(replay_projection_digest(&rows, &availability))
                .with_source_file_seal(snapshot.source_file_seal("contacts").unwrap()),
            ],
        );
        let adapter = EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot).unwrap();
        let replay = adapter.verified_replay_availability(&snapshot).unwrap();
        let projection = adapter
            .verified_e0_query_projection(
                &snapshot,
                &replay,
                "contacts",
                TableInput::new(
                    ProvenanceDigests::new(digest(seed), digest('b'), digest('c'), digest('d')),
                    rows,
                )
                .with_replay_row_availability(availability),
            )
            .unwrap();

        let mut repository = InMemoryArtifactRepository::default();
        let snapshot_ref = repository
            .append(
                None,
                ArtifactDraft::new(
                    tenant_id,
                    format!("018f50a1-7f00-7000-8000-00000000000{seed}"),
                    1,
                    ArtifactKind::SourceSnapshot,
                    json!({"source_snapshot_json":raw_snapshot}),
                    None,
                ),
            )
            .unwrap()
            .reference();
        let source_binding =
            resolve_source_snapshot_artifact(&mut repository, &snapshot_ref).unwrap();
        let source = LabSource::new(
            LabSourceManifest {
                tenant_id: tenant_id.to_owned(),
                snapshot_ref: snapshot_ref.clone(),
                source_contract_digest: digest('b'),
                source_digest: digest(seed),
                transform_digest: digest('c'),
                cutoff_unix_seconds: 100,
                classification: LabDataClassification::Treated,
                safe_for_discovery: true,
            },
            vec![LabTable::new(
                "contacts",
                vec!["event_time", "technical_error"],
                values.iter().map(|value| row(value)).collect(),
            )],
        )
        .unwrap();
        let approved = InMemoryLabSourceAuthority.approve(source).unwrap();
        let approved = projection
            .bind_approved_lab_source(approved, source_binding)
            .unwrap();
        let access = LabAccess::new(
            run_id,
            tenant_id,
            "investigation",
            "grant_e0",
            "authority_e0",
            snapshot_ref,
            1_000,
        );
        let mut grants = InMemoryLabGrantAuthority::default();
        grants.issue(LabGrant::from_access(&access));
        let mut lab = LocalInvestigationLab::new(grants);
        let session = lab.open(access.clone(), approved, 100).unwrap();
        let result = lab
            .query(
                session.session_id(),
                &access,
                LabQuery::select("contacts", vec!["event_time", "technical_error"], None),
                100,
            )
            .unwrap();
        E0QueryLab::admit(
            &projection,
            lab.governed_e0_candidate(session.session_id(), &access, &result.receipt().digest, 100)
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn portfolio_retains_every_qualifying_signal_with_stable_order_and_provenance() {
        let later_window = authenticated_signal("run_a", 'a', &["true", "false"], 100);
        let earlier_window = authenticated_signal("run_a", 'a', &["true", "false"], 99);
        let portfolio = super::SignalPortfolio::build(&[
            super::SignalOutcome::Measured(&later_window),
            super::SignalOutcome::Measured(&earlier_window),
        ])
        .unwrap();

        assert_eq!(portfolio.status(), super::PortfolioStatus::CandidatesReady);
        assert_eq!(portfolio.candidates().len(), 2);
        assert_eq!(portfolio.candidates()[0].window().start_unix_seconds(), 99);
        assert_eq!(portfolio.candidates()[1].window().start_unix_seconds(), 100);
        for candidate in portfolio.candidates() {
            assert_eq!(candidate.metric_id(), "e0_technical_error_rate");
            assert_eq!(candidate.policy_id(), "e0_diagnostic_allowlist");
            assert_eq!(candidate.policy_version(), 1);
            assert!(candidate.metric_spec_commitment().starts_with("sha256:"));
            assert!(candidate.signal_digest().starts_with("sha256:"));
            assert_eq!(candidate.source_snapshot_ref().tenant_id, "tenant_a");
            assert!(candidate.source_snapshot_binding().starts_with("sha256:"));
            assert_eq!(candidate.cutoff_unix_seconds(), 100);
            assert!(!candidate.query_receipt_digests().is_empty());
            assert_eq!(candidate.numerator(), 1);
            assert_eq!(candidate.denominator(), 2);
        }

        let reversed = super::SignalPortfolio::build(&[
            super::SignalOutcome::Measured(&earlier_window),
            super::SignalOutcome::Measured(&later_window),
        ])
        .unwrap();
        assert_eq!(portfolio, reversed);
    }

    #[test]
    fn unsupported_and_unknown_measurements_remain_explicit_not_zero_rates() {
        let witness = authenticated_query_scope("run_a", 'a', &["true", "false"], "tenant_a");
        let unsupported = super::SignalUnavailable::new(
            "e0_rejected_transaction_rate",
            "transaction_status_v1",
            1,
            super::SignalUnavailableReason::SourceFieldUnavailable,
            &witness,
        )
        .unwrap();
        let unknown = super::SignalUnavailable::new(
            "e0_contact_recurrence_rate",
            "contact_recurrence_v1",
            2,
            super::SignalUnavailableReason::EvidenceIncomplete,
            &witness,
        )
        .unwrap();
        let portfolio = super::SignalPortfolio::build(&[
            super::SignalOutcome::Unsupported(unsupported),
            super::SignalOutcome::Unknown(unknown),
        ])
        .unwrap();

        assert!(portfolio.candidates().is_empty());
        assert_eq!(portfolio.observations().len(), 2);
        assert!(matches!(
            portfolio.observations()[0],
            super::SignalObservation::Unknown(_)
        ));
        assert!(matches!(
            portfolio.observations()[1],
            super::SignalObservation::Unsupported(_)
        ));
        let serialized = serde_json::to_value(&portfolio).unwrap();
        assert_eq!(serialized["status"], "insufficient_evidence");
        assert_eq!(serialized["no_opportunity_reason"], "insufficient_evidence");
        assert!(serialized["observations"][0].get("numerator").is_none());
        assert!(serialized["observations"][0].get("denominator").is_none());
        assert!(serialized["observations"][1].get("numerator").is_none());
        assert!(serialized["observations"][1].get("denominator").is_none());
    }

    #[test]
    fn unavailable_observations_have_stable_order_when_reasons_are_permuted() {
        let witness = authenticated_query_scope("run_a", 'a', &["true", "false"], "tenant_a");
        let incomplete = super::SignalUnavailable::new(
            "e0_rejected_transaction_rate",
            "transaction_status_v1",
            1,
            super::SignalUnavailableReason::EvidenceIncomplete,
            &witness,
        )
        .unwrap();
        let missing_source = super::SignalUnavailable::new(
            "e0_rejected_transaction_rate",
            "transaction_status_v1",
            1,
            super::SignalUnavailableReason::SourceFieldUnavailable,
            &witness,
        )
        .unwrap();

        let first = super::SignalPortfolio::build(&[
            super::SignalOutcome::Unsupported(incomplete.clone()),
            super::SignalOutcome::Unsupported(missing_source.clone()),
        ])
        .unwrap();
        let reversed = super::SignalPortfolio::build(&[
            super::SignalOutcome::Unsupported(missing_source),
            super::SignalOutcome::Unsupported(incomplete),
        ])
        .unwrap();

        assert_eq!(first, reversed);
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&reversed).unwrap()
        );
    }

    #[test]
    fn unavailable_scope_must_match_measured_tenant_run_and_snapshot() {
        let measured = authenticated_signal("run_a", 'a', &["true", "false"], 100);
        let wrong_scopes = [
            authenticated_query_scope("run_a", 'a', &["true", "false"], "tenant_b"),
            authenticated_query_scope("run_b", 'a', &["true", "false"], "tenant_a"),
            authenticated_query_scope("run_a", 'b', &["true", "false"], "tenant_a"),
        ];

        for witness in &wrong_scopes {
            let unavailable = super::SignalUnavailable::new(
                "e0_contact_recurrence_rate",
                "contact_recurrence_v1",
                1,
                super::SignalUnavailableReason::EvidenceIncomplete,
                witness,
            )
            .unwrap();
            assert_eq!(
                super::SignalPortfolio::build(&[
                    super::SignalOutcome::Measured(&measured),
                    super::SignalOutcome::Unknown(unavailable),
                ])
                .unwrap_err(),
                super::SignalPortfolioError::MixedEvidenceScope
            );
        }
    }

    #[test]
    fn missing_metric_coverage_keeps_measured_zero_from_becoming_no_qualifying() {
        let measured_zero = authenticated_signal("run_a", 'a', &["false", "false"], 100);
        let scope = authenticated_query_scope("run_a", 'a', &["false", "false"], "tenant_a");
        let unsupported = super::SignalUnavailable::new(
            "e0_rejected_transaction_rate",
            "transaction_status_v1",
            1,
            super::SignalUnavailableReason::SourceFieldUnavailable,
            &scope,
        )
        .unwrap();
        let portfolio = super::SignalPortfolio::build(&[
            super::SignalOutcome::Measured(&measured_zero),
            super::SignalOutcome::Unsupported(unsupported),
        ])
        .unwrap();

        let serialized = serde_json::to_value(&portfolio).unwrap();
        assert_eq!(serialized["status"], "insufficient_evidence");
        assert_eq!(serialized["no_opportunity_reason"], "insufficient_evidence");
    }

    #[test]
    fn uncovered_measured_population_is_insufficient_not_a_no_signal_finding() {
        let uncovered_population = authenticated_signal("run_a", 'a', &["unknown"], 100);
        assert_eq!(uncovered_population.denominator(), 0);
        assert_eq!(uncovered_population.missing(), 1);

        let portfolio =
            super::SignalPortfolio::build(&[super::SignalOutcome::Measured(&uncovered_population)])
                .unwrap();
        let serialized = serde_json::to_value(&portfolio).unwrap();
        assert_eq!(serialized["status"], "insufficient_evidence");
        assert_eq!(serialized["no_opportunity_reason"], "insufficient_evidence");
    }

    #[test]
    fn digest_validation_requires_canonical_lowercase_sha256_hex() {
        assert!(super::is_digest(&format!("sha256:{}", "a".repeat(64))));
        assert!(!super::is_digest(&format!("sha256:{}", "A".repeat(64))));
    }

    #[test]
    fn portfolio_rejects_evidence_from_another_run() {
        let first = authenticated_signal("run_a", 'a', &["true", "false"], 100);
        let second = authenticated_signal("run_b", 'a', &["true", "false"], 99);

        assert!(
            super::SignalPortfolio::build(&[
                super::SignalOutcome::Measured(&first),
                super::SignalOutcome::Measured(&second),
            ])
            .is_err()
        );
    }

    #[test]
    fn no_positive_known_evidence_and_empty_input_are_explicit_no_ops() {
        let signal = authenticated_signal("run_a", 'a', &["false", ""], 100);
        let portfolio =
            super::SignalPortfolio::build(&[super::SignalOutcome::Measured(&signal)]).unwrap();
        assert_eq!(
            portfolio.status(),
            super::PortfolioStatus::NoQualifyingSignals
        );
        assert_eq!(
            portfolio.no_opportunity_reason(),
            Some(super::NoOpportunityReason::NoQualifyingSignals)
        );
        assert!(portfolio.candidates().is_empty());
        assert!(matches!(
            portfolio.observations()[0],
            super::SignalObservation::NotQualified(_)
        ));

        let empty = super::SignalPortfolio::build(&[]).unwrap();
        assert_eq!(empty.status(), super::PortfolioStatus::InsufficientEvidence);
        assert_eq!(
            empty.no_opportunity_reason(),
            Some(super::NoOpportunityReason::NoSignalsProvided)
        );
        assert!(empty.candidates().is_empty());
        assert!(empty.observations().is_empty());
    }
}
