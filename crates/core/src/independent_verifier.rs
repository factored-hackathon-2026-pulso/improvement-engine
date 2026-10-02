//! U14 independent evidence verification over an opaque U13-A capability.
//!
//! This module deliberately does not inspect a Scout draft, author a proposal,
//! select an Agent Core artifact, or infer causality. It binds an independent
//! check to the admitted candidate's immutable scope and commitments, then
//! publishes only a typed `Supported`, `Refuted`, or `Uncertain` outcome.
//! A future adapter may combine deterministic checks with U11's pinned Jev
//! decision port; that adapter must still return the exact input commitment
//! defined here.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ArtifactReference;
use crate::autonomous_scout::VerifiedScoutCandidate;
use crate::core_task::CoreTaskScope;

/// A non-causal result of independent evidence checking.
///
/// `Supported` means the configured verifier found the committed evidence
/// consistent with the candidate. It never means that the candidate proved a
/// causal relationship. `Uncertain` is an outcome, not a default promotion.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Supported,
    Refuted,
    Uncertain,
}

/// Read-only request assembled exclusively from an admitted U13-A candidate.
/// It intentionally carries commitments and a source reference, never a raw
/// hypothesis draft or model output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndependentVerificationInput {
    scope: CoreTaskScope,
    candidate_digest: String,
    provenance_commitment: String,
    source_snapshot_ref: ArtifactReference,
    commitment: String,
}

impl IndependentVerificationInput {
    #[must_use]
    pub fn from_admitted(candidate: &VerifiedScoutCandidate) -> Self {
        let scope = candidate.scope().clone();
        let candidate_digest = candidate.candidate_digest().to_owned();
        let provenance_commitment = candidate.provenance_commitment().to_owned();
        let source_snapshot_ref = candidate.source_snapshot_ref().clone();
        let commitment = input_commitment(
            &scope,
            &candidate_digest,
            &provenance_commitment,
            &source_snapshot_ref,
        );
        Self {
            scope,
            candidate_digest,
            provenance_commitment,
            source_snapshot_ref,
            commitment,
        }
    }

    #[must_use]
    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }

    #[must_use]
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }

    #[must_use]
    pub fn provenance_commitment(&self) -> &str {
        &self.provenance_commitment
    }

    #[must_use]
    pub fn source_snapshot_ref(&self) -> &ArtifactReference {
        &self.source_snapshot_ref
    }

    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }
}

/// Immutable result returned by a checker that is independent from U13.
/// The result must echo the exact request commitment, which prevents a receipt
/// for one candidate or tenant from being published for another.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndependentVerificationReceipt {
    input_commitment: String,
    verifier_id: String,
    verifier_version: String,
    evidence_commitment: String,
    status: VerificationStatus,
    digest: String,
}

impl IndependentVerificationReceipt {
    pub fn new(
        input_commitment: impl Into<String>,
        verifier_id: impl Into<String>,
        verifier_version: impl Into<String>,
        evidence_commitment: impl Into<String>,
        status: VerificationStatus,
    ) -> Result<Self, IndependentVerificationError> {
        let input_commitment = input_commitment.into();
        let verifier_id = verifier_id.into();
        let verifier_version = verifier_version.into();
        let evidence_commitment = evidence_commitment.into();
        if !is_sha256_digest(&input_commitment) || !is_sha256_digest(&evidence_commitment) {
            return Err(IndependentVerificationError::InvalidReceipt);
        }
        if !is_identifier(&verifier_id) || !is_identifier(&verifier_version) {
            return Err(IndependentVerificationError::InvalidReceipt);
        }
        let digest = receipt_digest(
            &input_commitment,
            &verifier_id,
            &verifier_version,
            &evidence_commitment,
            status,
        );
        Ok(Self {
            input_commitment,
            verifier_id,
            verifier_version,
            evidence_commitment,
            status,
            digest,
        })
    }

    #[must_use]
    pub fn input_commitment(&self) -> &str {
        &self.input_commitment
    }

    #[must_use]
    pub fn verifier_id(&self) -> &str {
        &self.verifier_id
    }

    #[must_use]
    pub fn verifier_version(&self) -> &str {
        &self.verifier_version
    }

    #[must_use]
    pub fn evidence_commitment(&self) -> &str {
        &self.evidence_commitment
    }

