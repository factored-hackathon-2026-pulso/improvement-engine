//! Durable publication boundary for the investigator wiki.
//!
//! Scratch work stays inside U15. This module turns one verified scratch result
//! into the next immutable `memory_wiki` revision, guarded by both the U02
//! artifact CAS and a scope-specific memory head. Revocation is an overlay: it
//! never edits or deletes historical wiki bytes.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::wiki_scratch::{
    MemoryScopeBinding, MemoryUseCommitAuthority, WikiAccess, WikiAuthorizationPort,
    WikiTransformResult,
};
use crate::{ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository, RepositoryError};

/// The one information partition in which a memory head may be reused.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct MemoryScope {
    pub tenant_id: String,
    pub purpose: String,
    pub world: String,
    pub campaign: String,
    pub protocol: String,
    pub partition: String,
}

impl MemoryScope {
    #[must_use]
    pub fn new(
        tenant_id: impl Into<String>,
        purpose: impl Into<String>,
        world: impl Into<String>,
        campaign: impl Into<String>,
        protocol: impl Into<String>,
        partition: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            purpose: purpose.into(),
            world: world.into(),
            campaign: campaign.into(),
            protocol: protocol.into(),
            partition: partition.into(),
        }
    }

    fn valid(&self) -> bool {
        [
            &self.tenant_id,
            &self.purpose,
            &self.world,
            &self.campaign,
            &self.protocol,
            &self.partition,
        ]
        .into_iter()
        .all(|value| !value.is_empty())
    }

    fn matches_binding(&self, access: &WikiAccess) -> bool {
        self.tenant_id == access.tenant_id
            && self.purpose == access.purpose
            && self.world == access.memory_scope.world
            && self.campaign == access.memory_scope.campaign
            && self.protocol == access.memory_scope.protocol
            && self.partition == access.memory_scope.partition
    }
}

/// An immutable snapshot selected by the current head for one memory scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryHead {
    pub scope: MemoryScope,
    pub snapshot_ref: ArtifactReference,
    pub head_version: u64,
}

/// A U15 result submitted for publication against an exact observed head.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPublishRequest {
    pub scope: MemoryScope,
    pub expected_head_version: u64,
    pub access: WikiAccess,
    pub transform: WikiTransformResult,
}

impl MemoryPublishRequest {
    #[must_use]
    pub fn new(
        scope: MemoryScope,
        expected_head_version: u64,
        access: WikiAccess,
        transform: WikiTransformResult,
    ) -> Self {
        Self {
            scope,
            expected_head_version,
            access,
            transform,
        }
    }
}

