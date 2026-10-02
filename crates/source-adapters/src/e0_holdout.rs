//! Post-discovery recurrence evaluation on the E0 Reproduccion partition.
//!
//! This API is intentionally downstream of candidate selection: a caller must
//! attest the selected core pattern against Arranque before a holdout result
//! can be created. Only already-opaque query signatures and case ordinals are
//! consumed. The persisted result contains aggregate counts and commitments,
//! never signatures, rows, labels, outcomes, or customer identifiers.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{CasePhase, E0Fact, PreparedSource, SourceKind};

const HOLDOUT_POLICY_ID: &str = "e0_recurrence_holdout";
const HOLDOUT_POLICY_VERSION: u16 = 2;
const MIN_POLICY_SUPPORT: u64 = 5;
const MAX_POLICY_SUPPORT: u64 = 5_000;

/// Versioned, deterministic threshold for interpreting a holdout recurrence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct E0HoldoutPolicy {
    minimum_distinct_case_support: u64,
}

impl E0HoldoutPolicy {
    pub fn new(minimum_distinct_case_support: u64) -> Result<Self, E0HoldoutError> {
        if !(MIN_POLICY_SUPPORT..=MAX_POLICY_SUPPORT).contains(&minimum_distinct_case_support) {
            return Err(E0HoldoutError::InvalidPolicy);
        }
        Ok(Self {
            minimum_distinct_case_support,
        })
    }

    #[must_use]
    pub fn minimum_distinct_case_support(&self) -> u64 {
        self.minimum_distinct_case_support
    }

    #[must_use]
    pub fn policy_id(&self) -> &'static str {
        HOLDOUT_POLICY_ID
    }

    #[must_use]
    pub fn policy_version(&self) -> u16 {
        HOLDOUT_POLICY_VERSION
    }
}

/// A selected recurrence candidate sealed against the discovery-only portion
/// of an E0 source. It intentionally cannot be serialized or debug-printed:
/// its opaque query-signature commitment is retained only in memory for the
/// post-selection comparison.
#[derive(Clone, Eq, PartialEq)]
pub struct SelectedE0RecurrenceCandidate {
    candidate_ref: String,
    discovery_source_commitment: String,
    tenant_id: String,
    opaque_query_signature: String,
    arranque_support_cases: u64,
}

impl SelectedE0RecurrenceCandidate {
    #[must_use]
    pub fn candidate_ref(&self) -> &str {
        &self.candidate_ref
    }

    #[must_use]
    pub fn discovery_source_commitment(&self) -> &str {
        &self.discovery_source_commitment
    }

    #[must_use]
    pub fn arranque_support_cases(&self) -> u64 {
        self.arranque_support_cases
    }
}

