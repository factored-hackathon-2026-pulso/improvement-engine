//! U23-P temporal boundary for governed published-memory use.
//!
//! This is deliberately only the temporal protocol, not the U23 E0 runner.
//! It receives no wiki bytes, does not publish a revision, and cannot score or
//! release a candidate. U22/U33 remain the authority for a governed receipt.

use crate::ArtifactRepository;
use crate::governed_memory_use::{
    MemoryUseAdmission, MemoryUseAdmissionError, MemoryUseRequest, VerifiedMemoryUse,
};
use crate::memory_store::{MemoryPublisher, MemoryScope, MemoryUseReceiptAttestationPort};
use crate::wiki_scratch::WikiAuthorizationPort;
use sha2::{Digest, Sha256};

/// The replay cutoff already established by the U04-B availability boundary.
///
/// The caller obtains this only after U04-B has validated its sealed source
/// snapshot and availability projection. This small contract never reopens
/// source files or interprets outcome data.
/// Opaque evidence emitted by the U04-B availability boundary (or its future
/// replay-runner adapter). It binds the full governed-use context to an
/// authority-issued nonce and is deliberately neither constructible nor
/// inspectable by callers.
pub struct TemporalMemoryEvidence {
    commitment: String,
    protocol: MemoryTemporalProtocol,
    cutoff_at_unix_seconds: u64,
    memory_use_at_unix_seconds: u64,
    outcome_available_at_unix_seconds: Option<u64>,
    outcome_provenance: Option<String>,
    scope: MemoryScope,
    snapshot_ref: crate::ArtifactReference,
    run_id: String,
    grant_id: String,
    purpose: String,
}

/// Crate-private stand-in for the U04-B/future runner issuer. The service
/// composition owns it; consumers only receive opaque evidence.
#[allow(dead_code)] // Called by the future U04-B/replay composition root.
pub(crate) struct TrustedTemporalEvidenceIssuer {
    nonce: String,
}

impl TrustedTemporalEvidenceIssuer {
    /// Composition-only factory for U04-B's verified availability projection.
    /// It is crate-private: transport callers cannot select a nonce, clock or
    /// outcome and therefore cannot mint temporal evidence.
    #[allow(dead_code)]
    pub(crate) fn from_u04b_verified_projection(authority_nonce: String) -> Self {
        Self {
            nonce: authority_nonce,
        }
    }

