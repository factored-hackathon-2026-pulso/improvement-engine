//! U18 writes one U17-authorized Core draft into a frozen local registry.
//!
//! It does not call Agent Core, execute a candidate, evaluate it, approve it,
//! publish it, or expose a production registry API. The in-memory adapter is
//! an executable contract for U18's future durable conditional write.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::ArtifactReference;
use crate::change_compiler::{
    CompiledChange, CompilerError, ExecutablePredicate, RegistryWriteMaterial,
};
use crate::core_task::CoreTaskScope;

/// Public marker for U18's boundary. Its construction and mutation are
/// crate-private so a caller cannot turn an arbitrary payload into a candidate.
pub struct GovernedRegistryWriter {
    _private: (),
}

/// ```compile_fail
/// use improvement_engine_core::governed_registry::GovernedRegistryWriter;
/// let _ = GovernedRegistryWriter { _private: () };
/// ```
///
/// The public type is intentionally only a boundary marker. The mutating
/// adapter and its `freeze` operation stay crate-private until a governed
/// service composition owns the durable registry transaction.
const _NO_PUBLIC_REGISTRY_WRITER_CONSTRUCTOR: () = ();

/// Opaque receipt for one frozen candidate. It proves only a U18 local draft
/// write—not execution, evaluation, release, approval or customer exposure.
#[allow(dead_code)] // Receipt fields are consumed by later U19/U21 boundaries.
#[derive(Clone, Debug, PartialEq)]
pub struct FrozenCandidateReceipt {
    candidate_id: String,
    scope: CoreTaskScope,
    source_snapshot: ArtifactReference,
    authorization_commitment: String,
    compiled_commitment: String,
    entity_digest: String,
}

impl FrozenCandidateReceipt {
    #[must_use]
    pub fn candidate_id(&self) -> &str {
        &self.candidate_id
    }
    #[must_use]
    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }
    #[must_use]
    pub fn authorizes_execution_or_release(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryWriteError {
    CompiledChangeInvalid,
    PredicateFailed,
    IdempotencyConflict,
    UnknownCommitOutcome,
}

#[allow(dead_code)] // Frozen registry state is intentionally internal to U18.
#[derive(Clone, Debug)]
struct FrozenEntity {
    version: String,
    content_digest: String,
    draft_digest: String,
    content: serde_json::Value,
}

#[allow(dead_code)] // Internal idempotency row.
#[derive(Clone)]
struct StoredCandidate {
    request_digest: String,
    receipt: FrozenCandidateReceipt,
}

/// Internal deterministic schedule used only to regression-test the final
/// write boundary. It represents a competing durable transaction, never a
/// caller supplied callback.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistryCommitInterleaving {
    OccupyEntity,
    CommitThenUnknown,
    UnknownBeforeCommit,
}

/// The local co-transactional U18 adapter. It owns entity heads, candidate
/// receipts and idempotency state; a future durable adapter must own the same
/// records in one conditional insert/CAS transaction.
#[allow(dead_code)]
#[derive(Default)]
pub(crate) struct InMemoryGovernedRegistryWriter {
    entities: BTreeMap<(String, String, String), FrozenEntity>,
    candidates: BTreeMap<(String, String), StoredCandidate>,
    #[cfg(test)]
    next_interleaving: Option<RegistryCommitInterleaving>,
}

impl InMemoryGovernedRegistryWriter {
    #[cfg(test)]
    pub(crate) fn schedule(&mut self, value: RegistryCommitInterleaving) {
        self.next_interleaving = Some(value);
    }

    #[cfg(test)]
    pub(crate) fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    #[allow(dead_code)] // Called by the crate-private U18 service composition.
    pub(crate) fn freeze(
        &mut self,
        compiled: CompiledChange,
    ) -> Result<FrozenCandidateReceipt, RegistryWriteError> {
        let material = compiled
            .registry_write_material()
            .map_err(|_: CompilerError| RegistryWriteError::CompiledChangeInvalid)?;
        self.freeze_material(material)
    }

