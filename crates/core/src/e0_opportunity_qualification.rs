//! U14-EQ: pure Frozen E0 opportunity qualification.
//!
//! This is deliberately not a proposal, a value model, a causal conclusion or
//! a U16 mechanism bridge. It says only that an exact U13-A candidate and its
//! exact U14-E Frozen-consistency report may be carried forward for *future*
//! mechanism and evaluation work.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ArtifactReference;
use crate::autonomous_scout::VerifiedScoutCandidate;
use crate::core_task::CoreTaskScope;
use crate::e0_frozen_verifier::{FrozenE0VerificationError, FrozenE0VerificationReport};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrozenE0ProvenanceQualification {
    FrozenProvenanceConsistent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentStatus {
    NotAssessed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationNextStep {
    RequiresMechanismAndEvaluation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationRoute {
    None,
}

/// Opaque, immutable, nonpersistent qualification. Its constants intentionally
/// forbid callers from reading it as commercial value, feasibility, causality
/// or a route selection.
pub struct FrozenE0OpportunityQualification {
    scope: CoreTaskScope,
    candidate_digest: String,
    candidate_provenance_commitment: String,
    e0_provenance_commitment: String,
    verifier_policy_commitment: String,
    verification_input_commitment: String,
    verification_evidence_commitment: String,
    verification_report_commitment: String,
    source_snapshot_ref: ArtifactReference,
    provenance: FrozenE0ProvenanceQualification,
    commercial_impact: AssessmentStatus,
    operational_effort: AssessmentStatus,
    next_step: QualificationNextStep,
    route: QualificationRoute,
    commitment: String,
}

impl FrozenE0OpportunityQualification {
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
    pub fn verifier_policy_commitment(&self) -> &str {
        &self.verifier_policy_commitment
    }
    #[must_use]
    pub fn verification_input_commitment(&self) -> &str {
        &self.verification_input_commitment
    }
    #[must_use]
    pub fn verification_evidence_commitment(&self) -> &str {
        &self.verification_evidence_commitment
    }
    #[must_use]
    pub fn verification_report_commitment(&self) -> &str {
        &self.verification_report_commitment
    }
    #[must_use]
    pub fn source_snapshot_ref(&self) -> &ArtifactReference {
        &self.source_snapshot_ref
    }
    #[must_use]
    pub fn provenance(&self) -> FrozenE0ProvenanceQualification {
        self.provenance
    }
    #[must_use]
    pub fn commercial_impact(&self) -> AssessmentStatus {
        self.commercial_impact
    }
    #[must_use]
    pub fn operational_effort(&self) -> AssessmentStatus {
        self.operational_effort
    }
    #[must_use]
    pub fn next_step(&self) -> QualificationNextStep {
        self.next_step
    }
    #[must_use]
    pub fn route(&self) -> QualificationRoute {
        self.route
    }
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrozenE0QualificationError {
    Verification(FrozenE0VerificationError),
    CandidateReportMismatch,
}

impl From<FrozenE0VerificationError> for FrozenE0QualificationError {
    fn from(value: FrozenE0VerificationError) -> Self {
        Self::Verification(value)
    }
}

/// Public API has only opaque U13-A/U14-E inputs. The actual composition is
/// crate-private, preventing construction from a report-like JSON/receipt.
pub struct FrozenE0OpportunityQualifier;

impl FrozenE0OpportunityQualifier {
    pub fn qualify(
        candidate: &VerifiedScoutCandidate,
        report: &FrozenE0VerificationReport,
    ) -> Result<FrozenE0OpportunityQualification, FrozenE0QualificationError> {
        FrozenE0QualificationComposer::compose(candidate, report)
    }
}

pub(crate) struct FrozenE0QualificationComposer;

impl FrozenE0QualificationComposer {
    fn compose(
        candidate: &VerifiedScoutCandidate,
        report: &FrozenE0VerificationReport,
    ) -> Result<FrozenE0OpportunityQualification, FrozenE0QualificationError> {
        // Rehydrate the exact U13-A canonical E0 record and independently
        // recompute the U14-E result before any qualification is emitted.
        candidate
            .rehydrate_frozen_e0()
            .map_err(|error| match error {
                crate::autonomous_scout::FrozenE0ScoutCandidateError::NotE0Candidate => {
                    FrozenE0QualificationError::CandidateReportMismatch
                }
                other => FrozenE0QualificationError::Verification(other.into()),
            })?;
        report.revalidate_for_candidate(candidate)?;
        let provenance = FrozenE0ProvenanceQualification::FrozenProvenanceConsistent;
        let commercial_impact = AssessmentStatus::NotAssessed;
        let operational_effort = AssessmentStatus::NotAssessed;
        let next_step = QualificationNextStep::RequiresMechanismAndEvaluation;
        let route = QualificationRoute::None;
        let commitment = digest(&QualificationCommitment {
            tenant_id: candidate.scope().tenant_id(),
            job_id: candidate.scope().job_id(),
            grant_id: candidate.scope().grant_id(),
            authority_ref: candidate.scope().authority_ref(),
            candidate_digest: candidate.candidate_digest(),
            candidate_provenance_commitment: candidate.provenance_commitment(),
            e0_provenance_commitment: report.e0_provenance_commitment(),
            verifier_policy_commitment: report.policy_commitment(),
            verification_input_commitment: report.input_commitment(),
            verification_evidence_commitment: report.evidence_commitment(),
            verification_report_commitment: report.report_commitment(),
            source_snapshot_ref: report.source_snapshot_ref(),
            provenance,
            commercial_impact,
            operational_effort,
            next_step,
            route,
        });
        Ok(FrozenE0OpportunityQualification {
            scope: candidate.scope().clone(),
            candidate_digest: candidate.candidate_digest().to_owned(),
            candidate_provenance_commitment: candidate.provenance_commitment().to_owned(),
            e0_provenance_commitment: report.e0_provenance_commitment().to_owned(),
            verifier_policy_commitment: report.policy_commitment().to_owned(),
            verification_input_commitment: report.input_commitment().to_owned(),
            verification_evidence_commitment: report.evidence_commitment().to_owned(),
            verification_report_commitment: report.report_commitment().to_owned(),
            source_snapshot_ref: report.source_snapshot_ref().clone(),
            provenance,
            commercial_impact,
            operational_effort,
            next_step,
            route,
            commitment,
        })
    }
}

#[derive(Serialize)]
struct QualificationCommitment<'a> {
    tenant_id: &'a str,
    job_id: &'a str,
    grant_id: &'a str,
    authority_ref: &'a str,
    candidate_digest: &'a str,
    candidate_provenance_commitment: &'a str,
    e0_provenance_commitment: &'a str,
    verifier_policy_commitment: &'a str,
    verification_input_commitment: &'a str,
    verification_evidence_commitment: &'a str,
    verification_report_commitment: &'a str,
    source_snapshot_ref: &'a ArtifactReference,
    provenance: FrozenE0ProvenanceQualification,
    commercial_impact: AssessmentStatus,
    operational_effort: AssessmentStatus,
    next_step: QualificationNextStep,
    route: QualificationRoute,
}

fn digest<T: Serialize>(value: &T) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("qualification commits serialize"))
    )
}

