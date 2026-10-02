//! U22 admission boundary for a second run using published memory.
//!
//! U33 remains the owner of the mutable head, tombstones, exact authorization
//! and idempotent use receipt. U22 narrows its successful result to opaque
//! provenance: it never returns wiki pages or a cache handle.

use crate::ArtifactRepository;
use crate::memory_store::{
    AtomicMemoryUseCommitPort, AtomicMemoryUseRequest, MemoryError, MemoryScope, MemoryUseReceipt,
};
use crate::wiki_scratch::{MemoryUseCommitAuthority, WikiAccess};

/// Caller-supplied context which U33 must validate before a memory use is
/// admitted. It contains no wiki payload or cache handle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryUseRequest {
    scope: MemoryScope,
    access: WikiAccess,
    temporal_commitment: Option<String>,
}

impl MemoryUseRequest {
    #[must_use]
    pub fn new(scope: MemoryScope, access: WikiAccess) -> Self {
        Self {
            scope,
            access,
            temporal_commitment: None,
        }
    }

    #[must_use]
    pub(crate) fn scope(&self) -> &MemoryScope {
        &self.scope
    }

    #[must_use]
    pub(crate) fn allowed_at_unix_seconds(&self) -> u64 {
        self.access.allowed_at_unix_seconds
    }

    /// Only the U23 trusted temporal evidence boundary may bind a receipt to
    /// a temporal claim. The public U22 request deliberately cannot set this.
    pub(crate) fn with_temporal_commitment(mut self, commitment: String) -> Self {
        self.temporal_commitment = Some(commitment);
        self
    }

    pub(crate) fn temporal_commitment(&self) -> Option<&str> {
        self.temporal_commitment.as_deref()
    }

    pub(crate) fn access(&self) -> &WikiAccess {
        &self.access
    }
}

/// Opaque provenance capability for one already-authorized use in a later run.
///
/// ```compile_fail
/// use improvement_engine_core::governed_memory_use::VerifiedMemoryUse;
/// let _ = VerifiedMemoryUse { receipt: todo!() };
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::governed_memory_use::MemoryUseAdmission;
/// let _ = MemoryUseAdmission::admit;
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::governed_memory_use::MemoryUseAdmission;
/// let _ = MemoryUseAdmission { _private: true };
/// ```
pub struct VerifiedMemoryUse {
    receipt: MemoryUseReceipt,
}

impl VerifiedMemoryUse {
    pub(crate) fn receipt(&self) -> &MemoryUseReceipt {
        &self.receipt
    }
    #[must_use]
    pub fn receipt_id(&self) -> &str {
        &self.receipt.receipt_id
    }

    #[must_use]
    pub fn scope(&self) -> &MemoryScope {
        &self.receipt.scope
    }

    #[must_use]
    pub fn snapshot_ref(&self) -> &crate::ArtifactReference {
        &self.receipt.snapshot_ref
    }

