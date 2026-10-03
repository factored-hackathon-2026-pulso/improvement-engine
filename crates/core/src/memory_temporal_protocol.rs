//! U23-P temporal boundary for governed published-memory use.
//!
//! This is deliberately only the temporal protocol, not the U23 E0 runner.
//! It receives no wiki bytes, does not publish a revision, and cannot score or
//! release a candidate. U22/U33 remain the authority for a governed receipt.

use crate::ArtifactRepository;
use crate::enriched_history::VerifiedReplayAvailability;
use crate::governed_memory_use::{
    MemoryUseAdmission, MemoryUseAdmissionError, MemoryUseRequest, VerifiedMemoryUse,
};
use crate::memory_store::{AtomicMemoryUseCommitPort, MemoryScope};
use sha2::{Digest, Sha256};

/// The replay cutoff already established by the U04-B availability boundary.
///
/// The caller obtains this only after U04-B has validated its sealed source
/// snapshot and availability projection. This small contract never reopens
/// source files or interprets outcome data.
/// Opaque evidence emitted by the U04-B availability boundary (or its future
/// replay-runner adapter). It binds the full governed-use context to an
/// exact U04-B snapshot/profile commitments and is deliberately neither
/// constructible nor inspectable by callers.
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
    grant_revision: u64,
    purpose: String,
}

/// Opaque U04-B availability projection. Its only production factory is the
/// verified U04-B adapter; it intentionally has no public constructor or raw
/// timestamp getters.
#[allow(dead_code)] // Invoked by the future trusted service composition root.
pub(crate) struct VerifiedAvailabilityProjection {
    request: MemoryUseRequest,
    cutoff_at_unix_seconds: u64,
    outcome_available_at_unix_seconds: Option<u64>,
    outcome_provenance: Option<String>,
    source_snapshot_digest: String,
    availability_profile_digest: String,
}

impl VerifiedAvailabilityProjection {
    /// The production U04-B composition path. It accepts only the opaque
    /// projection emitted after a V2 adapter revalidates the exact parsed
    /// snapshot and availability profile. No transport caller can construct
    /// either value or choose a separate replay clock.
    #[allow(dead_code)] // Invoked by the future trusted service composition root.
    pub(crate) fn from_u04b_replay(
        replay: VerifiedReplayAvailability,
        request: MemoryUseRequest,
    ) -> Result<Self, TemporalProtocolError> {
        if replay.tenant_id() != request.scope().tenant_id
            || replay.world_ref() != request.scope().world
        {
            return Err(TemporalProtocolError::U04BReplayScopeMismatch);
        }
        Ok(Self {
            cutoff_at_unix_seconds: replay.cutoff_at_unix_seconds(),
            source_snapshot_digest: replay.source_snapshot_digest().to_owned(),
            availability_profile_digest: replay.availability_profile_digest().to_owned(),
            request,
            outcome_available_at_unix_seconds: None,
            outcome_provenance: None,
        })
    }

    #[cfg(test)]
    fn deterministic(
        request: MemoryUseRequest,
        cutoff_at_unix_seconds: u64,
        outcome_available_at_unix_seconds: Option<u64>,
        outcome_provenance: Option<String>,
    ) -> Self {
        Self {
            request,
            cutoff_at_unix_seconds,
            outcome_available_at_unix_seconds,
            outcome_provenance,
            source_snapshot_digest: "sha256:test_source_snapshot".to_owned(),
            availability_profile_digest: "sha256:test_availability_profile".to_owned(),
        }
    }
}

/// Crate-private stand-in for the U04-B/future runner issuer. The service
/// composition owns it; consumers only receive opaque evidence.
#[allow(dead_code)] // Called by the future U04-B/replay composition root.
pub(crate) struct TrustedTemporalEvidenceIssuer {
    projection: VerifiedAvailabilityProjection,
}

impl TrustedTemporalEvidenceIssuer {
    /// Composition-only factory for a U04-B replay projection. It is
    /// crate-private: transport callers cannot select a snapshot/profile,
    /// tenant/world or cutoff and therefore cannot mint temporal evidence.
    /// This source has no sealed outcome adapter yet (U20-E/U27), so it can
    /// only result in an admitted `Frozen` use; `Continuous` evidence is
    /// rejected for missing outcome evidence before U33 observes a receipt.
    #[allow(dead_code)] // Invoked by the future trusted service composition root.
    pub(crate) fn from_u04b_replay(
        replay: VerifiedReplayAvailability,
        request: MemoryUseRequest,
    ) -> Result<Self, TemporalProtocolError> {
        let projection = VerifiedAvailabilityProjection::from_u04b_replay(replay, request)?;
        Ok(Self { projection })
    }

    #[cfg(test)]
    fn deterministic(projection: VerifiedAvailabilityProjection) -> Self {
        Self { projection }
    }