    #[cfg(test)]
    fn deterministic(nonce: impl Into<String>) -> Self {
        Self {
            nonce: nonce.into(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn attest(
        &self,
        protocol: MemoryTemporalProtocol,
        request: &MemoryUseRequest,
        cutoff_at_unix_seconds: u64,
        outcome_available_at_unix_seconds: Option<u64>,
        outcome_provenance: Option<String>,
    ) -> TemporalMemoryEvidence {
        let commitment = temporal_commitment(
            &self.nonce,
            protocol,
            request.scope(),
            request.access(),
            request.allowed_at_unix_seconds(),
            cutoff_at_unix_seconds,
            outcome_available_at_unix_seconds,
            outcome_provenance.as_deref(),
        );
        TemporalMemoryEvidence {
            commitment,
            protocol,
            cutoff_at_unix_seconds,
            memory_use_at_unix_seconds: request.allowed_at_unix_seconds(),
            outcome_available_at_unix_seconds,
            outcome_provenance,
            scope: request.scope().clone(),
            snapshot_ref: request.access().snapshot_ref.clone(),
            run_id: request.access().run_id.clone(),
            grant_id: request.access().grant_id.clone(),
            purpose: request.access().purpose.clone(),
        }
    }
}

/// The two allowed temporal semantics for a partitioned memory head.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryTemporalProtocol {
    Frozen,
    Continuous,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemporalProtocolError {
    ProtocolMismatch {
        expected: &'static str,
        actual: String,
    },
    MemoryAfterReplayCutoff,
    AccessTimeMismatch,
    OutcomeForbiddenInFrozen,
    OutcomeRequired,
    OutcomeAfterMemoryUse,
    OutcomeAfterReplayCutoff,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemporalMemoryAdmissionError {
    Temporal(TemporalProtocolError),
    Governed(MemoryUseAdmissionError),
}

/// Trusted composition entry point for the U23-P temporal gate.
///
/// It deliberately runs the pure temporal check before U22 records a receipt,
/// so a not-yet-observable outcome cannot create an audit/use side effect. The
/// underlying U22 call remains crate-private: an external caller cannot supply
/// a permissive publisher or receive a use capability from this boundary.
///
/// ```compile_fail
/// use improvement_engine_core::memory_temporal_protocol::MemoryTemporalAdmission;
/// let _ = MemoryTemporalAdmission::admit;
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::memory_temporal_protocol::MemoryTemporalAdmission;
/// let _ = MemoryTemporalAdmission { _private: true };
/// ```
pub struct MemoryTemporalAdmission {
    _private: bool,
}

impl MemoryTemporalAdmission {
    #[allow(dead_code)] // Invoked by the future trusted service composition root.
    pub(crate) fn admit<
        R: ArtifactRepository,
        P: MemoryPublisher + MemoryUseReceiptAttestationPort,
        A: WikiAuthorizationPort,
    >(
        protocol: MemoryTemporalProtocol,
        evidence: TemporalMemoryEvidence,
        publisher: &mut P,
        artifacts: &mut R,
        authority: &A,
        request: MemoryUseRequest,
    ) -> Result<VerifiedMemoryUse, TemporalMemoryAdmissionError> {
        if evidence.memory_use_at_unix_seconds != request.allowed_at_unix_seconds() {
            return Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch,
            ));
        }
        if evidence.scope != *request.scope()
            || evidence.snapshot_ref != request.access().snapshot_ref
            || evidence.run_id != request.access().run_id
            || evidence.grant_id != request.access().grant_id
            || evidence.purpose != request.access().purpose
        {
            return Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch,
            ));
        }
        if evidence.protocol != protocol {
            return Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::ProtocolMismatch {
                    expected: protocol.as_str(),
                    actual: evidence.protocol.as_str().to_owned(),
                },
            ));
        }
        protocol
            .validate_evidence(request.scope(), &evidence)
            .map_err(TemporalMemoryAdmissionError::Temporal)?;
        let request = request.with_temporal_commitment(evidence.commitment.clone());
        let admitted = MemoryUseAdmission::admit(publisher, artifacts, authority, request)
            .map_err(TemporalMemoryAdmissionError::Governed)?;
        // Re-attest after the temporal decision. U33 recomputes canonical
        // receipt identity, liveness, authorization and this exact commitment.
        if admitted.receipt().temporal_commitment.as_deref() != Some(evidence.commitment.as_str()) {
            return Err(TemporalMemoryAdmissionError::Governed(
                MemoryUseAdmissionError::ReceiptMismatch,
            ));
        }
        Ok(admitted)
    }
}