/// Only the post-discovery validator should receive this token. It validates
/// the core-selected pattern against Arranque evidence and does not inspect
/// Reproduccion facts while doing so.
pub fn attest_selected_e0_recurrence_candidate(
    discovery_source: &PreparedSource,
    selected_candidate_ref: &str,
    minimum_arranque_support: u64,
) -> Result<SelectedE0RecurrenceCandidate, E0HoldoutError> {
    if discovery_source.source_kind() != SourceKind::E0
        || !is_full_digest(selected_candidate_ref)
        || !(1..=MAX_POLICY_SUPPORT).contains(&minimum_arranque_support)
    {
        return Err(E0HoldoutError::InvalidCandidate);
    }
    let arranque_cases = discovery_source
        .agent_inputs()
        .cases()
        .iter()
        .filter(|case| case.phase() == CasePhase::Arranque)
        .map(|case| case.ordinal())
        .collect::<BTreeSet<_>>();
    if arranque_cases.is_empty() || !discovery_source.has_copilot_query_table() {
        return Err(E0HoldoutError::NoDiscoveryEvidence);
    }

    let mut support = BTreeMap::<String, BTreeSet<u32>>::new();
    for fact in discovery_source.agent_inputs().facts() {
        if let E0Fact::CopilotQuery {
            case_ordinal,
            query_signature,
            ..
        } = fact
        {
            if arranque_cases.contains(case_ordinal) {
                if !is_opaque_query_signature(query_signature) {
                    return Err(E0HoldoutError::InvalidSourceProjection);
                }
                support
                    .entry(query_signature.clone())
                    .or_default()
                    .insert(*case_ordinal);
            }
        }
    }
    let Some((signature, cases)) = support.iter().max_by(|left, right| {
        left.1
            .len()
            .cmp(&right.1.len())
            .then_with(|| right.0.cmp(left.0))
    }) else {
        return Err(E0HoldoutError::NoDiscoveryEvidence);
    };
    let support_cases = cases.len() as u64;
    if support_cases < minimum_arranque_support {
        return Err(E0HoldoutError::InsufficientDiscoverySupport);
    }
    let expected_candidate_ref = derive_candidate_ref(discovery_source, signature);
    if expected_candidate_ref != selected_candidate_ref {
        return Err(E0HoldoutError::CandidateDoesNotMatchArranque);
    }
    Ok(SelectedE0RecurrenceCandidate {
        candidate_ref: selected_candidate_ref.to_owned(),
        discovery_source_commitment: discovery_source.manifest_digest().to_owned(),
        tenant_id: discovery_source.snapshot_ref().tenant_id.clone(),
        opaque_query_signature: signature.clone(),
        arranque_support_cases: support_cases,
    })
}

/// Safe status for a descriptive recurrence check; no causal or outcome claim
/// is represented by any of these states.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum E0HoldoutStatus {
    Replicated,
    NotObserved,
    InsufficientSupport,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0HoldoutEvaluation {
    policy_id: String,
    policy_version: u16,
    minimum_distinct_case_support: u64,
    candidate_ref: String,
    discovery_source_commitment: String,
    holdout_source_commitment: String,
    status: E0HoldoutStatus,
    reproduction_case_count: Option<u64>,
    queried_case_count: Option<u64>,
    matching_case_count: Option<u64>,
    recurrence_rate_basis_points: Option<u16>,
    interpretation: &'static str,
}

impl E0HoldoutEvaluation {
    #[must_use]
    pub fn policy_version(&self) -> u16 {
        self.policy_version
    }
    #[must_use]
    pub fn status(&self) -> E0HoldoutStatus {
        self.status
    }
    #[must_use]
    pub fn reproduction_case_count(&self) -> Option<u64> {
        self.reproduction_case_count
    }
    #[must_use]
    pub fn queried_case_count(&self) -> Option<u64> {
        self.queried_case_count
    }
    #[must_use]
    pub fn matching_case_count(&self) -> Option<u64> {
        self.matching_case_count
    }
    #[must_use]
    pub fn recurrence_rate_basis_points(&self) -> Option<u16> {
        self.recurrence_rate_basis_points
    }
    #[must_use]
    pub fn discovery_source_commitment(&self) -> &str {
        &self.discovery_source_commitment
    }
    #[must_use]
    pub fn holdout_source_commitment(&self) -> &str {
        &self.holdout_source_commitment
    }
}