    #[must_use]
    pub fn head_version(&self) -> u64 {
        self.receipt.head_version
    }

    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.receipt.run_id
    }

    #[must_use]
    pub fn grant_id(&self) -> &str {
        &self.receipt.grant_id
    }

    #[must_use]
    pub fn purpose(&self) -> &str {
        &self.receipt.purpose
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryUseAdmissionError {
    Denied(MemoryError),
    ReceiptMismatch,
}

/// Trusted composition entry point. It is crate-private so an external caller
/// cannot substitute a permissive `MemoryPublisher` implementation and mint a
/// use capability. A future service composition root supplies U33's durable
/// adapter and U05 authorization boundary here.
pub struct MemoryUseAdmission {
    _private: bool,
}

impl MemoryUseAdmission {
    #[allow(dead_code)] // Invoked by the future trusted service composition root.
    pub(crate) fn admit<
        R: ArtifactRepository,
        P: AtomicMemoryUseCommitPort,
        A: MemoryUseCommitAuthority,
    >(
        publisher: &mut P,
        artifacts: &mut R,
        authority: &A,
        request: MemoryUseRequest,
    ) -> Result<VerifiedMemoryUse, MemoryUseAdmissionError> {
        let receipt = publisher
            .commit_allowed_use(
                artifacts,
                authority,
                AtomicMemoryUseRequest::new(
                    request.scope.clone(),
                    request.access.clone(),
                    request.temporal_commitment.clone(),
                    request.access.snapshot_ref.clone(),
                ),
            )
            .map_err(MemoryUseAdmissionError::Denied)?;
        // `commit_allowed_use` is the U33 conditional-commit port: a durable
        // adapter checks head/liveness/authority and inserts the exact receipt
        // in one transaction. Re-reading and attesting afterwards would create
        // a revocation/head race after a visible allowed receipt.
        if !receipt_matches_request(&receipt, &request) {
            return Err(MemoryUseAdmissionError::ReceiptMismatch);
        }
        Ok(VerifiedMemoryUse { receipt })
    }
}

#[allow(dead_code)] // Used together with the crate-private composition method above.
fn receipt_matches_request(receipt: &MemoryUseReceipt, request: &MemoryUseRequest) -> bool {
    receipt.head_version > 0
        && receipt.scope == request.scope
        && receipt.snapshot_ref == request.access.snapshot_ref
        && receipt.run_id == request.access.run_id
        && receipt.grant_id == request.access.grant_id
        && receipt.grant_revision == request.access.grant_revision
        && receipt.purpose == request.access.purpose
        && receipt.allowed_at_unix_seconds == request.access.allowed_at_unix_seconds
        && receipt.temporal_commitment.as_deref() == request.temporal_commitment()
}

#[cfg(test)]
mod tests {
    use super::{MemoryUseAdmission, MemoryUseAdmissionError, MemoryUseRequest};
    use crate::memory_store::{
        InMemoryMemoryRegistry, MemoryError, MemoryPublisher, MemoryScope,
        MemoryUseCommitInterleaving, MemoryUseReceiptAttestationPort,
    };
    use crate::wiki_scratch::{
        InMemoryWikiGrantAuthority, MemoryScopeBinding, MemoryUseGrantInterleaving, WikiAccess,
        WikiGrant,
    };
    use crate::{ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository};
    use serde_json::json;

    const TENANT: &str = "tenant-a";
    const WIKI_ID: &str = "018f50a1-7f00-7000-8000-000000000022";

    fn scope() -> MemoryScope {
        MemoryScope::new(
            TENANT,
            "investigation",
            "world-a",
            "campaign-a",
            "continuous",
            "train",
        )
    }

    fn seeded() -> (
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
            .expect("fixed published memory")
            .reference();
        let access = WikiAccess::new_scoped(
            "run-2",
            TENANT,
            "investigation",
            "grant-2",
            snapshot.clone(),
            100,
            MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train"),
        );
        let authority = InMemoryWikiGrantAuthority::default();
        authority.issue(WikiGrant::new_scoped(
            "grant-2",
            "run-2",
            TENANT,
            "investigation",
            snapshot.clone(),
            MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train"),
        ));
        let mut registry = InMemoryMemoryRegistry::default();
        registry
            .seed_head(&mut artifacts, scope(), snapshot)
            .expect("fixed memory head");
        (artifacts, registry, authority, access)
    }

    #[test]
    fn admits_a_second_run_only_through_u33_and_replays_the_same_receipt() {
        let (mut artifacts, mut registry, authority, access) = seeded();
        let request = MemoryUseRequest::new(scope(), access.clone());

        let admitted =
            MemoryUseAdmission::admit(&mut registry, &mut artifacts, &authority, request)
                .expect("U33-authorized second-run use");
        let replay = MemoryUseAdmission::admit(
            &mut registry,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope(), access),
        )
        .expect("same U33 receipt on replay");

        assert_eq!(admitted.run_id(), "run-2");
        assert_eq!(admitted.grant_id(), "grant-2");
        assert_eq!(admitted.purpose(), "investigation");
        assert_eq!(admitted.head_version(), 1);
        assert_eq!(admitted.receipt_id(), replay.receipt_id());
        assert_eq!(registry.receipts().len(), 1);
    }

    #[test]
    fn revoked_or_cross_scope_memory_never_mints_a_second_run_capability() {
        let (mut artifacts, mut registry, authority, access) = seeded();
        let snapshot = access.snapshot_ref.clone();
        registry
            .revoke(snapshot, "permission_revoked")
            .expect("fixed revocation");
        match MemoryUseAdmission::admit(
            &mut registry,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope(), access),
        ) {
            Err(error) => assert_eq!(
                error,
                MemoryUseAdmissionError::Denied(MemoryError::SnapshotRevoked)
            ),
            Ok(_) => panic!("revoked memory must not mint a capability"),
        }

        let (mut artifacts, mut registry, authority, access) = seeded();
        let other_scope = MemoryScope::new(
            TENANT,
            "investigation",
            "world-b",
            "campaign-a",
            "continuous",
            "train",
        );
        match MemoryUseAdmission::admit(
            &mut registry,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(other_scope, access),
        ) {
            Err(error) => assert_eq!(
                error,
                MemoryUseAdmissionError::Denied(MemoryError::HeadMissing)
            ),
            Ok(_) => panic!("cross-scope memory must not mint a capability"),
        }

        let (mut artifacts, mut registry, authority, access) = seeded();
        let cross_tenant_access = WikiAccess::new_scoped(
            "run-2",
            "tenant-b",
            "investigation",
            "grant-2",
            access.snapshot_ref,
            100,
            MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train"),
        );
        match MemoryUseAdmission::admit(
            &mut registry,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope(), cross_tenant_access),
        ) {
            Err(error) => assert_eq!(
                error,
                MemoryUseAdmissionError::Denied(MemoryError::AccessDenied)
            ),
            Ok(_) => panic!("cross-tenant memory must not mint a capability"),
        }
    }

    #[test]
    fn revocation_that_wins_inside_the_atomic_u33_predicate_emits_no_receipt_or_capability() {
        let (mut artifacts, mut registry, authority, access) = seeded();
        registry.schedule_atomic_commit_interleaving(MemoryUseCommitInterleaving::RevokeSnapshot);

        match MemoryUseAdmission::admit(
            &mut registry,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope(), access),
        ) {
            Err(MemoryUseAdmissionError::Denied(MemoryError::SnapshotRevoked)) => {}
            Err(other) => panic!("expected atomic revocation denial, got {other:?}"),
            Ok(_) => panic!("atomic revocation must not emit a capability"),
        }
        assert!(
            registry.receipts().is_empty(),
            "a failed conditional admission must not leave a receipt behind"
        );
    }

    #[test]
    fn head_change_that_wins_inside_the_atomic_u33_predicate_emits_no_receipt_or_capability() {
        let (mut artifacts, mut registry, authority, access) = seeded();
        registry.schedule_atomic_commit_interleaving(MemoryUseCommitInterleaving::AdvanceHead);

        match MemoryUseAdmission::admit(
            &mut registry,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope(), access),
        ) {
            Err(MemoryUseAdmissionError::Denied(MemoryError::HeadConflict { .. })) => {}
            Err(other) => panic!("expected atomic head conflict, got {other:?}"),
            Ok(_) => panic!("stale atomic head must not mint a capability"),
        }
        assert!(registry.receipts().is_empty());
    }

    #[test]
    fn replaced_or_revoked_grant_revision_cannot_pass_the_u33_fence_or_leave_a_receipt() {
        let (mut artifacts, mut registry, authority, access) = seeded();
        let binding = MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train");
        authority.issue(
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

        assert!(matches!(
            MemoryUseAdmission::admit(
                &mut registry,
                &mut artifacts,
                &authority,
                MemoryUseRequest::new(scope(), access.clone()),
            ),
            Err(MemoryUseAdmissionError::Denied(MemoryError::AccessDenied))
        ));
        assert!(registry.receipts().is_empty());

        assert!(authority.revoke("grant-2"));
        assert!(matches!(
            MemoryUseAdmission::admit(
                &mut registry,
                &mut artifacts,
                &authority,
                MemoryUseRequest::new(scope(), access),
            ),
            Err(MemoryUseAdmissionError::Denied(MemoryError::AccessDenied))
        ));
        assert!(registry.receipts().is_empty());
    }

    #[test]
    fn grant_revoke_or_replacement_that_wins_inside_u33_predicate_leaves_no_receipt() {
        for interleaving in [
            MemoryUseGrantInterleaving::Revoke,
            MemoryUseGrantInterleaving::ReplaceWithRevision(2),
        ] {
            let (mut artifacts, mut registry, authority, access) = seeded();
            authority.schedule_memory_use_interleaving(interleaving);

            assert!(matches!(
                MemoryUseAdmission::admit(
                    &mut registry,
                    &mut artifacts,
                    &authority,
                    MemoryUseRequest::new(scope(), access),
                ),
                Err(MemoryUseAdmissionError::Denied(MemoryError::AccessDenied))
            ));
            assert!(
                registry.receipts().is_empty(),
                "{interleaving:?} must fail the final U33 predicate before receipt insertion"
            );
        }
    }

    #[test]
    fn u33_attestation_rejects_a_forged_receipt_id_or_positive_wrong_head() {
        let (mut artifacts, mut registry, authority, access) = seeded();
        let receipt = registry
            .record_allowed_use(
                &mut artifacts,
                &authority,
                scope(),
                access.clone(),
                None,
                access.snapshot_ref.clone(),
            )
            .expect("fixed U33 receipt");
        let mut wrong_id = receipt.clone();
        wrong_id.receipt_id = "forged-receipt-id".to_owned();
        assert_eq!(
            registry.attest_allowed_use(
                &mut artifacts,
                &authority,
                &scope(),
                &access,
                None,
                &wrong_id
            ),
            Err(MemoryError::ReceiptConflict)
        );
        let mut wrong_head = receipt;
        wrong_head.head_version = 2;
        assert_eq!(
            registry.attest_allowed_use(
                &mut artifacts,
                &authority,
                &scope(),
                &access,
                None,
                &wrong_head
            ),
            Err(MemoryError::ReceiptConflict)
        );
    }
}