/// A treated receipt for a permitted memory use. It records provenance, not causality.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryUseReceipt {
    pub receipt_id: String,
    pub scope: MemoryScope,
    pub snapshot_ref: ArtifactReference,
    pub head_version: u64,
    pub run_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub purpose: String,
    pub allowed_at_unix_seconds: u64,
    /// Opaque commitment emitted by the U23 temporal evidence issuer. `None`
    /// is retained only for pre-U23/U22 uses.
    pub temporal_commitment: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PublishedMemory {
    pub head: MemoryHead,
    pub snapshot: ArtifactDraft,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryError {
    InvalidScope,
    ScopeAlreadySeeded,
    SnapshotAlreadyBound,
    HeadMissing,
    HeadConflict { expected: u64, actual: u64 },
    AccessDenied,
    ScopeMismatch,
    SnapshotMismatch,
    SnapshotRevoked,
    SnapshotInvalid,
    TransformDigestMismatch,
    ReceiptConflict,
    Repository(RepositoryError),
}

impl From<RepositoryError> for MemoryError {
    fn from(value: RepositoryError) -> Self {
        Self::Repository(value)
    }
}

/// The explicit publication/read boundary used by U22/U23 instead of a broad wiki cache.
pub trait MemoryPublisher {
    fn seed_head<R: ArtifactRepository>(
        &mut self,
        artifacts: &mut R,
        scope: MemoryScope,
        snapshot_ref: ArtifactReference,
    ) -> Result<MemoryHead, MemoryError>;

    fn publish<R: ArtifactRepository, A: WikiAuthorizationPort>(
        &mut self,
        artifacts: &mut R,
        authority: &A,
        request: MemoryPublishRequest,
    ) -> Result<PublishedMemory, MemoryError>;

    fn revoke(&mut self, snapshot_ref: ArtifactReference, reason: &str) -> Result<(), MemoryError>;

    fn record_allowed_use<R: ArtifactRepository, A: WikiAuthorizationPort>(
        &mut self,
        artifacts: &mut R,
        authority: &A,
        scope: MemoryScope,
        access: WikiAccess,
        temporal_commitment: Option<String>,
        snapshot_ref: ArtifactReference,
    ) -> Result<MemoryUseReceipt, MemoryError>;
}

/// Crate-private conditional commit used to turn a governed memory request
/// into its sole receipt.  Unlike the public publication port, this boundary
/// is intentionally unavailable to transport callers: an implementation must
/// validate the current head, tombstone ancestry, authorization and exact
/// request identity *in the same durable transaction* that inserts (or
/// idempotently returns) the receipt.  It must leave no receipt on a failed
/// fence.  The in-memory implementation below models that indivisible
/// mutation; a durable adapter must preserve it with one conditional commit.
pub(crate) trait AtomicMemoryUseCommitPort {
    fn commit_allowed_use<R: ArtifactRepository, A: MemoryUseCommitAuthority>(
        &mut self,
        artifacts: &mut R,
        authority: &A,
        request: AtomicMemoryUseRequest,
    ) -> Result<MemoryUseReceipt, MemoryError>;
}

/// Exact request-side fence for one U33 conditional admission. Its fields are
/// private so only U22 can create it. The adapter—not U22—resolves the active
/// grant revision and verifies its liveness inside the same conditional commit
/// as every other fence.
#[derive(Clone, Debug)]
pub(crate) struct AtomicMemoryUseRequest {
    scope: MemoryScope,
    access: WikiAccess,
    temporal_commitment: Option<String>,
    snapshot_ref: ArtifactReference,
}

impl AtomicMemoryUseRequest {
    pub(crate) fn new(
        scope: MemoryScope,
        access: WikiAccess,
        temporal_commitment: Option<String>,
        snapshot_ref: ArtifactReference,
    ) -> Self {
        Self {
            scope,
            access,
            temporal_commitment,
            snapshot_ref,
        }
    }
}

/// Re-attests one recorded use against the current U33 head, live snapshot,
/// authorization and canonical receipt identity. U22 consumes this narrow port
/// rather than inferring validity from a positive head version.
pub trait MemoryUseReceiptAttestationPort {
    fn attest_allowed_use<R: ArtifactRepository, A: WikiAuthorizationPort>(
        &mut self,
        artifacts: &mut R,
        authority: &A,
        scope: &MemoryScope,
        access: &WikiAccess,
        temporal_commitment: Option<&str>,
        receipt: &MemoryUseReceipt,
    ) -> Result<(), MemoryError>;
}

/// Deterministic local implementation. The later PostgreSQL adapter must retain these semantics.
#[derive(Clone, Debug, Default)]
pub struct InMemoryMemoryRegistry {
    heads: BTreeMap<MemoryScope, MemoryHead>,
    bound_snapshots: BTreeMap<String, MemoryScope>,
    parents: BTreeMap<String, ArtifactReference>,
    tombstones: BTreeSet<String>,
    receipts: Vec<MemoryUseReceipt>,
    #[cfg(test)]
    next_atomic_commit_interleaving: Option<MemoryUseCommitInterleaving>,
}

/// Deterministic test-only schedule for a state change that wins between the
/// first read and final predicate of an in-memory atomic admission.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryUseCommitInterleaving {
    RevokeSnapshot,
    AdvanceHead,
}

impl InMemoryMemoryRegistry {
    #[must_use]
    pub fn receipts(&self) -> &[MemoryUseReceipt] {
        &self.receipts
    }

    #[cfg(test)]
    pub(crate) fn schedule_atomic_commit_interleaving(
        &mut self,
        interleaving: MemoryUseCommitInterleaving,
    ) {
        self.next_atomic_commit_interleaving = Some(interleaving);
    }

    #[cfg(test)]
    fn apply_scheduled_atomic_commit_interleaving(
        &mut self,
        request: &AtomicMemoryUseRequest,
    ) -> Result<(), MemoryError> {
        match self.next_atomic_commit_interleaving.take() {
            None => Ok(()),
            Some(MemoryUseCommitInterleaving::RevokeSnapshot) => {
                self.tombstones.insert(reference_key(&request.snapshot_ref));
                Ok(())
            }
            Some(MemoryUseCommitInterleaving::AdvanceHead) => {
                let head = self
                    .heads
                    .get_mut(&request.scope)
                    .ok_or(MemoryError::HeadMissing)?;
                let expected = head.head_version;
                head.head_version = expected.checked_add(1).ok_or(MemoryError::HeadConflict {
                    expected,
                    actual: expected,
                })?;
                Ok(())
            }
        }
    }