    #[allow(dead_code)]
    pub(crate) fn attest(&self, protocol: MemoryTemporalProtocol) -> TemporalMemoryEvidence {
        let request = &self.projection.request;
        let commitment = temporal_commitment(&TemporalCommitmentInput {
            protocol,
            scope: request.scope(),
            access: request.access(),
            allowed_at: request.allowed_at_unix_seconds(),
            cutoff: self.projection.cutoff_at_unix_seconds,
            outcome_at: self.projection.outcome_available_at_unix_seconds,
            outcome_provenance: self.projection.outcome_provenance.as_deref(),
            source_snapshot_digest: &self.projection.source_snapshot_digest,
            availability_profile_digest: &self.projection.availability_profile_digest,
        });
        TemporalMemoryEvidence {
            commitment,
            protocol,
            cutoff_at_unix_seconds: self.projection.cutoff_at_unix_seconds,
            memory_use_at_unix_seconds: request.allowed_at_unix_seconds(),
            outcome_available_at_unix_seconds: self.projection.outcome_available_at_unix_seconds,
            outcome_provenance: self.projection.outcome_provenance.clone(),
            scope: request.scope().clone(),
            snapshot_ref: request.access().snapshot_ref.clone(),
            run_id: request.access().run_id.clone(),
            grant_id: request.access().grant_id.clone(),
            grant_revision: request.access().grant_revision,
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
    GrantRevisionMismatch,
    OutcomeForbiddenInFrozen,
    OutcomeRequired,
    OutcomeAfterMemoryUse,
    OutcomeAfterReplayCutoff,
    U04BReplayScopeMismatch,
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
    pub(crate) fn admit<R: ArtifactRepository, P: AtomicMemoryUseCommitPort>(
        protocol: MemoryTemporalProtocol,
        evidence: TemporalMemoryEvidence,
        publisher: &mut P,
        artifacts: &mut R,
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
        if evidence.grant_revision != request.access().grant_revision {
            return Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::GrantRevisionMismatch,
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
        let admitted = MemoryUseAdmission::admit(publisher, artifacts, request)
            .map_err(TemporalMemoryAdmissionError::Governed)?;
        // The atomic U33 commit recomputes canonical receipt identity,
        // liveness, authorization and this exact commitment before exposing a
        // receipt; this comparison is only a defensive output check.
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

/// U23-E uses the same U04-B trusted clock as U23-P, but carries the result to
/// the E0-specific U33-E sidecar instead of the generic U22 memory ledger.
/// This helper emits only the temporal commitment after the pure Frozen gate;
/// U33-E still rechecks exact publication, head, liveness and grant at commit.
#[allow(dead_code)] // Used by the trusted U23-E runtime composition once wired.
pub(crate) fn attest_frozen_e0_reuse(
    replay: &VerifiedReplayAvailability,
    scope: &MemoryScope,
    access: &crate::wiki_scratch::WikiAccess,
) -> Result<String, TemporalProtocolError> {
    if replay.tenant_id() != scope.tenant_id
        || replay.world_ref() != scope.world
        || access.tenant_id != scope.tenant_id
        || access.purpose != scope.purpose
        || access.memory_scope.world != scope.world
        || access.memory_scope.campaign != scope.campaign
        || access.memory_scope.protocol != scope.protocol
        || access.memory_scope.partition != scope.partition
    {
        return Err(TemporalProtocolError::U04BReplayScopeMismatch);
    }
    let request = MemoryUseRequest::new(scope.clone(), access.clone());
    let projection = VerifiedAvailabilityProjection {
        cutoff_at_unix_seconds: replay.cutoff_at_unix_seconds(),
        source_snapshot_digest: replay.source_snapshot_digest().to_owned(),
        availability_profile_digest: replay.availability_profile_digest().to_owned(),
        request,
        outcome_available_at_unix_seconds: None,
        outcome_provenance: None,
    };
    let evidence =
        TrustedTemporalEvidenceIssuer { projection }.attest(MemoryTemporalProtocol::Frozen);
    MemoryTemporalProtocol::Frozen.validate_evidence(scope, &evidence)?;
    Ok(evidence.commitment)
}

#[allow(dead_code)]
struct TemporalCommitmentInput<'a> {
    protocol: MemoryTemporalProtocol,
    scope: &'a MemoryScope,
    access: &'a crate::wiki_scratch::WikiAccess,
    allowed_at: u64,
    cutoff: u64,
    outcome_at: Option<u64>,
    outcome_provenance: Option<&'a str>,
    source_snapshot_digest: &'a str,
    availability_profile_digest: &'a str,
}

#[allow(dead_code)]
fn temporal_commitment(input: &TemporalCommitmentInput<'_>) -> String {
    let bytes = serde_json::to_vec(&(
        input.protocol.as_str(),
        input.scope,
        &input.access.snapshot_ref,
        &input.access.run_id,
        &input.access.grant_id,
        input.access.grant_revision,
        &input.access.purpose,
        input.allowed_at,
        input.cutoff,
        input.outcome_at,
        input.outcome_provenance,
        input.source_snapshot_digest,
        input.availability_profile_digest,
    ))
    .expect("temporal evidence inputs serialize deterministically");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        MemoryTemporalAdmission, MemoryTemporalProtocol, TemporalMemoryAdmissionError,
        TemporalProtocolError, TrustedTemporalEvidenceIssuer, VerifiedAvailabilityProjection,
    };
    use crate::enriched_history::{
        AvailabilityClockMode, AvailabilityProfile, EnrichedHistoryAdapter,
        EnrichedHistoryManifest, PackageFile, ProvenanceDigests,
    };
    use crate::governed_memory_use::MemoryUseRequest;
    use crate::memory_store::{
        InMemoryGovernedMemoryCommitPort, InMemoryMemoryRegistry, MemoryPublisher, MemoryScope,
    };
    use crate::source_validation::SourceSnapshot;
    use crate::wiki_scratch::{
        InMemoryWikiGrantAuthority, MemoryScopeBinding, WikiAccess, WikiGrant,
    };
    use crate::{ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository};
    use serde_json::json;

    const TENANT: &str = "tenant-a";
    const WIKI_ID: &str = "018f50a1-7f00-7000-8000-000000000023";

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn u04b_replay_projection() -> crate::enriched_history::VerifiedReplayAvailability {
        u04b_replay_projection_for(TENANT, "")
    }

    fn u04b_replay_projection_for(
        tenant_id: &str,
        raw_prefix: &str,
    ) -> crate::enriched_history::VerifiedReplayAvailability {
        let snapshot = SourceSnapshot::from_json(&format!(
            "{raw_prefix}{}",
            json!({
                "contract_version": {"major": 1, "minor": 0},
                "tenant_id": tenant_id,
                "source_namespace": "platform_history",
                "world_ref": "world-a",
                "observed_cutoff": "1970-01-01T00:01:40Z",
                "sources": [{
                    "table": "case",
                    "uri": "file://fixture.csv",
                    "file_digest": digest('a'),
                    "header_digest": digest('b'),
                    "row_count": 1,
                    "source_contract_ref": {"id": "case", "version": "v1", "digest": digest('c')}
                }]
            })
        ))
        .expect("fixed U04-B source snapshot");
        let profile = AvailabilityProfile::new(
            "e0_replay",
            1,
            AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed"),
            tenant_id,
            snapshot.binding_digest(),
        );
        let manifest = EnrichedHistoryManifest::new_replay(
            "platform_history",
            "world-a",
            "1970-01-01T00:01:40Z",
            profile,
            vec![
                PackageFile::new(
                    "case",
                    ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                    "1970-01-01T00:01:40Z",
                )
                .with_field_availability(BTreeMap::from([(
                    "event_time".to_owned(),
                    "1970-01-01T00:01:40Z".to_owned(),
                )]))
                .with_replay_projection_digest(digest('e'))
                .with_source_file_seal(
                    snapshot
                        .source_file_seal("case")
                        .expect("fixed snapshot source seal"),
                ),
            ],
        );
        EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot)
            .expect("fixed replay package bound to its snapshot")
            .verified_replay_availability(&snapshot)
            .expect("U04-B replay projection")
    }

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
        InMemoryGovernedMemoryCommitPort,
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
        let authority = InMemoryWikiGrantAuthority::default();
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
        (
            artifacts,
            InMemoryGovernedMemoryCommitPort::new(registry, authority),
            access,
        )
    }

    #[test]
    fn continuous_admission_rejects_an_outcome_not_yet_available_without_recording_a_receipt() {
        let (mut artifacts, mut publisher, access) = seeded("continuous");

        let request = MemoryUseRequest::new(scope("continuous"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(
                request.clone(),
                100,
                Some(101),
                Some("outcome:1".into()),
            ),
        )
        .attest(MemoryTemporalProtocol::Continuous);
        let result = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Continuous,
            evidence,
            &mut publisher,
            &mut artifacts,
            request,
        );

        match result {
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::OutcomeAfterMemoryUse,
            )) => {}
            Err(other) => panic!("expected temporal outcome denial, got {other:?}"),
            Ok(_) => panic!("future outcome must not mint a capability"),
        }
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn admission_binds_its_temporal_claim_to_the_u33_receipt_clock_before_recording() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");

