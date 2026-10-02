//! U15-EQ: scratch-only preparation of one Frozen E0 memory summary.
//!
//! This is not publication, a memory head update, a U23 use, a proposal or a
//! causal/value claim. It recomputes the sealed U13→U14→U14-EQ chain, checks
//! the exact U04-B replay boundary, and performs one static, allowlisted U15
//! transform inside an ephemeral workspace. U33-E/U23-E own every later
//! publication and governed-memory linkage.

#![allow(dead_code)] // Wired by the future trusted service-composition root.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ArtifactRepository;
use crate::autonomous_scout::{FrozenE0ScoutCandidateError, VerifiedScoutCandidate};
use crate::e0_frozen_verifier::{FrozenE0VerificationError, FrozenE0VerificationReport};
use crate::e0_opportunity_qualification::{
    FrozenE0OpportunityQualification, FrozenE0QualificationError,
};
use crate::enriched_history::VerifiedReplayAvailability;
use crate::memory_store::MemoryScope;
use crate::wiki_scratch::{
    MemoryScopeBinding, WikiAccess, WikiError, WikiScratchPort, WikiTransform,
    WikiTransformOperation,
};

const PURPOSE: &str = "investigation";
const CAMPAIGN: &str = "e0_diagnostic";
const PROTOCOL: &str = "frozen";
const PARTITION: &str = "train";
const SUMMARY_PATH: &str = "prepared/frozen-e0-summary.md";
// This text is intentionally static: no candidate facts, user text, outcomes,
// values, causal claims or source/wiki content can enter a U15-EQ transform.
const SUMMARY_CONTENT: &str =
    "# Frozen E0 preparation\n\nPrepared provenance for later governed evaluation.\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparedFrozenE0MemorySummaryStatus {
    PreparedScratchOnly,
}

/// Opaque result of a single scratch transform. It intentionally exposes only
/// commitments and receipt-derived digests: never pages, workspace identity,
/// paths, source bytes, summary text or a publication/use capability.
pub struct PreparedFrozenE0MemorySummary {
    candidate_digest: String,
    qualification_commitment: String,
    verification_report_commitment: String,
    replay_commitment: String,
    access_commitment: String,
    transform_commitment: String,
    result_commitment: String,
    preparation_commitment: String,
    status: PreparedFrozenE0MemorySummaryStatus,
}

