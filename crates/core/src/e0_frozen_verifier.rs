//! U14-E: deterministic consistency verification for an admitted Frozen E0
//! candidate. It is deliberately narrower than generic U14: it reports only
//! whether the complete sealed provenance remains internally consistent; it
//! never corroborates a cause, predicts an outcome, or authorizes a change.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ArtifactReference;
use crate::autonomous_scout::{
    FrozenE0ScoutCandidateError, VerifiedFrozenE0ScoutCandidate, VerifiedScoutCandidate,
};
use crate::core_task::CoreTaskScope;
use crate::e0_deterministic_sensor::{DiagnosticMetricPolicy, DiagnosticMetricSpec};

/// Versioned, sealed verifier policy. There is intentionally no dynamic
/// policy input or port: accepting one would allow a caller to self-attest a
/// candidate as verified.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) enum FrozenE0VerifierPolicy {
    ProvenanceConsistencyV1,
}

impl FrozenE0VerifierPolicy {
    fn id(self) -> &'static str {
        "frozen_e0_provenance_consistency"
    }
    fn version(self) -> u16 {
        1
    }
    fn commitment(self) -> String {
        digest(&FrozenPolicyCommitment {
            id: self.id(),
            version: self.version(),
        })
    }
}

#[derive(Serialize)]
struct FrozenPolicyCommitment<'a> {
    id: &'a str,
    version: u16,
}

/// A precise, non-causal result. `Consistent` means only the immutable E0
/// provenance passed the frozen structural checks; it is not `Supported` and
/// must never be interpreted as causal corroboration or release eligibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrozenE0ConsistencyStatus {
    Consistent,
}

/// Opaque U14-E report. It publishes commitments only; raw rows, query
/// receipts, model output and internal E0 provenance are never exposed.
pub struct FrozenE0VerificationReport {
    scope: CoreTaskScope,
    candidate_digest: String,
    candidate_provenance_commitment: String,
    e0_provenance_commitment: String,
    policy_commitment: String,
    input_commitment: String,
    evidence_commitment: String,
    source_snapshot_ref: ArtifactReference,
    status: FrozenE0ConsistencyStatus,
    report_commitment: String,
}

