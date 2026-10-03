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
use crate::wiki_scratch::{WikiAccess, WikiAuthorizationPort};

#[derive(Debug, Eq, PartialEq)]
#[allow(dead_code)] // The trusted runtime composition is not wired into the demo runner yet.
pub(crate) enum FrozenE0MemoryCycleError {
    Temporal(TemporalProtocolError),
    Publication(FrozenE0SummaryPublicationError),
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
    pub(crate) scope: MemoryScope,
    pub(crate) snapshot_ref: crate::ArtifactReference,
    pub(crate) head_version: u64,
    pub(crate) run_id: String,
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
}
