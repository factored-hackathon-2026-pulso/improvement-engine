//! U33-E: governed publication of the one static Frozen E0 summary.
//!
//! This is deliberately not the public, generic U33 `MemoryPublisher`
//! boundary. It redeems an opaque U15-EQ preparation only after replaying the
//! exact U13-A -> U14-E -> U14-EQ -> U04-B checks. The result is an opaque
//! publication capability, not a memory-use, proposal, route or release
//! capability.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::ArtifactRepository;
use crate::autonomous_scout::VerifiedScoutCandidate;
use crate::e0_frozen_summary::{FrozenE0SummaryPreparationError, PreparedFrozenE0MemorySummary};
use crate::e0_frozen_verifier::FrozenE0VerificationReport;
use crate::e0_opportunity_qualification::FrozenE0OpportunityQualification;
use crate::enriched_history::VerifiedReplayAvailability;
use crate::memory_store::{MemoryHead, MemoryScope};
use crate::wiki_scratch::{WikiAccess, WikiAuthorizationPort};
use crate::{ArtifactDraft, ArtifactKind, ArtifactReference, RepositoryError};

const SUMMARY_SCHEMA_VERSION: u8 = 1;

/// Opaque evidence that exactly one already-prepared Frozen E0 summary was
/// appended to a memory head. It deliberately does not expose pages, source
/// bytes, workspace details, an authorization object or MemoryUse authority.
pub struct PublishedFrozenE0MemorySummary {
    publication_commitment: String,
    memory_snapshot_ref: ArtifactReference,
    memory_head_version: u64,
}

impl PublishedFrozenE0MemorySummary {
    #[must_use]
    pub fn publication_commitment(&self) -> &str {
        &self.publication_commitment
    }

    #[must_use]
    pub fn memory_snapshot_ref(&self) -> &ArtifactReference {
        &self.memory_snapshot_ref
    }

    #[must_use]
    pub fn memory_head_version(&self) -> u64 {
        self.memory_head_version
    }