impl FrozenE0VerificationReport {
    #[must_use]
    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }
    #[must_use]
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }
    #[must_use]
    pub fn candidate_provenance_commitment(&self) -> &str {
        &self.candidate_provenance_commitment
    }
    #[must_use]
    pub fn e0_provenance_commitment(&self) -> &str {
        &self.e0_provenance_commitment
    }
    #[must_use]
    pub fn policy_commitment(&self) -> &str {
        &self.policy_commitment
    }
    #[must_use]
    pub fn input_commitment(&self) -> &str {
        &self.input_commitment
    }
    #[must_use]
    pub fn evidence_commitment(&self) -> &str {
        &self.evidence_commitment
    }
    #[must_use]
    pub fn source_snapshot_ref(&self) -> &ArtifactReference {
        &self.source_snapshot_ref
    }
    #[must_use]
    pub fn status(&self) -> FrozenE0ConsistencyStatus {
        self.status
    }
    #[must_use]
    pub fn report_commitment(&self) -> &str {
        &self.report_commitment
    }
    #[must_use]
    pub fn is_causal_corroboration(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrozenE0VerificationError {
    NotE0Candidate,
    InvalidCanonicalRecord,
    ProvenanceMismatch,
    FrozenConsistencyFailed,
}

impl From<FrozenE0ScoutCandidateError> for FrozenE0VerificationError {
    fn from(value: FrozenE0ScoutCandidateError) -> Self {
        match value {
            FrozenE0ScoutCandidateError::NotE0Candidate => Self::NotE0Candidate,
            FrozenE0ScoutCandidateError::InvalidCanonicalRecord => Self::InvalidCanonicalRecord,
            FrozenE0ScoutCandidateError::ProvenanceMismatch => Self::ProvenanceMismatch,
        }
    }
}

/// U14-E has no pluggable verifier port and no caller receipt. Its only input
/// is an opaque U13-A capability which is rehydrated again at this boundary.
pub struct FrozenE0IndependentVerifier;

impl FrozenE0IndependentVerifier {
    pub fn verify(
        candidate: &VerifiedScoutCandidate,
    ) -> Result<FrozenE0VerificationReport, FrozenE0VerificationError> {
        let frozen = candidate.rehydrate_frozen_e0()?;
        Self::verify_rehydrated(&frozen)
    }

    pub(crate) fn verify_rehydrated(
        frozen: &VerifiedFrozenE0ScoutCandidate,
    ) -> Result<FrozenE0VerificationReport, FrozenE0VerificationError> {
        let policy = FrozenE0VerifierPolicy::ProvenanceConsistencyV1;
        let (policy_id, policy_version, semantics) = frozen.metric_policy();
        let expected_metric_spec =
            DiagnosticMetricSpec::from_policy(DiagnosticMetricPolicy::TechnicalErrorRateV1);
        if policy_id != "e0_diagnostic_allowlist"
            || policy_version != 1
            || semantics != "observed_technical_error_flag"
            || frozen.metric_spec_commitment() != expected_metric_spec.commitment()
            || !frozen.frozen_bounds_are_consistent()
        {
            return Err(FrozenE0VerificationError::FrozenConsistencyFailed);
        }
        let policy_commitment = policy.commitment();
        let input_commitment = digest(&FrozenInputCommitment {
            tenant_id: frozen.scope().tenant_id(),
            job_id: frozen.scope().job_id(),
            grant_id: frozen.scope().grant_id(),
            authority_ref: frozen.scope().authority_ref(),
            candidate_digest: frozen.candidate_digest(),
            candidate_provenance_commitment: frozen.provenance_commitment(),
            e0_provenance_commitment: frozen.e0_commitment(),
            policy_commitment: &policy_commitment,
        });
        let evidence_commitment = digest(&FrozenEvidenceCommitment {
            input_commitment: &input_commitment,
            e0_provenance_commitment: frozen.e0_commitment(),
            policy_commitment: &policy_commitment,
        });
        let source_snapshot_ref = frozen.source_snapshot_ref();
        let status = FrozenE0ConsistencyStatus::Consistent;
        let report_commitment = digest(&FrozenReportCommitment {
            input_commitment: &input_commitment,
            evidence_commitment: &evidence_commitment,
            source_snapshot_ref: &source_snapshot_ref,
            status,
        });
        Ok(FrozenE0VerificationReport {
            scope: frozen.scope().clone(),
            candidate_digest: frozen.candidate_digest().to_owned(),
            candidate_provenance_commitment: frozen.provenance_commitment().to_owned(),
            e0_provenance_commitment: frozen.e0_commitment().to_owned(),
            policy_commitment,
            input_commitment,
            evidence_commitment,
            source_snapshot_ref,
            status,
            report_commitment,
        })
    }
}

#[derive(Serialize)]
struct FrozenInputCommitment<'a> {
    tenant_id: &'a str,
    job_id: &'a str,
    grant_id: &'a str,
    authority_ref: &'a str,
    candidate_digest: &'a str,
    candidate_provenance_commitment: &'a str,
    e0_provenance_commitment: &'a str,
    policy_commitment: &'a str,
}
#[derive(Serialize)]
struct FrozenEvidenceCommitment<'a> {
    input_commitment: &'a str,
    e0_provenance_commitment: &'a str,
    policy_commitment: &'a str,
}
#[derive(Serialize)]
struct FrozenReportCommitment<'a> {
    input_commitment: &'a str,
    evidence_commitment: &'a str,
    source_snapshot_ref: &'a ArtifactReference,
    status: FrozenE0ConsistencyStatus,
}
fn digest<T: Serialize>(value: &T) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("frozen verifier inputs serialize"))
    )
}

