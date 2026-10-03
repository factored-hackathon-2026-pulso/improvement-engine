//! U23-E admission of a later Frozen E0 run against the exact U33-E head.
//!
//! This boundary intentionally does not route through generic U22/U33 memory
//! state. U33-E owns the publication sidecar, current head, revocation overlay,
//! and use receipt together; durable admission remains dependency-blocked until
//! one adapter can preserve that same transaction boundary.

use crate::ArtifactRepository;
use crate::e0_frozen_memory_publication::{
    FrozenE0SummaryPublicationError, FrozenE0SummaryPublicationPort, PublishedFrozenE0MemorySummary,
};
use crate::enriched_history::VerifiedReplayAvailability;
use crate::memory_store::MemoryScope;
use crate::memory_temporal_protocol::{TemporalProtocolError, attest_frozen_e0_reuse};
use crate::wiki_scratch::{
    WikiAccess, WikiAuthorizationPort, WikiError, WikiReadResult, WikiScratchPort,
};

#[derive(Debug, Eq, PartialEq)]
#[allow(dead_code)] // The trusted runtime composition is not wired into the demo runner yet.
pub(crate) enum FrozenE0MemoryCycleError {
    Temporal(TemporalProtocolError),
    Publication(FrozenE0SummaryPublicationError),
    UseAccessMismatch,
    Scratch(WikiError),
}

/// Opaque provenance that a subsequent Frozen run was admitted to use the
/// exact, still-live U33-E publication before its replay cutoff.
#[allow(dead_code)] // The trusted runtime composition is not wired into the demo runner yet.
pub(crate) struct VerifiedFrozenE0MemoryUse {
    receipt: FrozenE0MemoryUseReceipt,
}

/// Treated receipt written by the same U33-E state that owns publication and
/// head advancement. It contains no wiki page payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrozenE0MemoryUseReceipt {
    pub(crate) receipt_id: String,
    pub(crate) publication_commitment: String,
    pub(crate) scope: MemoryScope,
    pub(crate) snapshot_ref: crate::ArtifactReference,
    pub(crate) head_version: u64,
    pub(crate) run_id: String,
    pub(crate) grant_id: String,
    pub(crate) grant_revision: u64,
    pub(crate) allowed_at_unix_seconds: u64,
    pub(crate) cutoff_at_unix_seconds: u64,
    pub(crate) temporal_commitment: String,
}

pub(crate) struct FrozenE0MemoryUseCommitRequest<'a> {
    pub(crate) published: &'a PublishedFrozenE0MemorySummary,
    pub(crate) replay: &'a VerifiedReplayAvailability,
    pub(crate) scope: &'a MemoryScope,
    pub(crate) access: &'a WikiAccess,
    pub(crate) temporal_commitment: &'a str,
}

/// U33-E atomic re-attestation/write boundary. Durable implementations must
/// bind publication sidecar, current head, revocation and grant in one commit.
#[allow(dead_code)] // Implemented by U33-E now; selected only when runtime composition is wired.
pub(crate) trait FrozenE0MemoryUseCommitPort {
    /// Re-attests the stored U33-E publication identity and explicit revocation
    /// state before later-run page bytes are materialized. A newer current head
    /// alone does not invalidate a still-authorized historical publication.
    fn revalidate_frozen_e0_memory_use(
        &self,
        receipt: &FrozenE0MemoryUseReceipt,
    ) -> Result<(), FrozenE0SummaryPublicationError>;

    fn commit_frozen_e0_memory_use<R, A>(
        &mut self,
        artifacts: &mut R,
        authority: &A,
        request: FrozenE0MemoryUseCommitRequest<'_>,
    ) -> Result<FrozenE0MemoryUseReceipt, FrozenE0SummaryPublicationError>
    where
        R: ArtifactRepository,
        A: WikiAuthorizationPort;
}

#[allow(dead_code)] // Consumed by trusted compositions; exercised in crate tests today.
impl VerifiedFrozenE0MemoryUse {
    pub(crate) fn receipt_id(&self) -> &str {
        &self.receipt.receipt_id
    }

    pub(crate) fn snapshot_ref(&self) -> &crate::ArtifactReference {
        &self.receipt.snapshot_ref
    }