    /// U23-E owns any future governed-memory-use admission.
    #[must_use]
    pub fn authorizes_memory_use_or_promotion(&self) -> bool {
        false
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum FrozenE0SummaryPublicationError {
    Preparation(FrozenE0SummaryPreparationError),
    HeadMissing,
    HeadConflict,
    AccessDenied,
    SnapshotInvalid,
    PublicationConflict,
    DependencyUnavailable,
    Repository(RepositoryError),
}

impl From<FrozenE0SummaryPreparationError> for FrozenE0SummaryPublicationError {
    fn from(value: FrozenE0SummaryPreparationError) -> Self {
        Self::Preparation(value)
    }
}

impl From<RepositoryError> for FrozenE0SummaryPublicationError {
    fn from(value: RepositoryError) -> Self {
        Self::Repository(value)
    }
}

/// Private, versioned publication request. The composer is its only issuer;
/// hashes and generic `MemoryPublishRequest` values cannot impersonate it.
#[allow(dead_code)] // Durable adapter consumes every fence; local adapter uses its write subset.
pub(crate) struct FrozenE0SummaryPublicationRequest {
    scope: MemoryScope,
    access: WikiAccess,
    expected_head_version: u64,
    expected_snapshot_ref: ArtifactReference,
    pages: BTreeMap<String, String>,
    preparation_commitment: String,
    candidate_digest: String,
    qualification_commitment: String,
    verification_report_commitment: String,
    replay_commitment: String,
    replay_source_snapshot_binding: String,
    replay_availability_profile: String,
    replay_cutoff_at_unix_seconds: u64,
    source_snapshot_ref: ArtifactReference,
    transform_commitment: String,
    result_commitment: String,
    publication_commitment: String,
}

/// Versioned U33-E sidecar model. The local fixture holds it in memory; a
/// durable adapter must persist its equivalent atomically with U02 append and
/// head CAS, not add E0 semantics to the generic U33 transform receipt.
#[allow(dead_code)] // Retained for the future U23-E exact-provenance reattestation.
struct FrozenE0PublicationRecord {
    schema_version: u8,
    published: PublishedFrozenE0MemorySummary,
    scope: MemoryScope,
    access: WikiAccess,
    preparation_commitment: String,
    candidate_digest: String,
    qualification_commitment: String,
    verification_report_commitment: String,
    replay_commitment: String,
    replay_source_snapshot_binding: String,
    replay_availability_profile: String,
    replay_cutoff_at_unix_seconds: u64,
    source_snapshot_ref: ArtifactReference,
    transform_commitment: String,
    result_commitment: String,
}

/// U33-E storage seam. A durable adapter must evaluate its grant revision and
/// liveness together with all head/provenance predicates in one transaction.
/// This crate intentionally ships no pretend durable success while the U05
/// transaction-scoped grant projection is unavailable.
pub(crate) trait FrozenE0SummaryPublicationPort {
    fn publish<R: ArtifactRepository>(
        &mut self,
        artifacts: &mut R,
        request: FrozenE0SummaryPublicationRequest,
    ) -> Result<PublishedFrozenE0MemorySummary, FrozenE0SummaryPublicationError>;
}

/// Explicit fail-closed placeholder for PostgreSQL/AWS composition. It exists
/// so callers cannot mistake the local fixture for a durable authorization
/// implementation.
#[allow(dead_code)] // Wired only by the future trusted service composition root.
pub(crate) struct DurableFrozenE0SummaryPublicationUnavailable;

impl FrozenE0SummaryPublicationPort for DurableFrozenE0SummaryPublicationUnavailable {
    fn publish<R: ArtifactRepository>(
        &mut self,
        _: &mut R,
        _: FrozenE0SummaryPublicationRequest,
    ) -> Result<PublishedFrozenE0MemorySummary, FrozenE0SummaryPublicationError> {
        Err(FrozenE0SummaryPublicationError::DependencyUnavailable)
    }
}

/// Deterministic local fixture for the semantic U33-E contract. It is not a
/// durable implementation and does not claim cross-process transactional grant
/// liveness. The trusted composer repeats authorization immediately before it
/// calls this port; production remains unavailable until U05 can be checked in
/// the same durable conditional commit.
#[allow(dead_code)] // Fixture is selected only by test composition.
#[derive(Default)]
pub(crate) struct InMemoryFrozenE0SummaryPublicationPort {
    heads: BTreeMap<MemoryScope, MemoryHead>,
    publications: BTreeMap<String, FrozenE0PublicationRecord>,
}

#[allow(dead_code)] // Fixture is selected only by test composition.
impl InMemoryFrozenE0SummaryPublicationPort {
    pub(crate) fn seed_head<R: ArtifactRepository>(
        &mut self,
        artifacts: &mut R,
        scope: MemoryScope,
        snapshot_ref: ArtifactReference,
    ) -> Result<MemoryHead, FrozenE0SummaryPublicationError> {
        if self.heads.contains_key(&scope) {
            return Err(FrozenE0SummaryPublicationError::PublicationConflict);
        }
        let snapshot = artifacts
            .get(
                &snapshot_ref.tenant_id,
                &snapshot_ref.id,
                snapshot_ref.revision,
            )?
            .ok_or(FrozenE0SummaryPublicationError::SnapshotInvalid)?;
        if snapshot.reference() != snapshot_ref
            || snapshot.kind != ArtifactKind::MemoryWiki
            || snapshot.tenant_id != scope.tenant_id
        {
            return Err(FrozenE0SummaryPublicationError::SnapshotInvalid);
        }
        let head = MemoryHead {
            scope: scope.clone(),
            snapshot_ref,
            head_version: 1,
        };
        self.heads.insert(scope, head.clone());
        Ok(head)
    }

    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn advance_head_for_test<R: ArtifactRepository>(
        &mut self,
        artifacts: &mut R,
        scope: &MemoryScope,
    ) -> Result<MemoryHead, FrozenE0SummaryPublicationError> {
        let head = self
            .heads
            .get(scope)
            .cloned()
            .ok_or(FrozenE0SummaryPublicationError::HeadMissing)?;
        let base = artifacts
            .get(
                &head.snapshot_ref.tenant_id,
                &head.snapshot_ref.id,
                head.snapshot_ref.revision,
            )?
            .ok_or(FrozenE0SummaryPublicationError::SnapshotInvalid)?;
        let next = artifacts.append(
            Some(base.revision),
            ArtifactDraft::new(
                scope.tenant_id.clone(),
                base.id,
                base.revision
                    .checked_add(1)
                    .ok_or(FrozenE0SummaryPublicationError::SnapshotInvalid)?,
                ArtifactKind::MemoryWiki,
                json!({
                    "available_at_unix_seconds": 100,
                    "purpose": scope.purpose.clone(),
                    "pages": {"external-head-advance.md": "fixture"},
                }),
                None,
            ),
        )?;
        let advanced = MemoryHead {
            scope: scope.clone(),
            snapshot_ref: next.reference(),
            head_version: head
                .head_version
                .checked_add(1)
                .ok_or(FrozenE0SummaryPublicationError::SnapshotInvalid)?,
        };
        self.heads.insert(scope.clone(), advanced.clone());
        Ok(advanced)
    }

    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn publication_count_for_test(&self) -> usize {
        self.publications.len()
    }

    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn head_for_test(&self, scope: &MemoryScope) -> Option<MemoryHead> {
        self.heads.get(scope).cloned()
    }
}

impl FrozenE0SummaryPublicationPort for InMemoryFrozenE0SummaryPublicationPort {
    fn publish<R: ArtifactRepository>(
        &mut self,
        artifacts: &mut R,
        request: FrozenE0SummaryPublicationRequest,
    ) -> Result<PublishedFrozenE0MemorySummary, FrozenE0SummaryPublicationError> {
        if let Some(existing) = self.publications.get(&request.publication_commitment) {
            return Ok(PublishedFrozenE0MemorySummary {
                publication_commitment: existing.published.publication_commitment.clone(),
                memory_snapshot_ref: existing.published.memory_snapshot_ref.clone(),
                memory_head_version: existing.published.memory_head_version,
            });
        }
        let head = self
            .heads
            .get(&request.scope)
            .cloned()
            .ok_or(FrozenE0SummaryPublicationError::HeadMissing)?;
        if head.head_version != request.expected_head_version
            || head.snapshot_ref != request.expected_snapshot_ref
            || request.access.snapshot_ref != head.snapshot_ref
        {
            return Err(FrozenE0SummaryPublicationError::HeadConflict);
        }
        let base = artifacts
            .get(
                &head.snapshot_ref.tenant_id,
                &head.snapshot_ref.id,
                head.snapshot_ref.revision,
            )?
            .ok_or(FrozenE0SummaryPublicationError::SnapshotInvalid)?;
        if base.reference() != head.snapshot_ref
            || base.kind != ArtifactKind::MemoryWiki
            || base.tenant_id != request.scope.tenant_id
        {
            return Err(FrozenE0SummaryPublicationError::SnapshotInvalid);
        }
        let next_revision = base
            .revision
            .checked_add(1)
            .ok_or(FrozenE0SummaryPublicationError::SnapshotInvalid)?;
        let next_head_version = head
            .head_version
            .checked_add(1)
            .ok_or(FrozenE0SummaryPublicationError::SnapshotInvalid)?;
        let next = artifacts.append(
            Some(base.revision),
            ArtifactDraft::new(
                request.scope.tenant_id.clone(),
                base.id.clone(),
                next_revision,
                ArtifactKind::MemoryWiki,
                json!({
                    "available_at_unix_seconds": request.access.allowed_at_unix_seconds,
                    "purpose": request.scope.purpose,
                    "pages": request.pages,
                }),
                None,
            ),
        )?;
        let published = PublishedFrozenE0MemorySummary {
            publication_commitment: request.publication_commitment,
            memory_snapshot_ref: next.reference(),
            memory_head_version: next_head_version,
        };
        self.heads.insert(
            request.scope.clone(),
            MemoryHead {
                scope: head.scope,
                snapshot_ref: published.memory_snapshot_ref.clone(),
                head_version: next_head_version,
            },
        );
        self.publications.insert(
            published.publication_commitment.clone(),
            FrozenE0PublicationRecord {
                schema_version: SUMMARY_SCHEMA_VERSION,
                scope: request.scope,
                access: request.access,
                preparation_commitment: request.preparation_commitment,
                candidate_digest: request.candidate_digest,
                qualification_commitment: request.qualification_commitment,
                verification_report_commitment: request.verification_report_commitment,
                replay_commitment: request.replay_commitment,
                replay_source_snapshot_binding: request.replay_source_snapshot_binding,
                replay_availability_profile: request.replay_availability_profile,
                replay_cutoff_at_unix_seconds: request.replay_cutoff_at_unix_seconds,
                source_snapshot_ref: request.source_snapshot_ref,
                transform_commitment: request.transform_commitment,
                result_commitment: request.result_commitment,
                published: PublishedFrozenE0MemorySummary {
                    publication_commitment: published.publication_commitment.clone(),
                    memory_snapshot_ref: published.memory_snapshot_ref.clone(),
                    memory_head_version: published.memory_head_version,
                },
            },
        );
        Ok(published)
    }
}

/// Crate-private composition root. Generic U33 publication is intentionally
/// not called: its request cannot carry/revalidate this E0 provenance chain.
#[allow(dead_code)] // Wired only by the future trusted service composition root.
pub(crate) struct FrozenE0SummaryPublisher;

#[allow(dead_code)] // Wired only by the future trusted service composition root.
impl FrozenE0SummaryPublisher {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn publish<
        R: ArtifactRepository,
        A: WikiAuthorizationPort,
        P: FrozenE0SummaryPublicationPort,
    >(
        prepared: &PreparedFrozenE0MemorySummary,
        qualification: &FrozenE0OpportunityQualification,
        candidate: &VerifiedScoutCandidate,
        report: &FrozenE0VerificationReport,
        replay: &VerifiedReplayAvailability,
        scope: &MemoryScope,
        access: &WikiAccess,
        expected_head: &MemoryHead,
        authority: &A,
        port: &mut P,
        repository: &mut R,
    ) -> Result<PublishedFrozenE0MemorySummary, FrozenE0SummaryPublicationError> {
        prepared.revalidate_for_publication(
            qualification,
            candidate,
            report,
            replay,
            scope,
            access,
            repository,
        )?;
        if expected_head.scope != *scope || expected_head.snapshot_ref != access.snapshot_ref {
            return Err(FrozenE0SummaryPublicationError::HeadConflict);
        }
        if !authority.authorize(access, &access.snapshot_ref) {
            return Err(FrozenE0SummaryPublicationError::AccessDenied);
        }
        let publication_commitment = digest(&PublicationCommitment {
            schema_version: SUMMARY_SCHEMA_VERSION,
            preparation_commitment: prepared.preparation_commitment(),
            candidate_digest: prepared.candidate_digest(),
            qualification_commitment: prepared.qualification_commitment(),
            verification_report_commitment: prepared.verification_report_commitment(),
            replay_commitment: prepared.replay_commitment(),
            access_commitment: prepared.access_commitment(),
            transform_commitment: prepared.transform_commitment(),
            result_commitment: prepared.result_commitment(),
            scope,
            expected_snapshot_ref: &access.snapshot_ref,
            expected_head_version: expected_head.head_version,
        });
        port.publish(
            repository,
            FrozenE0SummaryPublicationRequest {
                scope: scope.clone(),
                access: access.clone(),
                expected_head_version: expected_head.head_version,
                expected_snapshot_ref: access.snapshot_ref.clone(),
                pages: prepared.scratch_result().pages.clone(),
                preparation_commitment: prepared.preparation_commitment().to_owned(),
                candidate_digest: prepared.candidate_digest().to_owned(),
                qualification_commitment: prepared.qualification_commitment().to_owned(),
                verification_report_commitment: prepared
                    .verification_report_commitment()
                    .to_owned(),
                replay_commitment: prepared.replay_commitment().to_owned(),
                replay_source_snapshot_binding: replay.source_snapshot_digest().to_owned(),
                replay_availability_profile: replay.availability_profile_digest().to_owned(),
                replay_cutoff_at_unix_seconds: replay.cutoff_at_unix_seconds(),
                source_snapshot_ref: candidate.source_snapshot_ref().clone(),
                transform_commitment: prepared.transform_commitment().to_owned(),
                result_commitment: prepared.result_commitment().to_owned(),
                publication_commitment,
            },
        )
    }
}

#[derive(Serialize)]
struct PublicationCommitment<'a> {
    schema_version: u8,
    preparation_commitment: &'a str,
    candidate_digest: &'a str,
    qualification_commitment: &'a str,
    verification_report_commitment: &'a str,
    replay_commitment: &'a str,
    access_commitment: &'a str,
    transform_commitment: &'a str,
    result_commitment: &'a str,
    scope: &'a MemoryScope,
    expected_snapshot_ref: &'a ArtifactReference,
    expected_head_version: u64,
}

fn digest<T: Serialize>(value: &T) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("U33-E commitments serialize"))
    )
}