/// ```compile_fail
/// use improvement_engine_core::e0_frozen_verifier::{FrozenE0IndependentVerifier, FrozenE0VerificationReport};
/// let _ = FrozenE0VerificationReport {};
/// let _ = FrozenE0IndependentVerifier::verify("caller receipt");
/// ```
const _FROZEN_E0_REPORT_AND_PORT_ARE_NOT_CALLER_CONSTRUCTIBLE: () = ();

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "test-support")]
    use crate::autonomous_scout::corrupt_e0_metric_spec_for_frozen_verifier_test;
    use crate::autonomous_scout::verified_candidate_for_independent_verifier_test;
    #[cfg(feature = "test-support")]
    use crate::autonomous_scout::verified_real_e0_candidate_for_frozen_verifier_test;

    #[cfg(feature = "test-support")]
    #[test]
    fn real_frozen_e0_chain_emits_only_a_non_causal_consistency_report() {
        let candidate = verified_real_e0_candidate_for_frozen_verifier_test();
        let first = FrozenE0IndependentVerifier::verify(&candidate).unwrap();
        let second = FrozenE0IndependentVerifier::verify(&candidate).unwrap();

        assert_eq!(first.status(), FrozenE0ConsistencyStatus::Consistent);
        assert!(!first.is_causal_corroboration());
        assert_eq!(first.candidate_digest(), candidate.candidate_digest());
        assert_eq!(
            first.candidate_provenance_commitment(),
            candidate.provenance_commitment()
        );
        assert_eq!(first.scope(), candidate.scope());
        assert_eq!(first.report_commitment(), second.report_commitment());
        assert_eq!(first.evidence_commitment(), second.evidence_commitment());
    }

    #[test]
    fn generic_u13_candidate_is_rejected_without_a_report_or_port_fallback() {
        let candidate = verified_candidate_for_independent_verifier_test();
        assert!(matches!(
            FrozenE0IndependentVerifier::verify(&candidate),
            Err(FrozenE0VerificationError::NotE0Candidate)
        ));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn all_missing_e0_observation_is_consistent_when_coverage_is_zero() {
        let candidate = verified_real_e0_candidate_for_frozen_verifier_test();
        let frozen = candidate
            .rehydrate_frozen_e0()
            .unwrap()
            .all_missing_for_frozen_verifier_test();
        assert_eq!(
            FrozenE0IndependentVerifier::verify_rehydrated(&frozen)
                .unwrap()
                .status(),
            FrozenE0ConsistencyStatus::Consistent
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn inconsistent_coverage_is_rejected_without_a_report() {
        let candidate = verified_real_e0_candidate_for_frozen_verifier_test();
        let frozen = candidate
            .rehydrate_frozen_e0()
            .unwrap()
            .inconsistent_coverage_for_frozen_verifier_test();
        assert!(matches!(
            FrozenE0IndependentVerifier::verify_rehydrated(&frozen),
            Err(FrozenE0VerificationError::FrozenConsistencyFailed)
        ));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn invalid_policy_semantics_or_bounds_are_rejected_without_a_report() {
        let candidate = verified_real_e0_candidate_for_frozen_verifier_test();
        for frozen in [
            candidate
                .rehydrate_frozen_e0()
                .unwrap()
                .invalid_policy_for_frozen_verifier_test(),
            candidate
                .rehydrate_frozen_e0()
                .unwrap()
                .invalid_semantics_for_frozen_verifier_test(),
            candidate
                .rehydrate_frozen_e0()
                .unwrap()
                .invalid_bounds_for_frozen_verifier_test(),
        ] {
            assert!(matches!(
                FrozenE0IndependentVerifier::verify_rehydrated(&frozen),
                Err(FrozenE0VerificationError::FrozenConsistencyFailed)
            ));
        }
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn altered_but_rehashed_e0_canonical_record_is_rejected_during_rehydration() {
        let mut candidate = verified_real_e0_candidate_for_frozen_verifier_test();
        corrupt_e0_metric_spec_for_frozen_verifier_test(&mut candidate);
        assert!(matches!(
            FrozenE0IndependentVerifier::verify(&candidate),
            Err(FrozenE0VerificationError::ProvenanceMismatch)
        ));
    }
}
