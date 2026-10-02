use improvement_engine_core::memory_store::{
    InMemoryMemoryRegistry, MemoryError, MemoryPublishRequest, MemoryPublisher, MemoryScope,
};
use improvement_engine_core::wiki_scratch::{
    InMemoryWikiGrantAuthority, MemoryScopeBinding, WikiAccess, WikiGrant, WikiScratchPort,
    WikiTransform, WikiTransformOperation,
};
use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository,
};
use serde_json::json;

const TENANT: &str = "tenant-a";
const WIKI_ID: &str = "018f50a1-7f00-7000-8000-000000000001";

fn initial_wiki() -> ArtifactDraft {
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
    )
}

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

fn access(reference: improvement_engine_core::ArtifactReference) -> WikiAccess {
    WikiAccess::new_scoped(
        "run-1",
        TENANT,
        "investigation",
        "grant-1",
        reference,
        100,
        MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train"),
    )
}

fn authority(reference: improvement_engine_core::ArtifactReference) -> InMemoryWikiGrantAuthority {
    let mut authority = InMemoryWikiGrantAuthority::default();
    authority.issue(WikiGrant::new_scoped(
        "grant-1",
        "run-1",
        TENANT,
        "investigation",
        reference,
        MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train"),
    ));
    authority
}

#[test]
fn publishes_verified_scratch_diff_by_scoped_cas_and_records_allowed_use() {
    let mut artifacts = InMemoryArtifactRepository::default();
    let initial = artifacts.append(None, initial_wiki()).unwrap();
    let request_access = access(initial.reference());
    let mut authority = authority(initial.reference());
    let mut workspace = authority
        .mount(&mut artifacts, request_access.clone())
        .unwrap();
    let diff = authority
        .transform(
            &mut workspace,
            &request_access,
            WikiTransform::new(vec![WikiTransformOperation::replace(
                "index.md",
                "published",
            )]),
        )
        .unwrap();
    let mut registry = InMemoryMemoryRegistry::default();
    registry
        .seed_head(&mut artifacts, scope(), initial.reference())
        .unwrap();

    let published = registry
        .publish(
            &mut artifacts,
            &authority,
            MemoryPublishRequest::new(scope(), 1, request_access.clone(), diff),
        )
        .unwrap();

    assert_eq!(published.head.head_version, 2);
    assert_eq!(published.snapshot.payload["pages"]["index.md"], "published");
    assert_eq!(
        artifacts.get(TENANT, WIKI_ID, 1).unwrap().unwrap().payload["pages"]["index.md"],
        "original"
    );
    let published_access = WikiAccess::new_scoped(
        "run-2",
        TENANT,
        "investigation",
        "grant-2",
        published.snapshot.reference(),
        100,
        MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train"),
    );
    authority.issue(WikiGrant::new_scoped(
        "grant-2",
        "run-2",
        TENANT,
        "investigation",
        published.snapshot.reference(),
        MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train"),
    ));
    let use_receipt = registry
        .record_allowed_use(
            &mut artifacts,
            &authority,
            scope(),
            published_access.clone(),
            None,
            published.snapshot.reference(),
        )
        .unwrap();
    let duplicate = registry
        .record_allowed_use(
            &mut artifacts,
            &authority,
            scope(),
            published_access,
            None,
            published.snapshot.reference(),
        )
        .unwrap();
    assert_eq!(use_receipt.snapshot_ref, published.snapshot.reference());
    assert_eq!(use_receipt.head_version, 2);
    assert_eq!(use_receipt, duplicate);
    assert_eq!(registry.receipts().len(), 1);
}

#[test]
fn stale_publish_does_not_overwrite_newer_learning_and_requires_explicit_rebase() {
    let mut artifacts = InMemoryArtifactRepository::default();
    let initial = artifacts.append(None, initial_wiki()).unwrap();
    let request_access = access(initial.reference());
    let mut authority = authority(initial.reference());
    let mut workspace = authority
        .mount(&mut artifacts, request_access.clone())
        .unwrap();
    let diff = authority
        .transform(
            &mut workspace,
            &request_access,
            WikiTransform::new(vec![WikiTransformOperation::replace("index.md", "first")]),
        )
        .unwrap();
    let mut registry = InMemoryMemoryRegistry::default();
    registry
        .seed_head(&mut artifacts, scope(), initial.reference())
        .unwrap();
    registry
        .publish(
            &mut artifacts,
            &authority,
            MemoryPublishRequest::new(scope(), 1, request_access.clone(), diff.clone()),
        )
        .unwrap();

    assert_eq!(
        registry
            .publish(
                &mut artifacts,
                &authority,
                MemoryPublishRequest::new(scope(), 1, request_access, diff)
            )
            .unwrap_err(),
        MemoryError::HeadConflict {
            expected: 1,
            actual: 2
        }
    );
}