impl PreparedFrozenE0MemorySummary {
    #[must_use]
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }
    #[must_use]
    pub fn qualification_commitment(&self) -> &str {
        &self.qualification_commitment
    }
    #[must_use]
    pub fn verification_report_commitment(&self) -> &str {
        &self.verification_report_commitment
    }
    #[must_use]
    pub fn replay_commitment(&self) -> &str {
        &self.replay_commitment
    }
    #[must_use]
    pub fn access_commitment(&self) -> &str {
        &self.access_commitment
    }
    #[must_use]
    pub fn transform_commitment(&self) -> &str {
        &self.transform_commitment
    }
    #[must_use]
    pub fn result_commitment(&self) -> &str {
        &self.result_commitment
    }
    #[must_use]
    pub fn preparation_commitment(&self) -> &str {
        &self.preparation_commitment
    }
    #[must_use]
    pub fn status(&self) -> PreparedFrozenE0MemorySummaryStatus {
        self.status
    }
    #[must_use]
    pub fn authorizes_publication_or_memory_use(&self) -> bool {
        false
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum FrozenE0SummaryPreparationError {
    Candidate,
    Verification(FrozenE0VerificationError),
    Qualification(FrozenE0QualificationError),
    ReplayMismatch,
    ScopeMismatch,
    AccessMismatch,
    Scratch(WikiError),
    TransformReceiptMismatch,
}

impl From<FrozenE0ScoutCandidateError> for FrozenE0SummaryPreparationError {
    fn from(_: FrozenE0ScoutCandidateError) -> Self {
        Self::Candidate
    }
}
impl From<FrozenE0VerificationError> for FrozenE0SummaryPreparationError {
    fn from(value: FrozenE0VerificationError) -> Self {
        Self::Verification(value)
    }
}
impl From<FrozenE0QualificationError> for FrozenE0SummaryPreparationError {
    fn from(value: FrozenE0QualificationError) -> Self {
        Self::Qualification(value)
    }
}
impl From<WikiError> for FrozenE0SummaryPreparationError {
    fn from(value: WikiError) -> Self {
        Self::Scratch(value)
    }
}

/// Composition-only U15-EQ boundary. It is crate-private specifically so a
/// transport caller cannot select a permissive wiki port, scope, grant, clock
/// or replay projection. The enclosing service composition must obtain each
/// input from its owning sealed boundary.
pub(crate) struct FrozenE0SummaryComposer;

impl FrozenE0SummaryComposer {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare<R: ArtifactRepository, W: WikiScratchPort>(
        qualification: &FrozenE0OpportunityQualification,
        candidate: &VerifiedScoutCandidate,
        report: &FrozenE0VerificationReport,
        replay: &VerifiedReplayAvailability,
        scope: &MemoryScope,
        access: &WikiAccess,
        scratch: &mut W,
        repository: &mut R,
    ) -> Result<PreparedFrozenE0MemorySummary, FrozenE0SummaryPreparationError> {
        let frozen = candidate.rehydrate_frozen_e0()?;
        report.revalidate_for_candidate(candidate)?;
        qualification.revalidate_for_inputs(candidate, report)?;

        if report.scope() != candidate.scope()
            || qualification.scope() != candidate.scope()
            || report.source_snapshot_ref() != candidate.source_snapshot_ref()
            || qualification.source_snapshot_ref() != candidate.source_snapshot_ref()
            || replay.tenant_id() != candidate.scope().tenant_id()
            || replay.cutoff_at_unix_seconds() != frozen.cutoff_unix_seconds()
            || replay.source_snapshot_digest() != frozen.source_snapshot_binding()
            || replay.availability_profile_digest() != frozen.availability_profile_digest()
        {
            return Err(FrozenE0SummaryPreparationError::ReplayMismatch);
        }
        let expected_binding =
            MemoryScopeBinding::new(replay.world_ref(), CAMPAIGN, PROTOCOL, PARTITION);
        if scope.tenant_id != candidate.scope().tenant_id()
            || scope.purpose != PURPOSE
            || scope.world != replay.world_ref()
            || scope.campaign != CAMPAIGN
            || scope.protocol != PROTOCOL
            || scope.partition != PARTITION
            || access.tenant_id != scope.tenant_id
            || access.purpose != scope.purpose
            || access.memory_scope != expected_binding
        {
            return Err(FrozenE0SummaryPreparationError::ScopeMismatch);
        }
        if access.allowed_at_unix_seconds != replay.cutoff_at_unix_seconds()
            || access.run_id != frozen.run_id()
            || access.grant_id != candidate.scope().grant_id()
            || access.snapshot_ref.tenant_id != candidate.scope().tenant_id()
        {
            return Err(FrozenE0SummaryPreparationError::AccessMismatch);
        }

        let replay_commitment = digest(&ReplayCommitment {
            tenant_id: replay.tenant_id(),
            world_ref: replay.world_ref(),
            cutoff_at_unix_seconds: replay.cutoff_at_unix_seconds(),
            source_snapshot_digest: replay.source_snapshot_digest(),
            availability_profile_digest: replay.availability_profile_digest(),
        });
        let access_commitment = digest(&AccessCommitment {
            tenant_id: &access.tenant_id,
            run_id: &access.run_id,
            grant_id: &access.grant_id,
            grant_revision: access.grant_revision,
            purpose: &access.purpose,
            snapshot_ref: &access.snapshot_ref,
            allowed_at_unix_seconds: access.allowed_at_unix_seconds,
            scope,
        });
        let transform = canonical_transform();
        let expected_transform_commitment = digest(&transform);
        let mut workspace = scratch.mount(repository, access.clone())?;
        let result = scratch.transform(&mut workspace, access, transform)?;
        if result.receipt.snapshot_ref != access.snapshot_ref
            || result.receipt.run_id != access.run_id
            || result.receipt.tenant_id != access.tenant_id
            || result.receipt.purpose != access.purpose
            || result.receipt.grant_id != access.grant_id
            || result.receipt.memory_scope != access.memory_scope
            || result.receipt.transform_digest != expected_transform_commitment
            || result.receipt.result_digest != result.result_digest
        {
            return Err(FrozenE0SummaryPreparationError::TransformReceiptMismatch);
        }
        let preparation_commitment = digest(&PreparationCommitment {
            candidate_digest: candidate.candidate_digest(),
            qualification_commitment: qualification.commitment(),
            verification_report_commitment: report.report_commitment(),
            replay_commitment: &replay_commitment,
            access_commitment: &access_commitment,
            transform_commitment: &result.receipt.transform_digest,
            result_commitment: &result.receipt.result_digest,
            status: PreparedFrozenE0MemorySummaryStatus::PreparedScratchOnly,
        });
        Ok(PreparedFrozenE0MemorySummary {
            candidate_digest: candidate.candidate_digest().to_owned(),
            qualification_commitment: qualification.commitment().to_owned(),
            verification_report_commitment: report.report_commitment().to_owned(),
            replay_commitment,
            access_commitment,
            transform_commitment: result.receipt.transform_digest,
            result_commitment: result.receipt.result_digest,
            preparation_commitment,
            status: PreparedFrozenE0MemorySummaryStatus::PreparedScratchOnly,
        })
    }
}

fn canonical_transform() -> WikiTransform {
    WikiTransform::new(vec![WikiTransformOperation::create(
        SUMMARY_PATH,
        SUMMARY_CONTENT,
    )])
}

#[derive(Serialize)]
struct ReplayCommitment<'a> {
    tenant_id: &'a str,
    world_ref: &'a str,
    cutoff_at_unix_seconds: u64,
    source_snapshot_digest: &'a str,
    availability_profile_digest: &'a str,
}
#[derive(Serialize)]
struct AccessCommitment<'a> {
    tenant_id: &'a str,
    run_id: &'a str,
    grant_id: &'a str,
    grant_revision: u64,
    purpose: &'a str,
    snapshot_ref: &'a crate::ArtifactReference,
    allowed_at_unix_seconds: u64,
    scope: &'a MemoryScope,
}
#[derive(Serialize)]
struct PreparationCommitment<'a> {
    candidate_digest: &'a str,
    qualification_commitment: &'a str,
    verification_report_commitment: &'a str,
    replay_commitment: &'a str,
    access_commitment: &'a str,
    transform_commitment: &'a str,
    result_commitment: &'a str,
    status: PreparedFrozenE0MemorySummaryStatus,
}
fn digest<T: Serialize>(value: &T) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("U15-EQ commitments serialize"))
    )
}