impl MemoryTemporalProtocol {
    /// Validates only temporal eligibility. Receipt/authentication/liveness are
    /// deliberately revalidated by the U22/U33 composition boundary.
    fn as_str(self) -> &'static str {
        match self {
            Self::Frozen => "frozen",
            Self::Continuous => "continuous",
        }
    }

    fn validate_evidence(
        self,
        scope: &MemoryScope,
        evidence: &TemporalMemoryEvidence,
    ) -> Result<(), TemporalProtocolError> {
        let expected = match self {
            Self::Frozen => "frozen",
            Self::Continuous => "continuous",
        };
        if scope.protocol != expected {
            return Err(TemporalProtocolError::ProtocolMismatch {
                expected,
                actual: scope.protocol.clone(),
            });
        }
        if evidence.memory_use_at_unix_seconds > evidence.cutoff_at_unix_seconds {
            return Err(TemporalProtocolError::MemoryAfterReplayCutoff);
        }
        match self {
            Self::Frozen
                if evidence.outcome_available_at_unix_seconds.is_some()
                    || evidence.outcome_provenance.is_some() =>
            {
                Err(TemporalProtocolError::OutcomeForbiddenInFrozen)
            }
            Self::Frozen => Ok(()),
            Self::Continuous => {
                let outcome = evidence
                    .outcome_available_at_unix_seconds
                    .ok_or(TemporalProtocolError::OutcomeRequired)?;
                if evidence
                    .outcome_provenance
                    .as_deref()
                    .is_none_or(str::is_empty)
                {
                    return Err(TemporalProtocolError::OutcomeRequired);
                }
                if outcome > evidence.memory_use_at_unix_seconds {
                    return Err(TemporalProtocolError::OutcomeAfterMemoryUse);
                }
                if outcome > evidence.cutoff_at_unix_seconds {
                    return Err(TemporalProtocolError::OutcomeAfterReplayCutoff);
                }
                Ok(())
            }
        }
    }
}