    fn freeze_material(
        &mut self,
        material: RegistryWriteMaterial,
    ) -> Result<FrozenCandidateReceipt, RegistryWriteError> {
        let request_digest = request_digest(&material);
        let candidate_key = (
            material.scope.tenant_id().to_owned(),
            material.compiled_commitment.clone(),
        );
        if let Some(existing) = self.candidates.get(&candidate_key) {
            return if existing.request_digest == request_digest {
                Ok(existing.receipt.clone())
            } else {
                Err(RegistryWriteError::IdempotencyConflict)
            };
        }
        #[cfg(test)]
        let interleaving = self.next_interleaving.take();
        #[cfg(test)]
        if matches!(
            interleaving,
            Some(RegistryCommitInterleaving::UnknownBeforeCommit)
        ) {
            return Err(RegistryWriteError::UnknownCommitOutcome);
        }
        let entity_key = (
            material.scope.tenant_id().to_owned(),
            material.kind.as_str().to_owned(),
            material.entity_id.clone(),
        );
        // Final predicate boundary: schedule a competing transaction before
        // evaluating EntityAbsent and before any candidate receipt is inserted.
        #[cfg(test)]
        if matches!(interleaving, Some(RegistryCommitInterleaving::OccupyEntity)) {
            self.entities.insert(
                entity_key.clone(),
                FrozenEntity {
                    version: "0.0.1".to_owned(),
                    content_digest: "sha256:occupied".to_owned(),
                    draft_digest: "sha256:occupied".to_owned(),
                    content: serde_json::json!({"occupied": true}),
                },
            );
        }
        if !predicate_holds(&self.entities, &entity_key, &material.predicate) {
            return Err(RegistryWriteError::PredicateFailed);
        }
        let receipt = FrozenCandidateReceipt {
            candidate_id: format!("candidate:{}", request_digest),
            scope: material.scope.clone(),
            source_snapshot: material.source_snapshot,
            authorization_commitment: material.authorization_commitment,
            compiled_commitment: material.compiled_commitment,
            entity_digest: material.draft_digest.clone(),
        };
        self.entities.insert(
            entity_key,
            FrozenEntity {
                version: material.entity_version,
                content_digest: material.content_digest,
                draft_digest: material.draft_digest,
                content: material.content,
            },
        );
        self.candidates.insert(
            candidate_key,
            StoredCandidate {
                request_digest,
                receipt: receipt.clone(),
            },
        );
        #[cfg(test)]
        if matches!(
            interleaving,
            Some(RegistryCommitInterleaving::CommitThenUnknown)
        ) {
            return Err(RegistryWriteError::UnknownCommitOutcome);
        }
        Ok(receipt)
    }
}

#[allow(dead_code)] // Executed by the crate-private registry writer.
fn predicate_holds(
    entities: &BTreeMap<(String, String, String), FrozenEntity>,
    entity_key: &(String, String, String),
    predicate: &ExecutablePredicate,
) -> bool {
    match predicate {
        ExecutablePredicate::EntityAbsent { kind, entity_id } => {
            entity_key.1 == kind.as_str()
                && entity_key.2 == *entity_id
                && !entities.contains_key(entity_key)
        }
        ExecutablePredicate::RevisionEquals {
            kind,
            entity_id,
            entity_version,
            content_digest,
        } => entities.get(entity_key).is_some_and(|entity| {
            entity_key.1 == kind.as_str()
                && entity_key.2 == *entity_id
                && entity.version == *entity_version
                && entity.content_digest == *content_digest
        }),
    }
}

