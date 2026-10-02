//! U22 admission boundary for a second run using published memory.
//!
//! U33 remains the owner of the mutable head, tombstones, exact authorization
//! and idempotent use receipt. U22 narrows its successful result to opaque
//! provenance: it never returns wiki pages or a cache handle.

use crate::ArtifactRepository;
use crate::memory_store::{
    MemoryError, MemoryPublisher, MemoryScope, MemoryUseReceipt, MemoryUseReceiptAttestationPort,
};
use crate::wiki_scratch::{WikiAccess, WikiAuthorizationPort};

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
        P: MemoryPublisher + MemoryUseReceiptAttestationPort,
        A: WikiAuthorizationPort,
    >(
        publisher: &mut P,
        artifacts: &mut R,
        authority: &A,
        request: MemoryUseRequest,
    ) -> Result<VerifiedMemoryUse, MemoryUseAdmissionError> {
        let receipt = publisher
            .record_allowed_use(
                artifacts,
                authority,
                request.scope.clone(),
                request.access.clone(),
                request.temporal_commitment.clone(),
                request.access.snapshot_ref.clone(),
            )
            .map_err(MemoryUseAdmissionError::Denied)?;
        publisher
            .attest_allowed_use(
                artifacts,
                authority,
                &request.scope,
                &request.access,
                request.temporal_commitment(),
                &receipt,
            )
            .map_err(MemoryUseAdmissionError::Denied)?;
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
        && receipt.purpose == request.access.purpose
        && receipt.allowed_at_unix_seconds == request.access.allowed_at_unix_seconds
        && receipt.temporal_commitment.as_deref() == request.temporal_commitment()
}