    pub(crate) fn run_id(&self) -> &str {
        &self.receipt.run_id
    }

    pub(crate) fn scope(&self) -> &MemoryScope {
        &self.receipt.scope
    }
}

#[allow(dead_code)] // Composed by the trusted runtime after the persistent adapter is ready.
pub(crate) struct FrozenE0MemoryCycle;

#[allow(dead_code)] // Composed by the trusted runtime after the persistent adapter is ready.
impl FrozenE0MemoryCycle {
    /// Admits a later run only through the U33-E publication record and head.
    /// The current U04-B replay supplies tenant/world/cutoff; the request may
    /// not substitute an independent clock or point at another revision.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_reuse<R, A, P>(
        published: &PublishedFrozenE0MemorySummary,
        replay: &VerifiedReplayAvailability,
        scope: &MemoryScope,
        access: &WikiAccess,
        authority: &A,
        port: &mut P,
        artifacts: &mut R,
    ) -> Result<VerifiedFrozenE0MemoryUse, FrozenE0MemoryCycleError>
    where
        R: ArtifactRepository,
        A: WikiAuthorizationPort,
        P: FrozenE0SummaryPublicationPort + FrozenE0MemoryUseCommitPort,
    {
        let temporal_commitment = attest_frozen_e0_reuse(replay, scope, access)
            .map_err(FrozenE0MemoryCycleError::Temporal)?;
        let receipt = port
            .commit_frozen_e0_memory_use(
                artifacts,
                authority,
                FrozenE0MemoryUseCommitRequest {
                    published,
                    replay,
                    scope,
                    access,
                    temporal_commitment: &temporal_commitment,
                },
            )
            .map_err(FrozenE0MemoryCycleError::Publication)?;
        Ok(VerifiedFrozenE0MemoryUse { receipt })
    }

    /// Reads one page for the admitted run only. The sealed U23-E result binds
    /// the exact run, grant revision, scope, snapshot and clock admitted above;
    /// the ordinary scratch port still rechecks grant liveness at access time.
    #[allow(dead_code)] // Wired by the trusted runtime after the local cycle is exercised.
    pub(crate) fn read_admitted_page<R, P, W>(
        admitted: &VerifiedFrozenE0MemoryUse,
        access: &WikiAccess,
        publication_state: &P,
        scratch: &mut W,
        artifacts: &mut R,
        path: &str,
    ) -> Result<WikiReadResult, FrozenE0MemoryCycleError>
    where
        R: ArtifactRepository,
        P: FrozenE0MemoryUseCommitPort,
        W: WikiScratchPort + WikiAuthorizationPort,
    {
        let receipt = &admitted.receipt;
        let scope = &receipt.scope;
        if access.run_id != receipt.run_id
            || access.tenant_id != scope.tenant_id
            || access.purpose != scope.purpose
            || access.grant_id != receipt.grant_id
            || access.grant_revision != receipt.grant_revision
            || access.snapshot_ref != receipt.snapshot_ref
            || access.allowed_at_unix_seconds != receipt.allowed_at_unix_seconds
            || access.allowed_at_unix_seconds > receipt.cutoff_at_unix_seconds
            || access.memory_scope.world != scope.world
            || access.memory_scope.campaign != scope.campaign
            || access.memory_scope.protocol != scope.protocol
            || access.memory_scope.partition != scope.partition
        {
            return Err(FrozenE0MemoryCycleError::UseAccessMismatch);
        }
        publication_state
            .revalidate_frozen_e0_memory_use(receipt)
            .map_err(FrozenE0MemoryCycleError::Publication)?;
        if !scratch.authorize(access, &receipt.snapshot_ref) {
            return Err(FrozenE0MemoryCycleError::Scratch(
                WikiError::AuthorizationDenied,
            ));
        }
        let workspace = scratch
            .mount(artifacts, access.clone())
            .map_err(FrozenE0MemoryCycleError::Scratch)?;
        let read = scratch
            .read(&workspace, access, path)
            .map_err(FrozenE0MemoryCycleError::Scratch)?;
        if read.snapshot_ref != receipt.snapshot_ref || read.path != path {
            return Err(FrozenE0MemoryCycleError::UseAccessMismatch);
        }
        Ok(read)
    }
}