/// Evaluate a candidate only against Reproduccion queries from a committed E0
/// holdout snapshot. Arranque query facts are never used in this function.
pub fn evaluate_e0_recurrence_holdout(
    candidate: &SelectedE0RecurrenceCandidate,
    holdout_source: &PreparedSource,
    policy: &E0HoldoutPolicy,
) -> Result<E0HoldoutEvaluation, E0HoldoutError> {
    if holdout_source.source_kind() != SourceKind::E0 {
        return Err(E0HoldoutError::InvalidSource);
    }
    if holdout_source.snapshot_ref().tenant_id != candidate.tenant_id {
        return Err(E0HoldoutError::SourceScopeMismatch);
    }
    let mut evaluation = E0HoldoutEvaluation {
        policy_id: HOLDOUT_POLICY_ID.to_owned(),
        policy_version: HOLDOUT_POLICY_VERSION,
        minimum_distinct_case_support: policy.minimum_distinct_case_support,
        candidate_ref: candidate.candidate_ref.clone(),
        discovery_source_commitment: candidate.discovery_source_commitment.clone(),
        holdout_source_commitment: holdout_source.manifest_digest().to_owned(),
        status: E0HoldoutStatus::Unavailable,
        reproduction_case_count: None,
        queried_case_count: None,
        matching_case_count: None,
        recurrence_rate_basis_points: None,
        interpretation: "descriptive_recurrence_only_no_causal_or_outcome_claim",
    };
    if !holdout_source.has_copilot_query_table() {
        return Ok(evaluation);
    }

    let reproduction_cases = holdout_source
        .agent_inputs()
        .cases()
        .iter()
        .filter(|case| case.phase() == CasePhase::Reproduccion)
        .map(|case| case.ordinal())
        .collect::<BTreeSet<_>>();
    let mut queried_cases = BTreeSet::new();
    let mut matching_cases = BTreeSet::new();
    for fact in holdout_source.agent_inputs().facts() {
        if let E0Fact::CopilotQuery {
            case_ordinal,
            query_signature,
            ..
        } = fact
        {
            if reproduction_cases.contains(case_ordinal) {
                if !is_opaque_query_signature(query_signature) {
                    evaluation.status = E0HoldoutStatus::Unavailable;
                    return Ok(evaluation);
                }
                queried_cases.insert(*case_ordinal);
                if query_signature == &candidate.opaque_query_signature {
                    matching_cases.insert(*case_ordinal);
                }
            }
        }
    }
    let denominator = queried_cases.len() as u64;
    let numerator = matching_cases.len() as u64;
    if denominator < policy.minimum_distinct_case_support
        || (numerator > 0 && numerator < policy.minimum_distinct_case_support)
    {
        evaluation.status = E0HoldoutStatus::InsufficientSupport;
    } else {
        evaluation.reproduction_case_count = Some(reproduction_cases.len() as u64);
        evaluation.queried_case_count = Some(denominator);
        evaluation.matching_case_count = Some(numerator);
        evaluation.recurrence_rate_basis_points = (denominator > 0)
            .then(|| ((u128::from(numerator) * 10_000) / u128::from(denominator)) as u16);
        evaluation.status = if numerator == 0 {
            E0HoldoutStatus::NotObserved
        } else {
            E0HoldoutStatus::Replicated
        };
    }
    Ok(evaluation)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E0HoldoutError {
    InvalidPolicy,
    InvalidCandidate,
    NoDiscoveryEvidence,
    InsufficientDiscoverySupport,
    CandidateDoesNotMatchArranque,
    InvalidSourceProjection,
    InvalidSource,
    SourceScopeMismatch,
}

impl std::fmt::Display for E0HoldoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPolicy => "holdout policy is outside its safe aggregate range",
            Self::InvalidCandidate => "selected recurrence candidate is invalid",
            Self::NoDiscoveryEvidence => "Arranque contains no eligible recurrence evidence",
            Self::InsufficientDiscoverySupport => "Arranque recurrence support is below policy",
            Self::CandidateDoesNotMatchArranque => {
                "selected candidate does not match Arranque evidence"
            }
            Self::InvalidSourceProjection => "source adapter emitted an invalid opaque signature",
            Self::InvalidSource => "holdout source is not an E0 package",
            Self::SourceScopeMismatch => {
                "discovery and holdout sources have different tenant scopes"
            }
        })
    }
}

impl std::error::Error for E0HoldoutError {}

fn derive_candidate_ref(source: &PreparedSource, signature: &str) -> String {
    let value = format!(
        "pattern-v1:{}:{}:{}:{}",
        source.snapshot_ref().tenant_id,
        source.manifest_digest(),
        source.snapshot_ref().digest,
        signature
    );
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn is_opaque_query_signature(value: &str) -> bool {
    value.len() == 63
        && value.starts_with("sha256_")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_full_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