#[cfg(test)]
mod tests {
    use super::{MemoryUseAdmission, MemoryUseAdmissionError, MemoryUseRequest};
    use crate::memory_store::{
        InMemoryMemoryRegistry, MemoryError, MemoryHead, MemoryPublishRequest, MemoryPublisher,
        MemoryScope, MemoryUseReceipt, MemoryUseReceiptAttestationPort, PublishedMemory,
    };
    use crate::wiki_scratch::{
        InMemoryWikiGrantAuthority, MemoryScopeBinding, WikiAccess, WikiGrant,
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
        let mut authority = InMemoryWikiGrantAuthority::default();
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

    struct LyingPublisher;

    impl MemoryPublisher for LyingPublisher {
        fn seed_head<R: ArtifactRepository>(
            &mut self,
            _: &mut R,
            _: MemoryScope,
            _: crate::ArtifactReference,
        ) -> Result<MemoryHead, MemoryError> {
            unreachable!("not part of this admission regression")
        }

        fn publish<R: ArtifactRepository, A: crate::wiki_scratch::WikiAuthorizationPort>(
            &mut self,
            _: &mut R,
            _: &A,
            _: MemoryPublishRequest,
        ) -> Result<PublishedMemory, MemoryError> {
            unreachable!("not part of this admission regression")
        }

        fn revoke(&mut self, _: crate::ArtifactReference, _: &str) -> Result<(), MemoryError> {
            unreachable!("not part of this admission regression")
        }

        fn record_allowed_use<
            R: ArtifactRepository,
            A: crate::wiki_scratch::WikiAuthorizationPort,
        >(
            &mut self,
            _: &mut R,
            _: &A,
            scope: MemoryScope,
            access: WikiAccess,
            _: Option<String>,
            snapshot_ref: crate::ArtifactReference,
        ) -> Result<MemoryUseReceipt, MemoryError> {
            Ok(MemoryUseReceipt {
                receipt_id: "forged-receipt".to_owned(),
                scope,
                snapshot_ref,
                head_version: 1,
                run_id: "another-run".to_owned(),
                grant_id: access.grant_id,
                purpose: access.purpose,
                allowed_at_unix_seconds: access.allowed_at_unix_seconds,
                temporal_commitment: None,
            })
        }
    }

    impl MemoryUseReceiptAttestationPort for LyingPublisher {
        fn attest_allowed_use<
            R: ArtifactRepository,
            A: crate::wiki_scratch::WikiAuthorizationPort,
        >(
            &mut self,
            _: &mut R,
            _: &A,
            _: &MemoryScope,
            _: &WikiAccess,
            _: Option<&str>,
            _: &MemoryUseReceipt,
        ) -> Result<(), MemoryError> {
            Ok(())
        }
    }

    struct RevokingAfterRecord {
        inner: InMemoryMemoryRegistry,
    }

    impl MemoryPublisher for RevokingAfterRecord {
        fn seed_head<R: ArtifactRepository>(
            &mut self,
            artifacts: &mut R,
            scope: MemoryScope,
            snapshot_ref: crate::ArtifactReference,
        ) -> Result<MemoryHead, MemoryError> {
            self.inner.seed_head(artifacts, scope, snapshot_ref)
        }

        fn publish<R: ArtifactRepository, A: crate::wiki_scratch::WikiAuthorizationPort>(
            &mut self,
            artifacts: &mut R,
            authority: &A,
            request: MemoryPublishRequest,
        ) -> Result<PublishedMemory, MemoryError> {
            self.inner.publish(artifacts, authority, request)
        }

        fn revoke(
            &mut self,
            snapshot_ref: crate::ArtifactReference,
            reason: &str,
        ) -> Result<(), MemoryError> {
            self.inner.revoke(snapshot_ref, reason)
        }

        fn record_allowed_use<
            R: ArtifactRepository,
            A: crate::wiki_scratch::WikiAuthorizationPort,
        >(
            &mut self,
            artifacts: &mut R,
            authority: &A,
            scope: MemoryScope,
            access: WikiAccess,
            temporal_commitment: Option<String>,
            snapshot_ref: crate::ArtifactReference,
        ) -> Result<MemoryUseReceipt, MemoryError> {
            let receipt = self.inner.record_allowed_use(
                artifacts,
                authority,
                scope,
                access,
                temporal_commitment,
                snapshot_ref,
            )?;
            self.inner
                .revoke(receipt.snapshot_ref.clone(), "deterministic_interleaving")?;
            Ok(receipt)
        }
    }

    impl MemoryUseReceiptAttestationPort for RevokingAfterRecord {
        fn attest_allowed_use<
            R: ArtifactRepository,
            A: crate::wiki_scratch::WikiAuthorizationPort,
        >(
            &mut self,
            artifacts: &mut R,
            authority: &A,
            scope: &MemoryScope,
            access: &WikiAccess,
            temporal_commitment: Option<&str>,
            receipt: &MemoryUseReceipt,
        ) -> Result<(), MemoryError> {
            self.inner.attest_allowed_use(
                artifacts,
                authority,
                scope,
                access,
                temporal_commitment,
                receipt,
            )
        }
    }

    #[test]
    fn revocation_between_u33_record_and_attestation_emits_no_capability() {
        let (mut artifacts, registry, authority, access) = seeded();
        let mut publisher = RevokingAfterRecord { inner: registry };

        match MemoryUseAdmission::admit(
            &mut publisher,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope(), access),
        ) {
            Err(MemoryUseAdmissionError::Denied(MemoryError::SnapshotRevoked)) => {}
            Err(other) => panic!("expected post-record revocation denial, got {other:?}"),
            Ok(_) => panic!("post-record revocation must not emit a capability"),
        }
        assert_eq!(publisher.inner.receipts().len(), 1);
    }

    #[test]
    fn a_mismatched_receipt_never_becomes_a_capability_even_inside_trusted_composition() {
        let (mut artifacts, _, authority, access) = seeded();
        let mut publisher = LyingPublisher;

        match MemoryUseAdmission::admit(
            &mut publisher,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope(), access),
        ) {
            Err(MemoryUseAdmissionError::ReceiptMismatch) => {}
            Err(other) => panic!("expected receipt mismatch, got {other:?}"),
            Ok(_) => panic!("mismatched receipt must not mint a capability"),
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