    fn verify_live_snapshot<R: ArtifactRepository>(
        &self,
        artifacts: &mut R,
        scope: &MemoryScope,
        reference: &ArtifactReference,
        allowed_at_unix_seconds: u64,
    ) -> Result<ArtifactDraft, MemoryError> {
        if self.is_revoked_or_descends_from_tombstone(reference) {
            return Err(MemoryError::SnapshotRevoked);
        }
        if reference.tenant_id != scope.tenant_id {
            return Err(MemoryError::ScopeMismatch);
        }
        let snapshot = artifacts
            .get(&reference.tenant_id, &reference.id, reference.revision)?
            .ok_or(MemoryError::SnapshotMismatch)?;
        if snapshot.reference() != *reference || snapshot.kind != ArtifactKind::MemoryWiki {
            return Err(MemoryError::SnapshotMismatch);
        }
        let decoded = decode_wiki(&snapshot.payload)?;
        if decoded.purpose != scope.purpose
            || decoded.available_at_unix_seconds > allowed_at_unix_seconds
        {
            return Err(MemoryError::ScopeMismatch);
        }
        Ok(snapshot)
    }

    fn is_revoked_or_descends_from_tombstone(&self, reference: &ArtifactReference) -> bool {
        let mut cursor = reference.clone();
        let mut visited = BTreeSet::new();
        loop {
            let key = reference_key(&cursor);
            if !visited.insert(key.clone()) || self.tombstones.contains(&key) {
                return true;
            }
            let Some(parent) = self.parents.get(&key) else {
                return false;
            };
            cursor = parent.clone();
        }
    }

    fn expected_atomic_allowed_use_receipt<R: ArtifactRepository, A: MemoryUseCommitAuthority>(
        &self,
        artifacts: &mut R,
        authority: &A,
        request: &AtomicMemoryUseRequest,
    ) -> Result<MemoryUseReceipt, MemoryError> {
        let head = self
            .heads
            .get(&request.scope)
            .cloned()
            .ok_or(MemoryError::HeadMissing)?;
        if head.snapshot_ref != request.access.snapshot_ref
            || request.access.snapshot_ref != request.snapshot_ref
        {
            return Err(MemoryError::SnapshotMismatch);
        }
        if !request.scope.matches_binding(&request.access)
            || !authority.authorize(&request.access, &head.snapshot_ref)
        {
            return Err(MemoryError::AccessDenied);
        }
        // The authority may resolve the grant from its durable current state.
        // In the real adapter this is part of the single predicate; the test
        // authority can make a replacement/revocation win immediately after
        // this initial observation, forcing the final predicate below to fail.
        if !authority.memory_use_grant_is_live(&request.access) {
            return Err(MemoryError::AccessDenied);
        }
        self.verify_live_snapshot(
            artifacts,
            &request.scope,
            &head.snapshot_ref,
            request.access.allowed_at_unix_seconds,
        )?;
        Ok(MemoryUseReceipt {
            receipt_id: receipt_id(
                &request.scope,
                &request.access,
                &head.snapshot_ref,
                head.head_version,
                request.temporal_commitment.as_deref(),
            ),
            scope: request.scope.clone(),
            snapshot_ref: head.snapshot_ref,
            head_version: head.head_version,
            run_id: request.access.run_id.clone(),
            grant_id: request.access.grant_id.clone(),
            grant_revision: request.access.grant_revision,
            purpose: request.access.purpose.clone(),
            allowed_at_unix_seconds: request.access.allowed_at_unix_seconds,
            temporal_commitment: request.temporal_commitment.clone(),
        })
    }

