//! Authorized, ephemeral wiki workspaces for improvement investigations.
//!
//! U15 deliberately offers no host filesystem, network, publication, head or
//! revocation operation. A workspace is materialized from one exact immutable
//! `memory_wiki` artifact revision and can only be read or transformed in
//! process. U33 owns durable memory publication and revocation; U05 will
//! replace the deterministic grant authority below with the service grant
//! boundary without changing the workspace contract.

use std::cell::RefCell;
use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{ArtifactKind, ArtifactReference, ArtifactRepository};

/// Pinned information partition for memory work. It deliberately excludes tenant
/// and purpose because those remain explicit `WikiAccess` claims and are checked
/// together at every boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryScopeBinding {
    pub world: String,
    pub campaign: String,
    pub protocol: String,
    pub partition: String,
}

impl MemoryScopeBinding {
    #[must_use]
    pub fn new(
        world: impl Into<String>,
        campaign: impl Into<String>,
        protocol: impl Into<String>,
        partition: impl Into<String>,
    ) -> Self {
        Self {
            world: world.into(),
            campaign: campaign.into(),
            protocol: protocol.into(),
            partition: partition.into(),
        }
    }

    #[must_use]
    pub fn unscoped() -> Self {
        Self::new("unscoped", "unscoped", "unscoped", "unscoped")
    }
}

/// An exact snapshot and caller context submitted at every workspace operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WikiAccess {
    pub run_id: String,
    pub tenant_id: String,
    pub purpose: String,
    pub grant_id: String,
    /// Exact immutable revision of the grant selected by trusted composition.
    /// A later grant replacement or revocation must not authorize this access.
    pub grant_revision: u64,
    pub snapshot_ref: ArtifactReference,
    pub allowed_at_unix_seconds: u64,
    pub memory_scope: MemoryScopeBinding,
}

impl WikiAccess {
    #[must_use]
    pub fn new(
        run_id: impl Into<String>,
        tenant_id: impl Into<String>,
        purpose: impl Into<String>,
        grant_id: impl Into<String>,
        snapshot_ref: ArtifactReference,
        allowed_at_unix_seconds: u64,
    ) -> Self {
        Self::new_scoped(
            run_id,
            tenant_id,
            purpose,
            grant_id,
            snapshot_ref,
            allowed_at_unix_seconds,
            MemoryScopeBinding::unscoped(),
        )
    }

    #[must_use]
    pub fn new_scoped(
        run_id: impl Into<String>,
        tenant_id: impl Into<String>,
        purpose: impl Into<String>,
        grant_id: impl Into<String>,
        snapshot_ref: ArtifactReference,
        allowed_at_unix_seconds: u64,
        memory_scope: MemoryScopeBinding,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            tenant_id: tenant_id.into(),
            purpose: purpose.into(),
            grant_id: grant_id.into(),
            grant_revision: 1,
            snapshot_ref,
            allowed_at_unix_seconds,
            memory_scope,
        }
    }

    #[must_use]
    pub fn with_grant_revision(mut self, grant_revision: u64) -> Self {
        self.grant_revision = grant_revision;
        self
    }
}

/// A validated grant record for the deterministic adapter only.
///
/// U05 will become the authority for grant lifecycle and revocation. This type
/// intentionally grants exactly one run, snapshot, tenant and purpose rather
/// than a broad wiki namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WikiGrant {
    grant_id: String,
    revision: u64,
    run_id: String,
    tenant_id: String,
    purpose: String,
    snapshot_ref: ArtifactReference,
    memory_scope: MemoryScopeBinding,
}

impl WikiGrant {
    #[must_use]
    pub fn new(
        grant_id: impl Into<String>,
        run_id: impl Into<String>,
        tenant_id: impl Into<String>,
        purpose: impl Into<String>,
        snapshot_ref: ArtifactReference,
    ) -> Self {
        Self::new_scoped(
            grant_id,
            run_id,
            tenant_id,
            purpose,
            snapshot_ref,
            MemoryScopeBinding::unscoped(),
        )
    }