#[allow(dead_code)] // Executed by the crate-private registry writer.
fn request_digest(material: &RegistryWriteMaterial) -> String {
    let mut hasher = Sha256::new();
    for part in [
        material.scope.tenant_id(),
        material.scope.job_id(),
        material.scope.grant_id(),
        material.scope.authority_ref(),
        &material.source_snapshot.tenant_id,
        &material.source_snapshot.id,
        &material.source_snapshot.revision.to_string(),
        &material.source_snapshot.digest,
        &material.authorization_commitment,
        &material.compiled_commitment,
        &material.entity_id,
        &material.entity_version,
        &material.content_digest,
        &material.draft_digest,
    ] {
        hasher.update(part.len().to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change_compiler::{
        compiled_for_governed_registry_test, corrupt_registry_snapshot_tenant_for_test,
    };

    #[test]
    fn freezes_exact_u17_payload_once_and_replays_the_same_receipt() {
        let mut writer = InMemoryGovernedRegistryWriter::default();
        let first = writer
            .freeze(compiled_for_governed_registry_test("tenant_a"))
            .expect("opaque U17 draft freezes");
        let replay = writer
            .freeze(compiled_for_governed_registry_test("tenant_a"))
            .expect("same authorized intent is idempotent");
        assert_eq!(first.candidate_id(), replay.candidate_id());
        assert_eq!(writer.candidate_count(), 1);
        assert!(!first.authorizes_execution_or_release());
        let entity = writer.entities.values().next().expect("frozen entity");
        assert_eq!(entity.content["id"], "payment_status_resolution");
    }

    #[test]
    fn stale_entity_absent_predicate_at_final_boundary_leaves_no_candidate() {
        let mut writer = InMemoryGovernedRegistryWriter::default();
        writer.schedule(RegistryCommitInterleaving::OccupyEntity);
        assert_eq!(
            writer.freeze(compiled_for_governed_registry_test("tenant_a")),
            Err(RegistryWriteError::PredicateFailed)
        );
        assert_eq!(writer.candidate_count(), 0);
    }

    #[test]
    fn crash_unknown_never_claims_success_and_retries_are_truthful() {
        let mut writer = InMemoryGovernedRegistryWriter::default();
        writer.schedule(RegistryCommitInterleaving::UnknownBeforeCommit);
        assert_eq!(
            writer.freeze(compiled_for_governed_registry_test("tenant_a")),
            Err(RegistryWriteError::UnknownCommitOutcome)
        );
        assert_eq!(writer.candidate_count(), 0);
        let receipt = writer
            .freeze(compiled_for_governed_registry_test("tenant_a"))
            .expect("retry after pre-commit crash writes once");
        assert_eq!(writer.candidate_count(), 1);

        let mut lost_response = InMemoryGovernedRegistryWriter::default();
        lost_response.schedule(RegistryCommitInterleaving::CommitThenUnknown);
        assert_eq!(
            lost_response.freeze(compiled_for_governed_registry_test("tenant_a")),
            Err(RegistryWriteError::UnknownCommitOutcome)
        );
        assert_eq!(lost_response.candidate_count(), 1);
        let replay = lost_response
            .freeze(compiled_for_governed_registry_test("tenant_a"))
            .expect("retry resolves committed idempotent receipt");
        assert_eq!(receipt.candidate_id(), replay.candidate_id());
    }

    #[test]
    fn tenant_scopes_do_not_share_entity_or_idempotency_state() {
        let mut writer = InMemoryGovernedRegistryWriter::default();
        let a = writer
            .freeze(compiled_for_governed_registry_test("tenant_a"))
            .expect("tenant a freezes its candidate");
        let b = writer
            .freeze(compiled_for_governed_registry_test("tenant_b"))
            .expect("tenant b cannot collide with tenant a");
        assert_ne!(a.scope().tenant_id(), b.scope().tenant_id());
        assert_eq!(writer.candidate_count(), 2);
    }

    #[test]
    fn registry_revalidates_u17_tenant_snapshot_binding_before_any_write() {
        let mut writer = InMemoryGovernedRegistryWriter::default();
        let corrupt = corrupt_registry_snapshot_tenant_for_test(
            compiled_for_governed_registry_test("tenant_a"),
            "tenant_b",
        );
        assert_eq!(
            writer.freeze(corrupt),
            Err(RegistryWriteError::CompiledChangeInvalid)
        );
        assert_eq!(writer.candidate_count(), 0);
        assert!(writer.entities.is_empty());
    }

    #[test]
    fn conflicting_retry_for_the_same_compiled_key_is_rejected_fail_closed() {
        let compiled = compiled_for_governed_registry_test("tenant_a");
        let material = compiled
            .registry_write_material()
            .expect("fixed fixture produces verified material");
        let mut conflicting = material.clone();
        // This is an internal durability regression: a durable idempotency key
        // must never accept distinct write material under the same commitment.
        conflicting.entity_version = "1.0.1".to_owned();
        let mut writer = InMemoryGovernedRegistryWriter::default();
        writer
            .freeze_material(material)
            .expect("first exact write freezes");
        assert_eq!(
            writer.freeze_material(conflicting),
            Err(RegistryWriteError::IdempotencyConflict)
        );
        assert_eq!(writer.candidate_count(), 1);
    }
}