    fn expected_allowed_use_receipt<R: ArtifactRepository, A: WikiAuthorizationPort>(
        &self,
        artifacts: &mut R,
        authority: &A,
        scope: &MemoryScope,
        access: &WikiAccess,
        temporal_commitment: Option<&str>,
    ) -> Result<MemoryUseReceipt, MemoryError> {
        let head = self
            .heads
            .get(scope)
            .cloned()
            .ok_or(MemoryError::HeadMissing)?;
        if head.snapshot_ref != access.snapshot_ref {
            return Err(MemoryError::SnapshotMismatch);
        }
        if !scope.matches_binding(access) || !authority.authorize(access, &head.snapshot_ref) {
            return Err(MemoryError::AccessDenied);
        }
        self.verify_live_snapshot(
            artifacts,
            scope,
            &head.snapshot_ref,
            access.allowed_at_unix_seconds,
        )?;
        Ok(MemoryUseReceipt {
            receipt_id: receipt_id(
                scope,
                access,
                &head.snapshot_ref,
                head.head_version,
                temporal_commitment,
            ),
            scope: scope.clone(),
            snapshot_ref: head.snapshot_ref,
            head_version: head.head_version,
            run_id: access.run_id.clone(),
            grant_id: access.grant_id.clone(),
            grant_revision: access.grant_revision,
            purpose: access.purpose.clone(),
            allowed_at_unix_seconds: access.allowed_at_unix_seconds,
            temporal_commitment: temporal_commitment.map(str::to_owned),
        })
    }
}

impl MemoryPublisher for InMemoryMemoryRegistry {
    fn seed_head<R: ArtifactRepository>(
        &mut self,
        artifacts: &mut R,
        scope: MemoryScope,
        snapshot_ref: ArtifactReference,
    ) -> Result<MemoryHead, MemoryError> {
        if !scope.valid() {
            return Err(MemoryError::InvalidScope);
        }
        if self.heads.contains_key(&scope) {
            return Err(MemoryError::ScopeAlreadySeeded);
        }
        if self
            .bound_snapshots
            .contains_key(&reference_key(&snapshot_ref))
        {
            return Err(MemoryError::SnapshotAlreadyBound);
        }
        self.verify_live_snapshot(artifacts, &scope, &snapshot_ref, u64::MAX)?;
        let head = MemoryHead {
            scope: scope.clone(),
            snapshot_ref: snapshot_ref.clone(),
            head_version: 1,
        };
        self.bound_snapshots
            .insert(reference_key(&snapshot_ref), scope.clone());
        self.heads.insert(scope, head.clone());
        Ok(head)
    }

    fn publish<R: ArtifactRepository, A: WikiAuthorizationPort>(
        &mut self,
        artifacts: &mut R,
        authority: &A,
        request: MemoryPublishRequest,
    ) -> Result<PublishedMemory, MemoryError> {
        if !request.scope.valid() {
            return Err(MemoryError::InvalidScope);
        }
        let head = self
            .heads
            .get(&request.scope)
            .cloned()
            .ok_or(MemoryError::HeadMissing)?;
        if request.expected_head_version != head.head_version {
            return Err(MemoryError::HeadConflict {
                expected: request.expected_head_version,
                actual: head.head_version,
            });
        }
        if !request.scope.matches_binding(&request.access)
            || request.access.snapshot_ref != head.snapshot_ref
            || request.transform.receipt.snapshot_ref != head.snapshot_ref
            || !authority.authorize(&request.access, &head.snapshot_ref)
        {
            return Err(MemoryError::AccessDenied);
        }
        let receipt = &request.transform.receipt;
        if receipt.workspace_id.is_empty()
            || receipt.run_id != request.access.run_id
            || receipt.tenant_id != request.access.tenant_id
            || receipt.purpose != request.access.purpose
            || receipt.grant_id != request.access.grant_id
            || receipt.memory_scope
                != MemoryScopeBinding::new(
                    request.scope.world.clone(),
                    request.scope.campaign.clone(),
                    request.scope.protocol.clone(),
                    request.scope.partition.clone(),
                )
        {
            return Err(MemoryError::AccessDenied);
        }
        let base = self.verify_live_snapshot(
            artifacts,
            &request.scope,
            &head.snapshot_ref,
            request.access.allowed_at_unix_seconds,
        )?;
        if pages_digest(&request.transform.pages) != request.transform.result_digest
            || request.transform.receipt.result_digest != request.transform.result_digest
            || !request
                .transform
                .pages
                .iter()
                .all(|(path, content)| valid_page_path(path) && content.len() <= 64 * 1024)
        {
            return Err(MemoryError::TransformDigestMismatch);
        }
        let next_revision = base
            .revision
            .checked_add(1)
            .ok_or(MemoryError::SnapshotInvalid)?;
        // Validate every fallible state transition before the immutable append.
        // Otherwise a saturated memory head would leave a new snapshot that no
        // head can ever select.
        let next_head_version = head
            .head_version
            .checked_add(1)
            .ok_or(MemoryError::SnapshotInvalid)?;
        let next_payload = json!({
            "available_at_unix_seconds": request.access.allowed_at_unix_seconds,
            "purpose": request.scope.purpose,
            "pages": request.transform.pages,
        });
        let next = artifacts.append(
            Some(base.revision),
            ArtifactDraft::new(
                request.scope.tenant_id.clone(),
                base.id.clone(),
                next_revision,
                ArtifactKind::MemoryWiki,
                next_payload,
                None,
            ),
        )?;
        let next_head = MemoryHead {
            scope: request.scope.clone(),
            snapshot_ref: next.reference(),
            head_version: next_head_version,
        };
        self.bound_snapshots
            .insert(reference_key(&next.reference()), request.scope.clone());
        self.parents
            .insert(reference_key(&next.reference()), base.reference());
        self.heads.insert(request.scope, next_head.clone());
        Ok(PublishedMemory {
            head: next_head,
            snapshot: next,
        })
    }

