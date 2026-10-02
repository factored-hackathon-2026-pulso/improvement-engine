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
    MemoryScopeBinding, WikiAccess, WikiAuthorizationPort, WikiTransformResult,
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
    pub purpose: String,
    pub allowed_at_unix_seconds: u64,
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
        snapshot_ref: ArtifactReference,
    ) -> Result<MemoryUseReceipt, MemoryError>;
}

/// Deterministic local implementation. The later PostgreSQL adapter must retain these semantics.
#[derive(Clone, Debug, Default)]
pub struct InMemoryMemoryRegistry {
    heads: BTreeMap<MemoryScope, MemoryHead>,
    bound_snapshots: BTreeMap<String, MemoryScope>,
    parents: BTreeMap<String, ArtifactReference>,
    tombstones: BTreeSet<String>,
    receipts: Vec<MemoryUseReceipt>,
}

impl InMemoryMemoryRegistry {
    #[must_use]
    pub fn receipts(&self) -> &[MemoryUseReceipt] {
        &self.receipts
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
            head_version: head
                .head_version
                .checked_add(1)
                .ok_or(MemoryError::SnapshotInvalid)?,
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
        snapshot_ref: ArtifactReference,
    ) -> Result<MemoryUseReceipt, MemoryError> {
        let head = self
            .heads
            .get(&scope)
            .cloned()
            .ok_or(MemoryError::HeadMissing)?;
        if head.snapshot_ref != snapshot_ref || access.snapshot_ref != snapshot_ref {
            return Err(MemoryError::SnapshotMismatch);
        }
        if !scope.matches_binding(&access) || !authority.authorize(&access, &snapshot_ref) {
            return Err(MemoryError::AccessDenied);
        }
        self.verify_live_snapshot(
            artifacts,
            &scope,
            &snapshot_ref,
            access.allowed_at_unix_seconds,
        )?;
        let receipt = MemoryUseReceipt {
            receipt_id: receipt_id(&scope, &access, &snapshot_ref, head.head_version),
            scope,
            snapshot_ref,
            head_version: head.head_version,
            run_id: access.run_id,
            grant_id: access.grant_id,
            purpose: access.purpose,
            allowed_at_unix_seconds: access.allowed_at_unix_seconds,
        };
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
) -> String {
    let bytes = serde_json::to_vec(&(
        scope,
        &access.run_id,
        &access.tenant_id,
        &access.purpose,
        &access.grant_id,
        snapshot_ref,
        access.allowed_at_unix_seconds,
        head_version,
    ))
    .expect("memory receipt inputs serialize deterministically");
    format!("sha256:{:x}", Sha256::digest(bytes))
}