#[test]
fn tombstone_blocks_new_use_and_survives_registry_restore() {
    let mut artifacts = InMemoryArtifactRepository::default();
    let initial = artifacts.append(None, initial_wiki()).unwrap();
    let request_access = access(initial.reference());
    let authority = authority(initial.reference());
    let mut registry = InMemoryMemoryRegistry::default();
    registry
        .seed_head(&mut artifacts, scope(), initial.reference())
        .unwrap();
    registry
        .revoke(initial.reference(), "source_permission_revoked")
        .unwrap();

    let mut restored = registry.clone();
    assert_eq!(
        restored
            .record_allowed_use(
                &mut artifacts,
                &authority,
                scope(),
                request_access,
                None,
                initial.reference()
            )
            .unwrap_err(),
        MemoryError::SnapshotRevoked
    );
}

#[test]
fn publication_rejects_a_tampered_result_and_cross_scope_head() {
    let mut artifacts = InMemoryArtifactRepository::default();
    let initial = artifacts.append(None, initial_wiki()).unwrap();
    let request_access = access(initial.reference());
    let mut authority = authority(initial.reference());
    let mut workspace = authority
        .mount(&mut artifacts, request_access.clone())
        .unwrap();
    let mut result = authority
        .transform(
            &mut workspace,
            &request_access,
            WikiTransform::new(vec![WikiTransformOperation::replace("index.md", "safe")]),
        )
        .unwrap();
    let mut registry = InMemoryMemoryRegistry::default();
    registry
        .seed_head(&mut artifacts, scope(), initial.reference())
        .unwrap();

    result
        .pages
        .insert("../host.md".to_owned(), "escape".to_owned());
    assert_eq!(
        registry
            .publish(
                &mut artifacts,
                &authority,
                MemoryPublishRequest::new(scope(), 1, request_access.clone(), result),
            )
            .unwrap_err(),
        MemoryError::TransformDigestMismatch
    );

    let other_scope = MemoryScope::new(
        TENANT,
        "investigation",
        "world-b",
        "campaign-a",
        "continuous",
        "train",
    );
    assert_eq!(
        registry
            .seed_head(&mut artifacts, other_scope, initial.reference())
            .unwrap_err(),
        MemoryError::SnapshotAlreadyBound
    );
}

#[test]
fn ancestor_tombstone_revokes_descendant_snapshot_and_wrong_scope_receipt() {
    let mut artifacts = InMemoryArtifactRepository::default();
    let initial = artifacts.append(None, initial_wiki()).unwrap();
    let request_access = access(initial.reference());
    let mut authority = authority(initial.reference());
    let mut workspace = authority
        .mount(&mut artifacts, request_access.clone())
        .unwrap();
    let result = authority
        .transform(
            &mut workspace,
            &request_access,
            WikiTransform::new(vec![WikiTransformOperation::replace("index.md", "child")]),
        )
        .unwrap();
    let mut registry = InMemoryMemoryRegistry::default();
    registry
        .seed_head(&mut artifacts, scope(), initial.reference())
        .unwrap();
    let published = registry
        .publish(
            &mut artifacts,
            &authority,
            MemoryPublishRequest::new(scope(), 1, request_access.clone(), result),
        )
        .unwrap();
    let child_access = WikiAccess::new_scoped(
        "run-2",
        TENANT,
        "investigation",
        "grant-2",
        published.snapshot.reference(),
        100,
        MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train"),
    );
    authority.issue(WikiGrant::new_scoped(
        "grant-2",
        "run-2",
        TENANT,
        "investigation",
        published.snapshot.reference(),
        MemoryScopeBinding::new("world-a", "campaign-a", "continuous", "train"),
    ));
    registry
        .revoke(initial.reference(), "source_permission_revoked")
        .unwrap();
    assert_eq!(
        registry
            .record_allowed_use(
                &mut artifacts,
                &authority,
                scope(),
                child_access,
                None,
                published.snapshot.reference(),
            )
            .unwrap_err(),
        MemoryError::SnapshotRevoked
    );
}

#[test]
fn caller_cannot_relabel_a_granted_scope_in_transform_receipt() {
    let mut artifacts = InMemoryArtifactRepository::default();
    let initial = artifacts.append(None, initial_wiki()).unwrap();
    let request_access = access(initial.reference());
    let mut authority = authority(initial.reference());
    let mut workspace = authority
        .mount(&mut artifacts, request_access.clone())
        .unwrap();
    let mut result = authority
        .transform(
            &mut workspace,
            &request_access,
            WikiTransform::new(vec![WikiTransformOperation::replace(
                "index.md",
                "wrong-world",
            )]),
        )
        .unwrap();
    let mut registry = InMemoryMemoryRegistry::default();
    registry
        .seed_head(&mut artifacts, scope(), initial.reference())
        .unwrap();
    result.receipt.memory_scope =
        MemoryScopeBinding::new("world-b", "campaign-a", "continuous", "train");
    assert_eq!(
        registry
            .publish(
                &mut artifacts,
                &authority,
                MemoryPublishRequest::new(scope(), 1, request_access, result),
            )
            .unwrap_err(),
        MemoryError::AccessDenied
    );
}