    fn revoke(&mut self, snapshot_ref: ArtifactReference, reason: &str) -> Result<(), MemoryError> {
        if reason.is_empty()
            || !self
                .bound_snapshots
                .contains_key(&reference_key(&snapshot_ref))
        {
            return Err(MemoryError::SnapshotMismatch);
        }
        self.tombstones.insert(reference_key(&snapshot_ref));
        Ok(())
    }

    fn record_allowed_use<R: ArtifactRepository, A: WikiAuthorizationPort>(
        &mut self,
        artifacts: &mut R,
        authority: &A,
        scope: MemoryScope,
        access: WikiAccess,
        temporal_commitment: Option<String>,
        snapshot_ref: ArtifactReference,
    ) -> Result<MemoryUseReceipt, MemoryError> {
        if access.snapshot_ref != snapshot_ref {
            return Err(MemoryError::SnapshotMismatch);
        }
        let receipt = self.expected_allowed_use_receipt(
            artifacts,
            authority,
            &scope,
            &access,
            temporal_commitment.as_deref(),
        )?;
        if let Some(existing) = self
            .receipts
            .iter()
            .find(|existing| existing.receipt_id == receipt.receipt_id)
        {
            return if existing == &receipt {
                Ok(existing.clone())
            } else {
                Err(MemoryError::ReceiptConflict)
            };
        }
        self.receipts.push(receipt.clone());
        Ok(receipt)
    }
}

impl AtomicMemoryUseCommitPort for InMemoryMemoryRegistry {
    fn commit_allowed_use<R: ArtifactRepository, A: MemoryUseCommitAuthority>(
        &mut self,
        artifacts: &mut R,
        authority: &A,
        request: AtomicMemoryUseRequest,
    ) -> Result<MemoryUseReceipt, MemoryError> {
        // The registry owns head, tombstone and receipt state behind this one
        // mutable boundary. `record_allowed_use` computes every fence before
        // inserting, so a failing fence cannot leave a receipt.
        let receipt = self.expected_atomic_allowed_use_receipt(artifacts, authority, &request)?;
        #[cfg(test)]
        self.apply_scheduled_atomic_commit_interleaving(&request)?;
        // This repeats every fence after the deterministic interleaving. A
        // durable adapter expresses the same invariant in one SQL predicate;
        // no receipt is inserted until the final predicate agrees with the
        // initially observed exact head identity/version and authorization.
        let final_receipt =
            self.expected_atomic_allowed_use_receipt(artifacts, authority, &request)?;
        if final_receipt != receipt {
            return Err(MemoryError::HeadConflict {
                expected: receipt.head_version,
                actual: final_receipt.head_version,
            });
        }
        if let Some(existing) = self
            .receipts
            .iter()
            .find(|existing| existing.receipt_id == receipt.receipt_id)
        {
            return if existing == &receipt {
                Ok(existing.clone())
            } else {
                Err(MemoryError::ReceiptConflict)
            };
        }
        self.receipts.push(receipt.clone());
        Ok(receipt)
    }
}

impl MemoryUseReceiptAttestationPort for InMemoryMemoryRegistry {
    fn attest_allowed_use<R: ArtifactRepository, A: WikiAuthorizationPort>(
        &mut self,
        artifacts: &mut R,
        authority: &A,
        scope: &MemoryScope,
        access: &WikiAccess,
        temporal_commitment: Option<&str>,
        receipt: &MemoryUseReceipt,
    ) -> Result<(), MemoryError> {
        let expected = self.expected_allowed_use_receipt(
            artifacts,
            authority,
            scope,
            access,
            temporal_commitment,
        )?;
        if &expected != receipt || !self.receipts.iter().any(|recorded| recorded == receipt) {
            return Err(MemoryError::ReceiptConflict);
        }
        Ok(())
    }
}