/// ```compile_fail
/// use improvement_engine_core::e0_frozen_memory_publication::{FrozenE0SummaryPublisher, PublishedFrozenE0MemorySummary};
/// let _ = FrozenE0SummaryPublisher::publish;
/// let _ = PublishedFrozenE0MemorySummary {};
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::memory_store::MemoryPublishRequest;
/// use improvement_engine_core::e0_frozen_memory_publication::FrozenE0SummaryPublisher;
/// fn wrong(request: MemoryPublishRequest) { let _ = (FrozenE0SummaryPublisher::publish, request); }
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::e0_frozen_memory_publication::PublishedFrozenE0MemorySummary;
/// fn leak(summary: PublishedFrozenE0MemorySummary) -> String { format!("{summary:?}") }
/// ```
const _U33E_COMPOSITION_AND_GENERIC_PUBLICATION_ARE_NOT_PUBLIC: () = ();

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use crate::ArtifactRepository;
    use crate::InMemoryArtifactRepository;
    use crate::autonomous_scout::verified_real_e0_candidate_for_frozen_verifier_test;
    use crate::e0_deterministic_sensor::real_signal_and_replay_for_frozen_summary_test;
    use crate::e0_frozen_summary::FrozenE0SummaryComposer;
    use crate::e0_frozen_verifier::FrozenE0IndependentVerifier;
    use crate::e0_opportunity_qualification::FrozenE0OpportunityQualifier;
    use crate::wiki_scratch::{InMemoryWikiGrantAuthority, MemoryScopeBinding, WikiGrant};

    fn seeded() -> (
        InMemoryArtifactRepository,
        InMemoryWikiGrantAuthority,
        WikiAccess,
        MemoryScope,
        VerifiedScoutCandidate,
        FrozenE0VerificationReport,
        FrozenE0OpportunityQualification,
        VerifiedReplayAvailability,
    ) {
        let candidate = verified_real_e0_candidate_for_frozen_verifier_test();
        let report = FrozenE0IndependentVerifier::verify(&candidate).unwrap();
        let qualification = FrozenE0OpportunityQualifier::qualify(&candidate, &report).unwrap();
        let (_signal, replay) = real_signal_and_replay_for_frozen_summary_test();
        let scope = MemoryScope::new(
            "tenant_a",
            "investigation",
            "world_a",
            "e0_diagnostic",
            "frozen",
            "train",
        );
        let mut repository = InMemoryArtifactRepository::default();
        let snapshot_ref = repository.append(None, ArtifactDraft::new(
            "tenant_a", "018f50a1-7f00-7000-8000-000000000033", 1, ArtifactKind::MemoryWiki,
            json!({"available_at_unix_seconds": 100, "purpose": "investigation", "pages": {"index.md": "seed"}}), None,
        )).unwrap().reference();
        let binding = MemoryScopeBinding::new("world_a", "e0_diagnostic", "frozen", "train");
        let access = WikiAccess::new_scoped(
            "run_e0",
            "tenant_a",
            "investigation",
            "grant_e0",
            snapshot_ref.clone(),
            100,
            binding.clone(),
        );
        let authority = InMemoryWikiGrantAuthority::default();
        authority.issue(WikiGrant::new_scoped(
            "grant_e0",
            "run_e0",
            "tenant_a",
            "investigation",
            snapshot_ref,
            binding,
        ));
        (
            repository,
            authority,
            access,
            scope,
            candidate,
            report,
            qualification,
            replay,
        )
    }

    #[test]
    fn real_frozen_e0_chain_can_publish_only_an_opaque_summary_capability() {
        let (
            mut repository,
            mut authority,
            access,
            scope,
            candidate,
            report,
            qualification,
            replay,
        ) = seeded();
        let prepared = FrozenE0SummaryComposer::prepare(
            &qualification,
            &candidate,
            &report,
            &replay,
            &scope,
            &access,
            &mut authority,
            &mut repository,
        )
        .unwrap();
        let mut port = InMemoryFrozenE0SummaryPublicationPort::default();
        let head = port
            .seed_head(&mut repository, scope.clone(), access.snapshot_ref.clone())
            .unwrap();
        let published = FrozenE0SummaryPublisher::publish(
            &prepared,
            &qualification,
            &candidate,
            &report,
            &replay,
            &scope,
            &access,
            &head,
            &authority,
            &mut port,
            &mut repository,
        )
        .unwrap();
        assert!(published.publication_commitment().starts_with("sha256:"));
        assert_eq!(published.memory_head_version(), 2);
        assert!(!published.authorizes_memory_use_or_promotion());
        assert_ne!(published.memory_snapshot_ref(), &access.snapshot_ref);
    }

    #[test]
    fn scope_and_access_drift_fail_before_a_publication_append() {
        let (
            mut repository,
            mut authority,
            mut access,
            scope,
            candidate,
            report,
            qualification,
            replay,
        ) = seeded();
        let prepared = FrozenE0SummaryComposer::prepare(
            &qualification,
            &candidate,
            &report,
            &replay,
            &scope,
            &access,
            &mut authority,
            &mut repository,
        )
        .unwrap();
        let mut port = InMemoryFrozenE0SummaryPublicationPort::default();
        let head = port
            .seed_head(&mut repository, scope.clone(), access.snapshot_ref.clone())
            .unwrap();
        access.allowed_at_unix_seconds = 99;
        assert!(matches!(
            FrozenE0SummaryPublisher::publish(
                &prepared,
                &qualification,
                &candidate,
                &report,
                &replay,
                &scope,
                &access,
                &head,
                &authority,
                &mut port,
                &mut repository
            ),
            Err(FrozenE0SummaryPublicationError::Preparation(
                FrozenE0SummaryPreparationError::AccessMismatch
            ))
        ));
    }

    #[test]
    fn provenance_drift_is_rejected_before_the_private_result_is_redeemed() {
        let (
            mut repository,
            mut authority,
            access,
            scope,
            mut candidate,
            report,
            qualification,
            replay,
        ) = seeded();
        let prepared = FrozenE0SummaryComposer::prepare(
            &qualification,
            &candidate,
            &report,
            &replay,
            &scope,
            &access,
            &mut authority,
            &mut repository,
        )
        .unwrap();
        let mut port = InMemoryFrozenE0SummaryPublicationPort::default();
        let head = port
            .seed_head(&mut repository, scope.clone(), access.snapshot_ref.clone())
            .unwrap();
        crate::autonomous_scout::corrupt_e0_metric_spec_for_frozen_verifier_test(&mut candidate);
        assert!(matches!(
            FrozenE0SummaryPublisher::publish(
                &prepared,
                &qualification,
                &candidate,
                &report,
                &replay,
                &scope,
                &access,
                &head,
                &authority,
                &mut port,
                &mut repository
            ),
            Err(FrozenE0SummaryPublicationError::Preparation(
                FrozenE0SummaryPreparationError::Candidate
            ))
        ));
    }

    #[test]
    fn exact_retry_is_idempotent_and_durable_adapter_fails_closed() {
        let (
            mut repository,
            mut authority,
            access,
            scope,
            candidate,
            report,
            qualification,
            replay,
        ) = seeded();
        let prepared = FrozenE0SummaryComposer::prepare(
            &qualification,
            &candidate,
            &report,
            &replay,
            &scope,
            &access,
            &mut authority,
            &mut repository,
        )
        .unwrap();
        let mut port = InMemoryFrozenE0SummaryPublicationPort::default();
        let head = port
            .seed_head(&mut repository, scope.clone(), access.snapshot_ref.clone())
            .unwrap();
        let first = FrozenE0SummaryPublisher::publish(
            &prepared,
            &qualification,
            &candidate,
            &report,
            &replay,
            &scope,
            &access,
            &head,
            &authority,
            &mut port,
            &mut repository,
        )
        .unwrap();
        let retry = FrozenE0SummaryPublisher::publish(
            &prepared,
            &qualification,
            &candidate,
            &report,
            &replay,
            &scope,
            &access,
            &head,
            &authority,
            &mut port,
            &mut repository,
        )
        .unwrap();
        assert_eq!(
            first.publication_commitment(),
            retry.publication_commitment()
        );
        assert_eq!(first.memory_snapshot_ref(), retry.memory_snapshot_ref());

        let mut unavailable = DurableFrozenE0SummaryPublicationUnavailable;
        assert!(matches!(
            FrozenE0SummaryPublisher::publish(
                &prepared,
                &qualification,
                &candidate,
                &report,
                &replay,
                &scope,
                &access,
                &head,
                &authority,
                &mut unavailable,
                &mut repository
            ),
            Err(FrozenE0SummaryPublicationError::DependencyUnavailable)
        ));
    }

    #[test]
    fn stale_expected_head_after_an_external_advance_creates_no_publication_sidecar() {
        let (
            mut repository,
            mut authority,
            access,
            scope,
            candidate,
            report,
            qualification,
            replay,
        ) = seeded();
        let prepared = FrozenE0SummaryComposer::prepare(
            &qualification,
            &candidate,
            &report,
            &replay,
            &scope,
            &access,
            &mut authority,
            &mut repository,
        )
        .unwrap();
        let mut port = InMemoryFrozenE0SummaryPublicationPort::default();
        let expected_head = port
            .seed_head(&mut repository, scope.clone(), access.snapshot_ref.clone())
            .unwrap();
        let advanced = port.advance_head_for_test(&mut repository, &scope).unwrap();
        assert_ne!(advanced.snapshot_ref, expected_head.snapshot_ref);
        assert_eq!(port.publication_count_for_test(), 0);

        assert!(matches!(
            FrozenE0SummaryPublisher::publish(
                &prepared,
                &qualification,
                &candidate,
                &report,
                &replay,
                &scope,
                &access,
                &expected_head,
                &authority,
                &mut port,
                &mut repository,
            ),
            Err(FrozenE0SummaryPublicationError::HeadConflict)
        ));
        assert_eq!(port.publication_count_for_test(), 0);
        assert_eq!(port.head_for_test(&scope), Some(advanced.clone()));
        assert!(
            repository
                .get("tenant_a", &advanced.snapshot_ref.id, 3)
                .unwrap()
                .is_none()
        );
    }
}