/// ```compile_fail
/// use improvement_engine_core::e0_opportunity_qualification::{FrozenE0OpportunityQualification, FrozenE0OpportunityQualifier};
/// let _ = FrozenE0OpportunityQualification {};
/// let _ = FrozenE0OpportunityQualifier::qualify("draft", "receipt");
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::e0_opportunity_qualification::FrozenE0OpportunityQualification;
/// use improvement_engine_core::workflow_bridge::{WorkflowBridge, WorkflowBridgeInput};
/// # let qualification: FrozenE0OpportunityQualification = todo!();
/// # let input: WorkflowBridgeInput = todo!();
/// let _ = WorkflowBridge::assess_verified(&qualification, input);
/// ```
const _QUALIFICATION_CANNOT_BE_FORGED_OR_CONVERTED_TO_U16: () = ();

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "test-support")]
    use crate::autonomous_scout::verified_real_e0_candidate_for_frozen_verifier_test;
    #[cfg(feature = "test-support")]
    use crate::e0_frozen_verifier::{
        FrozenE0IndependentVerifier, FrozenE0ReportBindingField,
        corrupt_frozen_e0_report_for_qualification_test,
    };

    #[cfg(feature = "test-support")]
    #[test]
    fn real_e0_chain_qualifies_only_as_unassessed_future_work() {
        let candidate = verified_real_e0_candidate_for_frozen_verifier_test();
        let report = FrozenE0IndependentVerifier::verify(&candidate).unwrap();
        let first = FrozenE0OpportunityQualifier::qualify(&candidate, &report).unwrap();
        let second = FrozenE0OpportunityQualifier::qualify(&candidate, &report).unwrap();

        assert_eq!(
            first.provenance(),
            FrozenE0ProvenanceQualification::FrozenProvenanceConsistent
        );
        assert_eq!(first.commercial_impact(), AssessmentStatus::NotAssessed);
        assert_eq!(first.operational_effort(), AssessmentStatus::NotAssessed);
        assert_eq!(
            first.next_step(),
            QualificationNextStep::RequiresMechanismAndEvaluation
        );
        assert_eq!(first.route(), QualificationRoute::None);
        assert_eq!(first.commitment(), second.commitment());
        assert_eq!(first.candidate_digest(), candidate.candidate_digest());
        assert_eq!(
            first.verification_report_commitment(),
            report.report_commitment()
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn every_report_binding_drift_is_rejected_without_a_qualification() {
        let candidate = verified_real_e0_candidate_for_frozen_verifier_test();
        for field in [
            FrozenE0ReportBindingField::CandidateDigest,
            FrozenE0ReportBindingField::E0Provenance,
            FrozenE0ReportBindingField::Policy,
            FrozenE0ReportBindingField::Input,
            FrozenE0ReportBindingField::Evidence,
            FrozenE0ReportBindingField::Snapshot,
            FrozenE0ReportBindingField::Report,
        ] {
            let mut report = FrozenE0IndependentVerifier::verify(&candidate).unwrap();
            corrupt_frozen_e0_report_for_qualification_test(&mut report, field);
            assert!(matches!(
                FrozenE0OpportunityQualifier::qualify(&candidate, &report),
                Err(FrozenE0QualificationError::Verification(
                    FrozenE0VerificationError::ProvenanceMismatch
                ))
            ));
        }
    }
}