    #[must_use]
    pub fn status(&self) -> VerificationStatus {
        self.status
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Narrow port owned by the independent-verification adapter. Its input lacks
/// the raw Scout draft, so it cannot quietly become a proposal author.
pub trait IndependentEvidenceVerifierPort {
    fn verify(
        &mut self,
        input: IndependentVerificationInput,
    ) -> Result<IndependentVerificationReceipt, IndependentVerificationPortError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndependentVerificationPortError {
    DependencyUnavailable,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndependentVerificationError {
    InvalidReceipt,
    ReceiptInputMismatch,
    Port(IndependentVerificationPortError),
}

/// Published U14 outcome. This is not a proposal and has no mutable effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationReport {
    scope: CoreTaskScope,
    candidate_digest: String,
    provenance_commitment: String,
    input_commitment: String,
    receipt: IndependentVerificationReceipt,
}

impl VerificationReport {
    #[must_use]
    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }

    #[must_use]
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }

    #[must_use]
    pub fn provenance_commitment(&self) -> &str {
        &self.provenance_commitment
    }

    #[must_use]
    pub fn input_commitment(&self) -> &str {
        &self.input_commitment
    }

    #[must_use]
    pub fn status(&self) -> VerificationStatus {
        self.receipt.status()
    }

    #[must_use]
    pub fn receipt(&self) -> &IndependentVerificationReceipt {
        &self.receipt
    }
}

/// Stateless composition boundary for U14.
///
/// ```compile_fail
/// use improvement_engine_core::autonomous_scout::ScoutCandidateDraft;
/// use improvement_engine_core::independent_verifier::{
///     IndependentEvidenceVerifierPort, IndependentVerifier,
/// };
///
/// fn cannot_verify_a_raw_scout_draft(
///     draft: &ScoutCandidateDraft,
///     port: &mut impl IndependentEvidenceVerifierPort,
/// ) {
///     let _ = IndependentVerifier::verify(draft, port);
/// }
/// ```
pub struct IndependentVerifier;

impl IndependentVerifier {
    pub fn verify(
        candidate: &VerifiedScoutCandidate,
        port: &mut impl IndependentEvidenceVerifierPort,
    ) -> Result<VerificationReport, IndependentVerificationError> {
        let input = IndependentVerificationInput::from_admitted(candidate);
        let receipt = port
            .verify(input.clone())
            .map_err(IndependentVerificationError::Port)?;
        if receipt.input_commitment() != input.commitment() {
            return Err(IndependentVerificationError::ReceiptInputMismatch);
        }
        Ok(VerificationReport {
            scope: input.scope,
            candidate_digest: input.candidate_digest,
            provenance_commitment: input.provenance_commitment,
            input_commitment: input.commitment,
            receipt,
        })
    }
}

#[derive(Serialize)]
struct InputCommitment<'a> {
    tenant_id: &'a str,
    job_id: &'a str,
    grant_id: &'a str,
    authority_ref: &'a str,
    candidate_digest: &'a str,
    provenance_commitment: &'a str,
    source_snapshot_ref: &'a ArtifactReference,
}

fn input_commitment(
    scope: &CoreTaskScope,
    candidate_digest: &str,
    provenance_commitment: &str,
    source_snapshot_ref: &ArtifactReference,
) -> String {
    digest(&InputCommitment {
        tenant_id: scope.tenant_id(),
        job_id: scope.job_id(),
        grant_id: scope.grant_id(),
        authority_ref: scope.authority_ref(),
        candidate_digest,
        provenance_commitment,
        source_snapshot_ref,
    })
}

#[derive(Serialize)]
struct ReceiptDigest<'a> {
    input_commitment: &'a str,
    verifier_id: &'a str,
    verifier_version: &'a str,
    evidence_commitment: &'a str,
    status: VerificationStatus,
}

fn receipt_digest(
    input_commitment: &str,
    verifier_id: &str,
    verifier_version: &str,
    evidence_commitment: &str,
    status: VerificationStatus,
) -> String {
    digest(&ReceiptDigest {
        input_commitment,
        verifier_id,
        verifier_version,
        evidence_commitment,
        status,
    })
}

fn digest<T: Serialize>(value: &T) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("verification commitments serialize"))
    )
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(
            |byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autonomous_scout::verified_candidate_for_independent_verifier_test;

    fn d(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    struct ScriptedVerifier {
        status: VerificationStatus,
        wrong_input: bool,
    }

    impl IndependentEvidenceVerifierPort for ScriptedVerifier {
        fn verify(
            &mut self,
            input: IndependentVerificationInput,
        ) -> Result<IndependentVerificationReceipt, IndependentVerificationPortError> {
            IndependentVerificationReceipt::new(
                if self.wrong_input {
                    d('f')
                } else {
                    input.commitment().to_owned()
                },
                "independent_evidence",
                "v1",
                d('e'),
                self.status,
            )
            .map_err(|_| IndependentVerificationPortError::Unknown)
        }
    }

    #[test]
    fn publishes_supported_report_for_admitted_candidate_only() {
        let candidate = verified_candidate_for_independent_verifier_test();
        let mut port = ScriptedVerifier {
            status: VerificationStatus::Supported,
            wrong_input: false,
        };

        let report = IndependentVerifier::verify(&candidate, &mut port).unwrap();

        assert_eq!(report.status(), VerificationStatus::Supported);
        assert_eq!(report.candidate_digest(), candidate.candidate_digest());
        assert_eq!(
            report.provenance_commitment(),
            candidate.provenance_commitment()
        );
        assert_eq!(report.scope(), candidate.scope());
        assert_eq!(report.receipt().verifier_id(), "independent_evidence");
    }

    #[test]
    fn rejects_a_receipt_bound_to_another_candidate_or_scope() {
        let candidate = verified_candidate_for_independent_verifier_test();
        let mut port = ScriptedVerifier {
            status: VerificationStatus::Refuted,
            wrong_input: true,
        };

        assert_eq!(
            IndependentVerifier::verify(&candidate, &mut port),
            Err(IndependentVerificationError::ReceiptInputMismatch)
        );
    }

    #[test]
    fn uncertain_is_explicit_and_does_not_become_supported() {
        let candidate = verified_candidate_for_independent_verifier_test();
        let mut port = ScriptedVerifier {
            status: VerificationStatus::Uncertain,
            wrong_input: false,
        };

        assert_eq!(
            IndependentVerifier::verify(&candidate, &mut port)
                .unwrap()
                .status(),
            VerificationStatus::Uncertain
        );
    }

    #[test]
    fn refuted_is_an_explicit_non_causal_outcome() {
        let candidate = verified_candidate_for_independent_verifier_test();
        let mut port = ScriptedVerifier {
            status: VerificationStatus::Refuted,
            wrong_input: false,
        };

        assert_eq!(
            IndependentVerifier::verify(&candidate, &mut port)
                .unwrap()
                .status(),
            VerificationStatus::Refuted
        );
    }
}