    #[must_use]
    pub fn new_scoped(
        grant_id: impl Into<String>,
        run_id: impl Into<String>,
        tenant_id: impl Into<String>,
        purpose: impl Into<String>,
        snapshot_ref: ArtifactReference,
        memory_scope: MemoryScopeBinding,
    ) -> Self {
        Self {
            grant_id: grant_id.into(),
            revision: 1,
            run_id: run_id.into(),
            tenant_id: tenant_id.into(),
            purpose: purpose.into(),
            snapshot_ref,
            memory_scope,
        }
    }

    #[must_use]
    pub fn with_revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }
}

/// Narrow authorization seam retained at mount, read and transform boundaries.
pub trait WikiAuthorizationPort {
    fn authorize(&self, access: &WikiAccess, snapshot_ref: &ArtifactReference) -> bool;
}

/// Crate-private extension used by the governed-memory commit. A production
/// implementation must resolve the exact grant revision and verify it remains
/// live *inside the same transaction* as the U33 receipt predicate. U22 must
/// not pre-read a fence and hand a stale authorization observation to U33.
#[allow(dead_code)] // Consumed only by the crate-private U33 composition adapter.
pub(crate) trait MemoryUseCommitAuthority: WikiAuthorizationPort {
    fn memory_use_grant_is_live(&self, access: &WikiAccess) -> bool;
}

/// Test/local authority that only accepts issued exact grants.
///
/// It is not an identity provider or substitute for U05. It makes the required
/// authorization seam executable without granting host or source access.
#[derive(Default)]
pub struct InMemoryWikiGrantAuthority {
    grants: RefCell<BTreeMap<String, WikiGrant>>,
}

impl InMemoryWikiGrantAuthority {
    pub fn issue(&self, grant: WikiGrant) {
        self.grants
            .borrow_mut()
            .insert(grant.grant_id.clone(), grant);
    }

    pub fn revoke(&self, grant_id: &str) -> bool {
        self.grants.borrow_mut().remove(grant_id).is_some()
    }

    #[allow(dead_code)] // Used by U33's deterministic final-boundary race regression.
    pub(crate) fn replace_revision(&self, grant_id: &str, revision: u64) -> bool {
        if revision == 0 {
            return false;
        }
        let mut grants = self.grants.borrow_mut();
        let Some(grant) = grants.get_mut(grant_id) else {
            return false;
        };
        grant.revision = revision;
        true
    }
}

impl WikiAuthorizationPort for InMemoryWikiGrantAuthority {
    fn authorize(&self, access: &WikiAccess, snapshot_ref: &ArtifactReference) -> bool {
        self.grants
            .borrow()
            .get(&access.grant_id)
            .is_some_and(|grant| {
                grant.run_id == access.run_id
                    && grant.revision == access.grant_revision
                    && grant.tenant_id == access.tenant_id
                    && grant.purpose == access.purpose
                    && grant.snapshot_ref == *snapshot_ref
                    && access.snapshot_ref == *snapshot_ref
                    && grant.memory_scope == access.memory_scope
            })
    }
}

impl MemoryUseCommitAuthority for InMemoryWikiGrantAuthority {
    fn memory_use_grant_is_live(&self, access: &WikiAccess) -> bool {
        self.authorize(access, &access.snapshot_ref)
    }
}

/// The sole stateful object of one scratch mount. It is never persisted by U15.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WikiWorkspace {
    workspace_id: String,
    run_id: String,
    snapshot_ref: ArtifactReference,
    purpose: String,
    memory_scope: MemoryScopeBinding,
    snapshot_available_at_unix_seconds: u64,
    pages: BTreeMap<String, String>,
}

impl WikiWorkspace {
    #[must_use]
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    #[must_use]
    pub fn pages(&self) -> &BTreeMap<String, String> {
        &self.pages
    }
}

/// Evidence for an ephemeral mount. It cites the exact source revision only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WikiMountReceipt {
    pub workspace_id: String,
    pub snapshot_ref: ArtifactReference,
    pub purpose: String,
    pub grant_id: String,
    pub allowed_at_unix_seconds: u64,
}