        let request = MemoryUseRequest::new(scope("frozen"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(request.clone(), 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);
        let result = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            evidence,
            &mut publisher,
            &mut artifacts,
            request,
        );

        let admitted = result.expect("issuer binds the exact access time");
        assert_eq!(
            admitted.receipt().temporal_commitment,
            publisher.receipts()[0].temporal_commitment
        );
        assert!(admitted.receipt().temporal_commitment.is_some());
    }

    #[test]
    fn frozen_admission_mints_only_the_existing_u22_receipt_provenance() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");

        let request = MemoryUseRequest::new(scope("frozen"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(request.clone(), 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);
        let admitted = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            evidence,
            &mut publisher,
            &mut artifacts,
            request,
        )
        .expect("timely U22/U33 frozen admission");

        assert_eq!(admitted.head_version(), 1);
        assert_eq!(admitted.run_id(), "run-2");
        assert_eq!(publisher.receipts().len(), 1);
    }

    #[test]
    fn production_issuer_only_accepts_a_revalidated_u04b_replay_projection() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access);
        let issuer = TrustedTemporalEvidenceIssuer::from_u04b_replay(
            u04b_replay_projection(),
            request.clone(),
        )
        .expect("tenant and world were bound by the verified U04-B projection");
        let admitted = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            issuer.attest(MemoryTemporalProtocol::Frozen),
            &mut publisher,
            &mut artifacts,
            request,
        )
        .expect("bound replay projection can produce only the governed receipt");

        assert_eq!(admitted.scope().tenant_id, TENANT);
        assert_eq!(publisher.receipts().len(), 1);
    }

    #[test]
    fn production_issuer_rejects_u04b_projection_for_another_world_before_a_receipt() {
        let (_artifacts, publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(
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
            TrustedTemporalEvidenceIssuer::from_u04b_replay(u04b_replay_projection(), request),
            Err(TemporalProtocolError::U04BReplayScopeMismatch)
        ));
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn production_issuer_rejects_u04b_projection_for_another_tenant_before_a_receipt() {
        let (_artifacts, publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access);

        assert!(matches!(
            TrustedTemporalEvidenceIssuer::from_u04b_replay(
                u04b_replay_projection_for("tenant-b", ""),
                request,
            ),
            Err(TemporalProtocolError::U04BReplayScopeMismatch)
        ));
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn source_snapshot_and_profile_digest_change_the_temporal_commitment() {
        let (_artifacts, _publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access);
        let first = TrustedTemporalEvidenceIssuer::from_u04b_replay(
            u04b_replay_projection_for(TENANT, ""),
            request.clone(),
        )
        .expect("first exact U04-B snapshot");
        let same_semantics_new_snapshot = TrustedTemporalEvidenceIssuer::from_u04b_replay(
            u04b_replay_projection_for(TENANT, "\n"),
            request,
        )
        .expect("second exact U04-B snapshot");

        assert_ne!(
            first.attest(MemoryTemporalProtocol::Frozen).commitment,
            same_semantics_new_snapshot
                .attest(MemoryTemporalProtocol::Frozen)
                .commitment,
            "raw snapshot and profile seals are part of temporal identity"
        );
    }

    #[test]
    fn reissued_grant_revision_cannot_reuse_prior_temporal_evidence_or_leave_a_receipt() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");
        let original_request = MemoryUseRequest::new(scope("frozen"), access.clone());
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(original_request, 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);

        let binding = MemoryScopeBinding::new("world-a", "campaign-a", "frozen", "train");
        publisher.authority().issue(
            WikiGrant::new_scoped(
                "grant-2",
                "run-2",
                TENANT,
                "investigation",
                access.snapshot_ref.clone(),
                binding,
            )
            .with_revision(2),
        );
        let reissued_request =
            MemoryUseRequest::new(scope("frozen"), access.with_grant_revision(2));

        assert!(matches!(
            MemoryTemporalAdmission::admit(
                MemoryTemporalProtocol::Frozen,
                evidence,
                &mut publisher,
                &mut artifacts,
                reissued_request,
            ),
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::GrantRevisionMismatch
            ))
        ));
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn evidence_rejects_caller_elevation_of_allowed_at_before_u33_side_effects() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(request.clone(), 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);
        let original_access = request.access().clone();
        let elevated = MemoryUseRequest::new(
            scope("frozen"),
            WikiAccess::new_scoped(
                original_access.run_id,
                original_access.tenant_id,
                original_access.purpose,
                original_access.grant_id,
                original_access.snapshot_ref,
                101,
                original_access.memory_scope,
            ),
        );

        assert!(matches!(
            MemoryTemporalAdmission::admit(
                MemoryTemporalProtocol::Frozen,
                evidence,
                &mut publisher,
                &mut artifacts,
                elevated,
            ),
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch
            ))
        ));
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn evidence_cannot_cross_a_memory_scope_before_u33_side_effects() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access.clone());
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(request.clone(), 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);
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
                &mut publisher,
                &mut artifacts,
                crossed,
            ),
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch
            ))
        ));
        assert!(publisher.receipts().is_empty());
    }
}