#[allow(dead_code)]
fn temporal_commitment(
    nonce: &str,
    protocol: MemoryTemporalProtocol,
    scope: &MemoryScope,
    access: &crate::wiki_scratch::WikiAccess,
    allowed_at: u64,
    cutoff: u64,
    outcome_at: Option<u64>,
    outcome_provenance: Option<&str>,
) -> String {
    let bytes = serde_json::to_vec(&(
        nonce,
        protocol.as_str(),
        scope,
        &access.snapshot_ref,
        &access.run_id,
        &access.grant_id,
        &access.purpose,
        allowed_at,
        cutoff,
        outcome_at,
        outcome_provenance,
    ))
    .expect("temporal evidence inputs serialize deterministically");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::{
        MemoryTemporalAdmission, MemoryTemporalProtocol, TemporalMemoryAdmissionError,
        TemporalProtocolError, TrustedTemporalEvidenceIssuer,
    };
    use crate::governed_memory_use::MemoryUseRequest;
    use crate::memory_store::{InMemoryMemoryRegistry, MemoryPublisher, MemoryScope};
    use crate::wiki_scratch::{
        InMemoryWikiGrantAuthority, MemoryScopeBinding, WikiAccess, WikiGrant,
    };
    use crate::{ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository};
    use serde_json::json;

    const TENANT: &str = "tenant-a";
    const WIKI_ID: &str = "018f50a1-7f00-7000-8000-000000000023";

    fn scope(protocol: &str) -> MemoryScope {
        MemoryScope::new(
            TENANT,
            "investigation",
            "world-a",
            "campaign-a",
            protocol,
            "train",
        )
    }

    fn seeded(
        protocol: &str,
    ) -> (
        InMemoryArtifactRepository,
        InMemoryMemoryRegistry,
        InMemoryWikiGrantAuthority,
        WikiAccess,
    ) {
        let mut artifacts = InMemoryArtifactRepository::default();
        let snapshot = artifacts
            .append(
                None,
                ArtifactDraft::new(
                    TENANT,
                    WIKI_ID,
                    1,
                    ArtifactKind::MemoryWiki,
                    json!({
                        "available_at_unix_seconds": 100,
                        "purpose": "investigation",
                        "pages": {"index.md": "published"}
                    }),
                    None,
                ),
            )
            .expect("fixed memory wiki")
            .reference();
        let binding = MemoryScopeBinding::new("world-a", "campaign-a", protocol, "train");
        let access = WikiAccess::new_scoped(
            "run-2",
            TENANT,
            "investigation",
            "grant-2",
            snapshot.clone(),
            100,
            binding.clone(),
        );
        let mut authority = InMemoryWikiGrantAuthority::default();
        authority.issue(WikiGrant::new_scoped(
            "grant-2",
            "run-2",
            TENANT,
            "investigation",
            snapshot.clone(),
            binding,
        ));
        let mut registry = InMemoryMemoryRegistry::default();
        registry
            .seed_head(&mut artifacts, scope(protocol), snapshot)
            .expect("fixed memory head");
        (artifacts, registry, authority, access)
    }

    #[test]
    fn continuous_admission_rejects_an_outcome_not_yet_available_without_recording_a_receipt() {
        let (mut artifacts, mut registry, authority, access) = seeded("continuous");

        let request = MemoryUseRequest::new(scope("continuous"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic("u04b").attest(
            MemoryTemporalProtocol::Continuous,
            &request,
            100,
            Some(101),
            Some("outcome:1".into()),
        );
        let result = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Continuous,
            evidence,
            &mut registry,
            &mut artifacts,
            &authority,
            request,
        );

        match result {
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::OutcomeAfterMemoryUse,
            )) => {}
            Err(other) => panic!("expected temporal outcome denial, got {other:?}"),
            Ok(_) => panic!("future outcome must not mint a capability"),
        }
        assert!(registry.receipts().is_empty());
    }

    #[test]
    fn admission_binds_its_temporal_claim_to_the_u33_receipt_clock_before_recording() {
        let (mut artifacts, mut registry, authority, access) = seeded("frozen");

        let request = MemoryUseRequest::new(scope("frozen"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic("u04b").attest(
            MemoryTemporalProtocol::Frozen,
            &request,
            100,
            None,
            None,
        );
        let result = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            evidence,
            &mut registry,
            &mut artifacts,
            &authority,
            request,
        );

        let admitted = result.expect("issuer binds the exact access time");
        assert_eq!(
            admitted.receipt().temporal_commitment,
            registry.receipts()[0].temporal_commitment
        );
        assert!(admitted.receipt().temporal_commitment.is_some());
    }

    #[test]
    fn frozen_admission_mints_only_the_existing_u22_receipt_provenance() {
        let (mut artifacts, mut registry, authority, access) = seeded("frozen");

        let request = MemoryUseRequest::new(scope("frozen"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic("u04b").attest(
            MemoryTemporalProtocol::Frozen,
            &request,
            100,
            None,
            None,
        );
        let admitted = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            evidence,
            &mut registry,
            &mut artifacts,
            &authority,
            request,
        )
        .expect("timely U22/U33 frozen admission");

        assert_eq!(admitted.head_version(), 1);
        assert_eq!(admitted.run_id(), "run-2");
        assert_eq!(registry.receipts().len(), 1);
    }

    #[test]
    fn evidence_rejects_caller_elevation_of_allowed_at_before_u33_side_effects() {
        let (mut artifacts, mut registry, authority, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic("u04b").attest(
            MemoryTemporalProtocol::Frozen,
            &request,
            100,
            None,
            None,
        );
        let mut elevated = request;
        elevated.access.allowed_at_unix_seconds = 101;

        assert!(matches!(
            MemoryTemporalAdmission::admit(
                MemoryTemporalProtocol::Frozen,
                evidence,
                &mut registry,
                &mut artifacts,
                &authority,
                elevated,
            ),
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch
            ))
        ));
        assert!(registry.receipts().is_empty());
    }

    #[test]
    fn evidence_cannot_cross_a_memory_scope_before_u33_side_effects() {
        let (mut artifacts, mut registry, authority, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access.clone());
        let evidence = TrustedTemporalEvidenceIssuer::deterministic("u04b").attest(
            MemoryTemporalProtocol::Frozen,
            &request,
            100,
            None,
            None,
        );
        let crossed = MemoryUseRequest::new(
            MemoryScope::new(
                TENANT,
                "investigation",
                "world-b",
                "campaign-a",
                "frozen",
                "train",
            ),
            access,
        );
        assert!(matches!(
            MemoryTemporalAdmission::admit(
                MemoryTemporalProtocol::Frozen,
                evidence,
                &mut registry,
                &mut artifacts,
                &authority,
                crossed,
            ),
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch
            ))
        ));
        assert!(registry.receipts().is_empty());
    }
}