struct DecodedWiki {
    available_at_unix_seconds: u64,
    purpose: String,
}

fn decode_wiki(payload: &Value) -> Result<DecodedWiki, MemoryError> {
    let object = payload.as_object().ok_or(MemoryError::SnapshotInvalid)?;
    if object.len() != 3
        || !object.contains_key("available_at_unix_seconds")
        || !object.contains_key("purpose")
        || !object.contains_key("pages")
    {
        return Err(MemoryError::SnapshotInvalid);
    }
    let available_at_unix_seconds = object
        .get("available_at_unix_seconds")
        .and_then(Value::as_u64)
        .ok_or(MemoryError::SnapshotInvalid)?;
    let purpose = object
        .get("purpose")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(MemoryError::SnapshotInvalid)?
        .to_owned();
    if object.get("pages").and_then(Value::as_object).is_none() {
        return Err(MemoryError::SnapshotInvalid);
    }
    Ok(DecodedWiki {
        available_at_unix_seconds,
        purpose,
    })
}

fn pages_digest(pages: &BTreeMap<String, String>) -> String {
    let bytes = serde_json::to_vec(pages).expect("BTreeMap pages serialize deterministically");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn valid_page_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn reference_key(reference: &ArtifactReference) -> String {
    format!(
        "{}:{}:{}:{}",
        reference.tenant_id, reference.id, reference.revision, reference.digest
    )
}

fn receipt_id(
    scope: &MemoryScope,
    access: &WikiAccess,
    snapshot_ref: &ArtifactReference,
    head_version: u64,
    temporal_commitment: Option<&str>,
) -> String {
    let bytes = serde_json::to_vec(&(
        scope,
        &access.run_id,
        &access.tenant_id,
        &access.purpose,
        &access.grant_id,
        access.grant_revision,
        snapshot_ref,
        access.allowed_at_unix_seconds,
        head_version,
        temporal_commitment,
    ))
    .expect("memory receipt inputs serialize deterministically");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiki_scratch::{
        InMemoryWikiGrantAuthority, WikiGrant, WikiScratchPort, WikiTransform,
        WikiTransformOperation,
    };
    use crate::{ArtifactKind, InMemoryArtifactRepository};

    const TENANT: &str = "tenant-a";
    const WIKI_ID: &str = "018f50a1-7f00-7000-8000-000000000001";

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

    fn binding() -> MemoryScopeBinding {
        MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train")
    }

    #[test]
    fn publish_overflow_does_not_append_an_orphan_snapshot_or_advance_the_head() {
        let mut artifacts = InMemoryArtifactRepository::default();
        let initial = artifacts
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
                        "pages": {"index.md": "original"}
                    }),
                    None,
                ),
            )
            .unwrap();
        let access = WikiAccess::new_scoped(
            "run-1",
            TENANT,
            "investigation",
            "grant-1",
            initial.reference(),
            100,
            binding(),
        );
        let mut authority = InMemoryWikiGrantAuthority::default();
        authority.issue(WikiGrant::new_scoped(
            "grant-1",
            "run-1",
            TENANT,
            "investigation",
            initial.reference(),
            binding(),
        ));
        let mut workspace = authority.mount(&mut artifacts, access.clone()).unwrap();
        let transform = authority
            .transform(
                &mut workspace,
                &access,
                WikiTransform::new(vec![WikiTransformOperation::replace("index.md", "next")]),
            )
            .unwrap();
        let memory_scope = scope();
        let mut registry = InMemoryMemoryRegistry::default();
        registry
            .seed_head(&mut artifacts, memory_scope.clone(), initial.reference())
            .unwrap();

        // Fixture only: persistent adapters can restore a head written at this
        // boundary; exercising the public publish API must still be atomic at
        // the numeric limit.
        registry.heads.get_mut(&memory_scope).unwrap().head_version = u64::MAX;

        assert_eq!(
            registry
                .publish(
                    &mut artifacts,
                    &authority,
                    MemoryPublishRequest::new(memory_scope.clone(), u64::MAX, access, transform),
                )
                .unwrap_err(),
            MemoryError::SnapshotInvalid
        );
        assert_eq!(registry.heads[&memory_scope].head_version, u64::MAX);
        assert!(artifacts.get(TENANT, WIKI_ID, 2).unwrap().is_none());
    }
}