/// ```compile_fail
/// use improvement_engine_core::e0_frozen_summary::{FrozenE0SummaryComposer, PreparedFrozenE0MemorySummary};
/// let _ = FrozenE0SummaryComposer::prepare;
/// let _ = PreparedFrozenE0MemorySummary {};
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::e0_frozen_summary::PreparedFrozenE0MemorySummary;
/// fn leak(summary: PreparedFrozenE0MemorySummary) -> String { format!("{summary:?}") }
/// ```
const _SUMMARY_COMPOSITION_AND_CONTENT_ARE_NOT_PUBLIC: () = ();

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use crate::ArtifactRepository;
    use crate::autonomous_scout::verified_real_e0_candidate_for_frozen_verifier_test;
    use crate::e0_deterministic_sensor::real_signal_and_replay_for_frozen_summary_test;
    use crate::e0_frozen_verifier::FrozenE0IndependentVerifier;
    use crate::e0_opportunity_qualification::FrozenE0OpportunityQualifier;
    use crate::wiki_scratch::{InMemoryWikiGrantAuthority, WikiGrant};
    use crate::{ArtifactDraft, ArtifactKind, InMemoryArtifactRepository};
    use serde_json::json;

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
            "tenant_a", PURPOSE, "world_a", CAMPAIGN, PROTOCOL, PARTITION,
        );
        let mut repository = InMemoryArtifactRepository::default();
        let snapshot_ref = repository.append(None, ArtifactDraft::new(
            "tenant_a", "018f50a1-7f00-7000-8000-000000000015", 1, ArtifactKind::MemoryWiki,
            json!({"available_at_unix_seconds": 100, "purpose": PURPOSE, "pages": {"index.md": "seed"}}), None,
        )).unwrap().reference();
        let binding = MemoryScopeBinding::new("world_a", CAMPAIGN, PROTOCOL, PARTITION);
        let access = WikiAccess::new_scoped(
            "run_e0",
            "tenant_a",
            PURPOSE,
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
            PURPOSE,
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

    /// A denied precondition must not mount or transform a scratch workspace.
    struct NoTouchScratch;
    impl WikiScratchPort for NoTouchScratch {
        fn mount<R: ArtifactRepository>(
            &mut self,
            _: &mut R,
            _: WikiAccess,
        ) -> Result<crate::wiki_scratch::WikiWorkspace, WikiError> {
            panic!("invalid U15-EQ input must not mount scratch")
        }
        fn read(
            &self,
            _: &crate::wiki_scratch::WikiWorkspace,
            _: &WikiAccess,
            _: &str,
        ) -> Result<crate::wiki_scratch::WikiReadResult, WikiError> {
            panic!("U15-EQ never reads source/wiki content")
        }
        fn transform(
            &self,
            _: &mut crate::wiki_scratch::WikiWorkspace,
            _: &WikiAccess,
            _: WikiTransform,
        ) -> Result<crate::wiki_scratch::WikiTransformResult, WikiError> {
            panic!("invalid U15-EQ input must not transform scratch")
        }
    }

    #[test]
    fn real_e2e_chain_prepares_one_static_scratch_summary_without_publication() {
        let (mut repository, mut scratch, access, scope, candidate, report, qualification, replay) =
            seeded();
        let prepared = FrozenE0SummaryComposer::prepare(
            &qualification,
            &candidate,
            &report,
            &replay,
            &scope,
            &access,
            &mut scratch,
            &mut repository,
        )
        .unwrap();
        assert_eq!(
            prepared.status(),
            PreparedFrozenE0MemorySummaryStatus::PreparedScratchOnly
        );
        assert!(!prepared.authorizes_publication_or_memory_use());
        assert_eq!(prepared.candidate_digest(), candidate.candidate_digest());
        assert_eq!(
            prepared.qualification_commitment(),
            qualification.commitment()
        );
        assert_eq!(
            prepared.verification_report_commitment(),
            report.report_commitment()
        );
        assert!(prepared.transform_commitment().starts_with("sha256:"));
        assert!(prepared.result_commitment().starts_with("sha256:"));
        assert!(!prepared.preparation_commitment().contains(SUMMARY_CONTENT));
    }

    #[test]
    fn replay_scope_and_access_drift_fail_before_any_scratch_transform() {
        let (mut repository, _scratch, mut access, scope, candidate, report, qualification, replay) =
            seeded();
        let mut scratch = NoTouchScratch;
        access.allowed_at_unix_seconds = 99;
        assert!(matches!(
            FrozenE0SummaryComposer::prepare(
                &qualification,
                &candidate,
                &report,
                &replay,
                &scope,
                &access,
                &mut scratch,
                &mut repository
            ),
            Err(FrozenE0SummaryPreparationError::AccessMismatch)
        ));

        let (mut repository, _scratch, access, mut scope, candidate, report, qualification, replay) =
            seeded();
        let mut scratch = NoTouchScratch;
        scope.protocol = "continuous".to_owned();
        assert!(matches!(
            FrozenE0SummaryComposer::prepare(
                &qualification,
                &candidate,
                &report,
                &replay,
                &scope,
                &access,
                &mut scratch,
                &mut repository
            ),
            Err(FrozenE0SummaryPreparationError::ScopeMismatch)
        ));
    }
}
