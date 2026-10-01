use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository, InMemoryArtifactRepository,
    RepositoryError,
};
use serde_json::json;

const TENANT_A: &str = "tenant-a";
const TENANT_B: &str = "tenant-b";
const SOURCE_ID: &str = "018f0f4e-7bbd-7000-8000-000000000001";
const SIGNAL_ID: &str = "018f0f4e-7bbd-7000-8000-000000000002";

fn source_snapshot() -> ArtifactDraft {
    ArtifactDraft::new(
        TENANT_A,
        SOURCE_ID,
        1,
        ArtifactKind::SourceSnapshot,
        json!({"cutoff": "2026-01-01T00:00:00Z", "table": "contacts"}),
        None,
    )
}

fn signal(source_snapshot_ref: ArtifactReference) -> ArtifactDraft {
    ArtifactDraft::new(
        TENANT_A,
        SIGNAL_ID,
        1,
        ArtifactKind::Signal,
        json!({"metric": "repeat_contact_rate", "value": 0.31}),
        Some(source_snapshot_ref),
    )
}

#[test]
fn appends_and_reads_back_an_immutable_revision() {
    let mut repository = InMemoryArtifactRepository::default();
    let source = repository.append(None, source_snapshot()).unwrap();
    let stored = repository.append(None, signal(source.reference())).unwrap();

    assert_eq!(stored.revision, 1);
    assert_eq!(
        repository
            .get(TENANT_A, SIGNAL_ID, 1)
            .unwrap()
            .expect("stored revision"),
        stored
    );
}

#[test]
fn rejects_a_revision_when_the_compare_and_swap_head_is_stale() {
    let mut repository = InMemoryArtifactRepository::default();
    let first = repository.append(None, source_snapshot()).unwrap();
    let second = ArtifactDraft::new(
        TENANT_A,
        SOURCE_ID,
        2,
        ArtifactKind::SourceSnapshot,
        json!({"cutoff": "2026-01-02T00:00:00Z", "table": "contacts"}),
        None,
    );

    let error = repository.append(Some(0), second).unwrap_err();

    assert_eq!(
        error,
        RepositoryError::RevisionConflict {
            artifact_id: first.id,
            expected_head: Some(0),
            actual_head: Some(1),
        }
    );
}

#[test]
fn rejects_a_tampered_digest_before_persisting() {
    let mut repository = InMemoryArtifactRepository::default();
    let mut draft = source_snapshot();
    draft.digest = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into();

    assert!(matches!(
        repository.append(None, draft),
        Err(RepositoryError::DigestMismatch { .. })
    ));
}

#[test]
fn prevents_cross_tenant_readback() {
    let mut repository = InMemoryArtifactRepository::default();
    repository.append(None, source_snapshot()).unwrap();

    assert_eq!(repository.get(TENANT_B, SOURCE_ID, 1).unwrap(), None);
}

#[test]
fn accepts_a_source_snapshot_reference_only_when_it_resolves_to_that_kind() {
    let mut repository = InMemoryArtifactRepository::default();
    let source = repository.append(None, source_snapshot()).unwrap();
    repository.append(None, signal(source.reference())).unwrap();
}

#[test]
fn rejects_a_source_snapshot_reference_to_a_non_snapshot_artifact() {
    let mut repository = InMemoryArtifactRepository::default();
    let non_snapshot = repository
        .append(
            None,
            ArtifactDraft::new(
                TENANT_A,
                SOURCE_ID,
                1,
                ArtifactKind::Signal,
                json!({"metric": "irrelevant"}),
                None,
            ),
        )
        .unwrap();

    assert!(matches!(
        repository.append(None, signal(non_snapshot.reference())),
        Err(RepositoryError::SourceSnapshotKindMismatch { .. })
    ));
}

#[test]
fn rejects_a_source_snapshot_reference_with_a_different_digest() {
    let mut repository = InMemoryArtifactRepository::default();
    let mut source_ref = repository
        .append(None, source_snapshot())
        .unwrap()
        .reference();
    source_ref.digest =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();

    assert!(matches!(
        repository.append(None, signal(source_ref)),
        Err(RepositoryError::SourceSnapshotReferenceMismatch { .. })
    ));
}

#[test]
fn rejects_a_source_snapshot_reference_from_another_tenant() {
    let mut repository = InMemoryArtifactRepository::default();
    let mut source_ref = repository
        .append(None, source_snapshot())
        .unwrap()
        .reference();
    source_ref.tenant_id = TENANT_B.into();

    assert!(matches!(
        repository.append(None, signal(source_ref)),
        Err(RepositoryError::CrossTenantReference { .. })
    ));
}