/// Result of an authorized scratch read; source content never reaches a host path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WikiReadResult {
    pub content: String,
    pub snapshot_ref: ArtifactReference,
    pub workspace_id: String,
    pub path: String,
}

/// Typed in-scratch change; no arbitrary command or filesystem operation exists.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum WikiTransformOperation {
    Create { path: String, content: String },
    Replace { path: String, content: String },
    Remove { path: String },
}

impl WikiTransformOperation {
    #[must_use]
    pub fn create(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Create {
            path: path.into(),
            content: content.into(),
        }
    }

    #[must_use]
    pub fn replace(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Replace {
            path: path.into(),
            content: content.into(),
        }
    }

    #[must_use]
    pub fn remove(path: impl Into<String>) -> Self {
        Self::Remove { path: path.into() }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WikiTransform {
    operations: Vec<WikiTransformOperation>,
}

impl WikiTransform {
    #[must_use]
    pub fn new(operations: Vec<WikiTransformOperation>) -> Self {
        Self { operations }
    }
}

/// Deterministic evidence that one scratch-only batch completed atomically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WikiTransformReceipt {
    pub workspace_id: String,
    pub snapshot_ref: ArtifactReference,
    pub run_id: String,
    pub tenant_id: String,
    pub purpose: String,
    pub grant_id: String,
    pub memory_scope: MemoryScopeBinding,
    pub transform_digest: String,
    pub result_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WikiTransformResult {
    pub pages: BTreeMap<String, String>,
    pub result_digest: String,
    pub receipt: WikiTransformReceipt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WikiError {
    AuthorizationDenied,
    SnapshotNotFound,
    SnapshotReferenceMismatch,
    SnapshotKindMismatch,
    SnapshotPayloadInvalid,
    FutureSnapshot {
        available_at_unix_seconds: u64,
        allowed_at_unix_seconds: u64,
    },
    WorkspaceAccessDenied,
    InvalidPath {
        path: String,
    },
    PageMissing {
        path: String,
    },
    PageAlreadyExists {
        path: String,
    },
}

/// Operations available to a generic investigator inside its isolated scratch.
///
/// The port has no publish method by construction. An implementation can be
/// replaced by a process-isolated runner later without changing callers.
pub trait WikiScratchPort {
    fn mount<R: ArtifactRepository>(
        &mut self,
        repository: &mut R,
        access: WikiAccess,
    ) -> Result<WikiWorkspace, WikiError>;

    fn read(
        &self,
        workspace: &WikiWorkspace,
        access: &WikiAccess,
        path: &str,
    ) -> Result<WikiReadResult, WikiError>;

    fn transform(
        &self,
        workspace: &mut WikiWorkspace,
        access: &WikiAccess,
        transform: WikiTransform,
    ) -> Result<WikiTransformResult, WikiError>;
}

/// Recomputes an already-issued scratch transform from its sealed source
/// snapshot without exposing its pages to a new caller. Composition-only
/// consumers use it before a governed boundary redeems a result; matching
/// receipt strings alone are not sufficient evidence that the result came
/// from the claimed canonical transform.
pub(crate) fn verify_transform_result_against_snapshot<R: ArtifactRepository>(
    repository: &mut R,
    access: &WikiAccess,
    transform: &WikiTransform,
    result: &WikiTransformResult,
) -> Result<bool, WikiError> {
    let snapshot = repository
        .get(
            &access.snapshot_ref.tenant_id,
            &access.snapshot_ref.id,
            access.snapshot_ref.revision,
        )
        .map_err(|_| WikiError::SnapshotNotFound)?
        .ok_or(WikiError::SnapshotNotFound)?;
    if snapshot.reference() != access.snapshot_ref {
        return Err(WikiError::SnapshotReferenceMismatch);
    }
    if snapshot.kind != ArtifactKind::MemoryWiki {
        return Err(WikiError::SnapshotKindMismatch);
    }
    let decoded = decode_snapshot(&snapshot.payload)?;
    if decoded.available_at_unix_seconds > access.allowed_at_unix_seconds {
        return Err(WikiError::FutureSnapshot {
            available_at_unix_seconds: decoded.available_at_unix_seconds,
            allowed_at_unix_seconds: access.allowed_at_unix_seconds,
        });
    }
    if decoded.purpose != access.purpose {
        return Err(WikiError::AuthorizationDenied);
    }
    let mut expected_pages = decoded.pages;
    for operation in &transform.operations {
        apply_operation(&mut expected_pages, operation)?;
    }
    Ok(result.receipt.snapshot_ref == access.snapshot_ref
        && result.receipt.run_id == access.run_id
        && result.receipt.tenant_id == access.tenant_id
        && result.receipt.purpose == access.purpose
        && result.receipt.grant_id == access.grant_id
        && result.receipt.memory_scope == access.memory_scope
        && result.receipt.transform_digest == digest(transform)
        && result.receipt.result_digest == digest(&expected_pages)
        && result.result_digest == digest(&expected_pages)
        && result.pages == expected_pages)
}

impl WikiScratchPort for InMemoryWikiGrantAuthority {
    fn mount<R: ArtifactRepository>(
        &mut self,
        repository: &mut R,
        access: WikiAccess,
    ) -> Result<WikiWorkspace, WikiError> {
        if !self.authorize(&access, &access.snapshot_ref) {
            return Err(WikiError::AuthorizationDenied);
        }
        let snapshot = repository
            .get(
                &access.snapshot_ref.tenant_id,
                &access.snapshot_ref.id,
                access.snapshot_ref.revision,
            )
            .map_err(|_| WikiError::SnapshotNotFound)?
            .ok_or(WikiError::SnapshotNotFound)?;
        if snapshot.reference() != access.snapshot_ref {
            return Err(WikiError::SnapshotReferenceMismatch);
        }
        if snapshot.kind != ArtifactKind::MemoryWiki {
            return Err(WikiError::SnapshotKindMismatch);
        }
        let decoded = decode_snapshot(&snapshot.payload)?;
        if decoded.available_at_unix_seconds > access.allowed_at_unix_seconds {
            return Err(WikiError::FutureSnapshot {
                available_at_unix_seconds: decoded.available_at_unix_seconds,
                allowed_at_unix_seconds: access.allowed_at_unix_seconds,
            });
        }
        if decoded.purpose != access.purpose {
            return Err(WikiError::AuthorizationDenied);
        }
        Ok(WikiWorkspace {
            workspace_id: workspace_id(&access),
            run_id: access.run_id,
            snapshot_ref: access.snapshot_ref,
            purpose: decoded.purpose,
            memory_scope: access.memory_scope,
            snapshot_available_at_unix_seconds: decoded.available_at_unix_seconds,
            pages: decoded.pages,
        })
    }

    fn read(
        &self,
        workspace: &WikiWorkspace,
        access: &WikiAccess,
        path: &str,
    ) -> Result<WikiReadResult, WikiError> {
        validate_workspace_access(self, workspace, access)?;
        validate_path(path)?;
        let content = workspace
            .pages
            .get(path)
            .cloned()
            .ok_or_else(|| WikiError::PageMissing {
                path: path.to_owned(),
            })?;
        Ok(WikiReadResult {
            content,
            snapshot_ref: workspace.snapshot_ref.clone(),
            workspace_id: workspace.workspace_id.clone(),
            path: path.to_owned(),
        })
    }

    fn transform(
        &self,
        workspace: &mut WikiWorkspace,
        access: &WikiAccess,
        transform: WikiTransform,
    ) -> Result<WikiTransformResult, WikiError> {
        validate_workspace_access(self, workspace, access)?;
        let mut next_pages = workspace.pages.clone();
        for operation in &transform.operations {
            apply_operation(&mut next_pages, operation)?;
        }
        let transform_digest = digest(&transform);
        let result_digest = digest(&next_pages);
        workspace.pages = next_pages.clone();
        Ok(WikiTransformResult {
            pages: next_pages,
            result_digest: result_digest.clone(),
            receipt: WikiTransformReceipt {
                workspace_id: workspace.workspace_id.clone(),
                snapshot_ref: workspace.snapshot_ref.clone(),
                run_id: access.run_id.clone(),
                tenant_id: access.tenant_id.clone(),
                purpose: access.purpose.clone(),
                grant_id: access.grant_id.clone(),
                memory_scope: access.memory_scope.clone(),
                transform_digest,
                result_digest,
            },
        })
    }
}

struct DecodedSnapshot {
    available_at_unix_seconds: u64,
    purpose: String,
    pages: BTreeMap<String, String>,
}

fn decode_snapshot(payload: &Value) -> Result<DecodedSnapshot, WikiError> {
    let object = payload
        .as_object()
        .ok_or(WikiError::SnapshotPayloadInvalid)?;
    if object.len() != 3
        || !object.contains_key("available_at_unix_seconds")
        || !object.contains_key("purpose")
        || !object.contains_key("pages")
    {
        return Err(WikiError::SnapshotPayloadInvalid);
    }
    let available_at_unix_seconds = object
        .get("available_at_unix_seconds")
        .and_then(Value::as_u64)
        .ok_or(WikiError::SnapshotPayloadInvalid)?;
    let purpose = object
        .get("purpose")
        .and_then(Value::as_str)
        .filter(|purpose| !purpose.is_empty())
        .ok_or(WikiError::SnapshotPayloadInvalid)?
        .to_owned();
    let pages = object
        .get("pages")
        .and_then(Value::as_object)
        .ok_or(WikiError::SnapshotPayloadInvalid)?
        .iter()
        .map(|(path, content)| {
            validate_path(path)?;
            content
                .as_str()
                .map(|content| (path.clone(), content.to_owned()))
                .ok_or(WikiError::SnapshotPayloadInvalid)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(DecodedSnapshot {
        available_at_unix_seconds,
        purpose,
        pages,
    })
}

fn validate_workspace_access<A: WikiAuthorizationPort>(
    authority: &A,
    workspace: &WikiWorkspace,
    access: &WikiAccess,
) -> Result<(), WikiError> {
    if !authority.authorize(access, &workspace.snapshot_ref) {
        return Err(WikiError::AuthorizationDenied);
    }
    if workspace.run_id != access.run_id
        || workspace.snapshot_ref != access.snapshot_ref
        || workspace.purpose != access.purpose
        || workspace.memory_scope != access.memory_scope
    {
        return Err(WikiError::WorkspaceAccessDenied);
    }
    if workspace.snapshot_available_at_unix_seconds > access.allowed_at_unix_seconds {
        return Err(WikiError::FutureSnapshot {
            available_at_unix_seconds: workspace.snapshot_available_at_unix_seconds,
            allowed_at_unix_seconds: access.allowed_at_unix_seconds,
        });
    }
    Ok(())
}

fn apply_operation(
    pages: &mut BTreeMap<String, String>,
    operation: &WikiTransformOperation,
) -> Result<(), WikiError> {
    match operation {
        WikiTransformOperation::Create { path, content } => {
            validate_path(path)?;
            if pages.contains_key(path) {
                return Err(WikiError::PageAlreadyExists { path: path.clone() });
            }
            pages.insert(path.clone(), content.clone());
        }
        WikiTransformOperation::Replace { path, content } => {
            validate_path(path)?;
            let page = pages
                .get_mut(path)
                .ok_or_else(|| WikiError::PageMissing { path: path.clone() })?;
            *page = content.clone();
        }
        WikiTransformOperation::Remove { path } => {
            validate_path(path)?;
            pages
                .remove(path)
                .ok_or_else(|| WikiError::PageMissing { path: path.clone() })?;
        }
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), WikiError> {
    let valid = !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if valid {
        Ok(())
    } else {
        Err(WikiError::InvalidPath {
            path: path.to_owned(),
        })
    }
}

fn workspace_id(access: &WikiAccess) -> String {
    digest(&(
        &access.run_id,
        &access.tenant_id,
        &access.purpose,
        &access.grant_id,
        &access.snapshot_ref,
        &access.memory_scope,
        access.allowed_at_unix_seconds,
    ))
}

fn digest<T: Serialize>(value: &T) -> String {
    let bytes =
        serde_json::to_vec(value).expect("supported U15 values serialize deterministically");
    format!("sha256:{:x}", Sha256::digest(bytes))
}
