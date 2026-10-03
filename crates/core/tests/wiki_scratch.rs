use std::collections::BTreeMap;

use improvement_engine_core::wiki_scratch::{
    InMemoryWikiGrantAuthority, WikiAccess, WikiError, WikiGrant, WikiScratchPort, WikiTransform,
    WikiTransformOperation, WikiWorkspace,
};
use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository,
};
use serde_json::json;

const TENANT: &str = "tenant-a";
const WIKI_ID: &str = "018f50a1-7f00-7000-8000-000000000001";

fn snapshot(available_at: u64) -> ArtifactDraft {
    ArtifactDraft::new(
        TENANT,
        WIKI_ID,
        1,
        ArtifactKind::MemoryWiki,
        json!({
            "available_at_unix_seconds": available_at,
            "purpose": "investigation",
            "pages": {
                "payments/disputes.md": "verify transaction state before escalation",
                "index.md": "# Investigation wiki"
            }
        }),
        None,
    )
}

fn authority(reference: &improvement_engine_core::ArtifactReference) -> InMemoryWikiGrantAuthority {
    authority_for(reference, "run-1")
}

fn authority_for(
    reference: &improvement_engine_core::ArtifactReference,
    run_id: &str,
) -> InMemoryWikiGrantAuthority {
    let authority = InMemoryWikiGrantAuthority::default();
    authority.issue(WikiGrant::new(
        "grant-1",
        run_id,
        TENANT,
        "investigation",
        reference.clone(),
    ));
    authority
}

fn access(
    reference: &improvement_engine_core::ArtifactReference,
    allowed_clock: u64,
) -> WikiAccess {
    WikiAccess::new(
        "run-1",
        TENANT,
        "investigation",
        "grant-1",
        reference.clone(),
        allowed_clock,
    )
}

fn mounted_workspace(available_at: u64) -> (WikiWorkspace, InMemoryWikiGrantAuthority, WikiAccess) {
    let mut repository = InMemoryArtifactRepository::default();
    let stored = repository.append(None, snapshot(available_at)).unwrap();
    let mut authority = authority(&stored.reference());
    let request = access(&stored.reference(), available_at);
    let workspace = authority.mount(&mut repository, request.clone()).unwrap();
    (workspace, authority, request)
}

#[test]
fn authorized_digest_addressed_snapshot_mounts_and_transforms_without_mutating_source() {
    let mut repository = InMemoryArtifactRepository::default();
    let stored = repository.append(None, snapshot(100)).unwrap();
    let request = access(&stored.reference(), 100);
    let mut authority = authority(&stored.reference());

    let mut workspace = authority.mount(&mut repository, request.clone()).unwrap();
    assert_eq!(
        authority
            .read(&workspace, &request, "payments/disputes.md")
            .unwrap()
            .content,
        "verify transaction state before escalation"
    );

    let result = authority
        .transform(
            &mut workspace,
            &request,
            WikiTransform::new(vec![WikiTransformOperation::replace(
                "payments/disputes.md",
                "verify transaction and contact timeline before escalation",
            )]),
        )
        .unwrap();

    assert_eq!(
        result.pages["payments/disputes.md"],
        "verify transaction and contact timeline before escalation"
    );
    assert_eq!(result.receipt.snapshot_ref, stored.reference());
    assert_eq!(result.receipt.workspace_id, workspace.workspace_id());
    assert_eq!(
        repository.get(TENANT, WIKI_ID, 1).unwrap().unwrap().payload["pages"]["payments/disputes.md"],
        "verify transaction state before escalation"
    );
}

#[test]
fn workspace_debug_redacts_page_content() {
    let (workspace, _, _) = mounted_workspace(100);
    let debug = format!("{workspace:?}");

    assert!(
        debug.contains("page_count: 2"),
        "debug output should retain useful page-count metadata"
    );
    assert!(
        !debug.contains(workspace.workspace_id()),
        "debug output must not expose the stable workspace correlation handle"
    );
    assert!(
        !debug.contains("verify transaction state before escalation"),
        "debug formatting must not bypass the authorized content-read boundary"
    );
    assert!(!debug.contains("# Investigation wiki"));
}

#[test]
fn expired_memory_is_not_mounted_at_or_after_its_exclusive_expiry_boundary() {
    let mut repository = InMemoryArtifactRepository::default();
    let stored = repository
        .append(
            None,
            ArtifactDraft::new(
                TENANT,
                WIKI_ID,
                1,
                ArtifactKind::MemoryWiki,
                json!({
                    "available_at_unix_seconds": 100,
                    "expires_at_unix_seconds": 150,
                    "purpose": "investigation",
                    "pages": {"index.md": "must not be served after expiry"}
                }),
                None,
            ),
        )
        .unwrap();

    let mut authority = authority(&stored.reference());
    let mut request = access(&stored.reference(), 149);
    let mut workspace = authority
        .mount(&mut repository, request.clone())
        .expect("access strictly before expiry is allowed");
    assert_eq!(
        authority
            .read(&workspace, &request, "index.md")
            .unwrap()
            .content,
        "must not be served after expiry"
    );
    assert!(
        authority
            .transform(
                &mut workspace,
                &request,
                WikiTransform::new(vec![WikiTransformOperation::replace(
                    "index.md",
                    "edited before expiry",
                )]),
            )
            .is_ok()
    );
    request.allowed_at_unix_seconds = 150;
    assert_eq!(
        authority.read(&workspace, &request, "index.md"),
        Err(WikiError::SnapshotExpired {
            expires_at_unix_seconds: 150,
            allowed_at_unix_seconds: 150,
        })
    );
    assert_eq!(
        workspace.pages(&authority, &request),
        Err(WikiError::SnapshotExpired {
            expires_at_unix_seconds: 150,
            allowed_at_unix_seconds: 150,
        }),
        "the public pages accessor must not reveal mounted content after expiry"
    );
    assert_eq!(
        authority.transform(
            &mut workspace,
            &request,
            WikiTransform::new(vec![WikiTransformOperation::replace(
                "index.md",
                "must not edit after expiry",
            )]),
        ),
        Err(WikiError::SnapshotExpired {
            expires_at_unix_seconds: 150,
            allowed_at_unix_seconds: 150,
        })
    );

    for allowed_clock in [150, 151] {
        let mut authority = authority_for(&stored.reference(), "run-1");
        let request = access(&stored.reference(), allowed_clock);
        assert_eq!(
            authority.mount(&mut repository, request),
            Err(WikiError::SnapshotExpired {
                expires_at_unix_seconds: 150,
                allowed_at_unix_seconds: allowed_clock,
            }),
            "access at or after expiry must not expose pages or allow a later publish"
        );
    }
}

#[test]
fn memory_expiry_uses_the_supplied_as_of_value_not_an_authenticated_wall_clock() {
    let mut repository = InMemoryArtifactRepository::default();
    let stored = repository
        .append(
            None,
            ArtifactDraft::new(
                TENANT,
                WIKI_ID,
                1,
                ArtifactKind::MemoryWiki,
                json!({
                    "available_at_unix_seconds": 100,
                    "expires_at_unix_seconds": 150,
                    "purpose": "investigation",
                    "pages": {"index.md": "replay-scoped memory"}
                }),
                None,
            ),
        )
        .unwrap();
    let mut authority = authority(&stored.reference());
    let original_access = access(&stored.reference(), 149);
    let workspace = authority
        .mount(&mut repository, original_access.clone())
        .unwrap();

    let mut at_expiry = original_access.clone();
    at_expiry.allowed_at_unix_seconds = 150;
    assert!(matches!(
        workspace.pages(&authority, &at_expiry),
        Err(WikiError::SnapshotExpired { .. })
    ));

    // Characterize the adapter boundary: WikiAccess is caller-constructible
    // and this local authority has no authenticated clock to reject backdating.
    let mut backdated = at_expiry;
    backdated.allowed_at_unix_seconds = 149;
    assert_eq!(
        workspace.pages(&authority, &backdated).unwrap()["index.md"],
        "replay-scoped memory",
        "the caller's timestamp is policy input, not proof of current wall-clock time"
    );
}

#[test]
fn memory_expiry_must_follow_snapshot_availability() {
    let mut repository = InMemoryArtifactRepository::default();
    let invalid = ArtifactDraft::new(
        TENANT,
        WIKI_ID,
        1,
        ArtifactKind::MemoryWiki,
        json!({
            "available_at_unix_seconds": 100,
            "expires_at_unix_seconds": 100,
            "purpose": "investigation",
            "pages": {"index.md": "invalid retention interval"}
        }),
        None,
    );
    let stored = repository.append(None, invalid).unwrap();
    let mut authority = authority(&stored.reference());

    assert_eq!(
        authority.mount(&mut repository, access(&stored.reference(), 100)),
        Err(WikiError::SnapshotPayloadInvalid)
    );
}

#[test]
fn changing_memory_expiry_changes_the_immutable_artifact_digest() {
    let reference_for_expiry = |expires_at_unix_seconds| {
        let mut repository = InMemoryArtifactRepository::default();
        repository
            .append(
                None,
                ArtifactDraft::new(
                    TENANT,
                    WIKI_ID,
                    1,
                    ArtifactKind::MemoryWiki,
                    json!({
                        "available_at_unix_seconds": 100,
                        "expires_at_unix_seconds": expires_at_unix_seconds,
                        "purpose": "investigation",
                        "pages": {"index.md": "same page"}
                    }),
                    None,
                ),
            )
            .unwrap()
            .reference()
    };

    assert_ne!(
        reference_for_expiry(150).digest,
        reference_for_expiry(151).digest
    );
}

#[test]
fn read_denies_cross_tenant_wrong_purpose_and_unapproved_grant() {
    let (workspace, authority, request) = mounted_workspace(100);

    let tenant = WikiAccess::new(
        "run-1",
        "tenant-b",
        "investigation",
        "grant-1",
        request.snapshot_ref.clone(),
        100,
    );
    assert_eq!(
        authority.read(&workspace, &tenant, "index.md").unwrap_err(),
        WikiError::AuthorizationDenied
    );
    let purpose = WikiAccess::new(
        "run-1",
        TENANT,
        "evaluation",
        "grant-1",
        request.snapshot_ref.clone(),
        100,
    );
    assert_eq!(
        authority
            .read(&workspace, &purpose, "index.md")
            .unwrap_err(),
        WikiError::AuthorizationDenied
    );
    let grant = WikiAccess::new(
        "run-1",
        TENANT,
        "investigation",
        "grant-unknown",
        request.snapshot_ref.clone(),
        100,
    );
    assert_eq!(
        authority.read(&workspace, &grant, "index.md").unwrap_err(),
        WikiError::AuthorizationDenied
    );
}

#[test]
fn future_snapshot_is_denied_before_any_content_is_read() {
    let mut repository = InMemoryArtifactRepository::default();
    let stored = repository.append(None, snapshot(101)).unwrap();
    let mut authority = authority(&stored.reference());
    let request = access(&stored.reference(), 100);

    assert_eq!(
        authority.mount(&mut repository, request).unwrap_err(),
        WikiError::FutureSnapshot {
            available_at_unix_seconds: 101,
            allowed_at_unix_seconds: 100,
        }
    );
}

#[test]
fn tampered_digest_and_unapproved_reference_are_rejected() {
    let mut repository = InMemoryArtifactRepository::default();
    let stored = repository.append(None, snapshot(100)).unwrap();
    let mut authority = authority(&stored.reference());
    let mut tampered = stored.reference();
    tampered.digest =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned();
    let request = WikiAccess::new("run-1", TENANT, "investigation", "grant-1", tampered, 100);

    assert_eq!(
        authority.mount(&mut repository, request).unwrap_err(),
        WikiError::AuthorizationDenied
    );
}

#[test]
fn traversal_and_failed_batch_leave_scratch_unchanged() {
    let (mut workspace, authority, request) = mounted_workspace(100);
    let before: BTreeMap<_, _> = workspace.pages(&authority, &request).unwrap().clone();

    assert_eq!(
        authority
            .transform(
                &mut workspace,
                &request,
                WikiTransform::new(vec![WikiTransformOperation::create("../escape.md", "no")]),
            )
            .unwrap_err(),
        WikiError::InvalidPath {
            path: "../escape.md".to_owned()
        }
    );
    assert_eq!(workspace.pages(&authority, &request).unwrap(), &before);

    for path in ["x:relative.md", "\\\\server\\share.md"] {
        assert_eq!(
            authority
                .transform(
                    &mut workspace,
                    &request,
                    WikiTransform::new(vec![WikiTransformOperation::create(path, "no")]),
                )
                .unwrap_err(),
            WikiError::InvalidPath {
                path: path.to_owned()
            }
        );
        assert_eq!(workspace.pages(&authority, &request).unwrap(), &before);
    }

    assert_eq!(
        authority
            .transform(
                &mut workspace,
                &request,
                WikiTransform::new(vec![WikiTransformOperation::create("C:/escape.md", "no")]),
            )
            .unwrap_err(),
        WikiError::InvalidPath {
            path: "C:/escape.md".to_owned()
        }
    );
    assert_eq!(workspace.pages(&authority, &request).unwrap(), &before);

    assert_eq!(
        authority
            .transform(
                &mut workspace,
                &request,
                WikiTransform::new(vec![
                    WikiTransformOperation::replace("index.md", "changed"),
                    WikiTransformOperation::replace("missing.md", "fails"),
                ]),
            )
            .unwrap_err(),
        WikiError::PageMissing {
            path: "missing.md".to_owned()
        }
    );
    assert_eq!(workspace.pages(&authority, &request).unwrap(), &before);
}

#[test]
fn create_collision_in_a_later_operation_is_atomic_and_leaves_workspace_identical() {
    let (mut workspace, authority, request) = mounted_workspace(100);
    let before = workspace.pages(&authority, &request).unwrap().clone();

    let error = authority
        .transform(
            &mut workspace,
            &request,
            WikiTransform::new(vec![
                WikiTransformOperation::create("prepared/frozen-e0-summary.md", "first write"),
                WikiTransformOperation::create("index.md", "must collide"),
            ]),
        )
        .unwrap_err();

    assert_eq!(
        error,
        WikiError::PageAlreadyExists {
            path: "index.md".to_owned()
        }
    );
    assert_eq!(
        workspace.pages(&authority, &request).unwrap(),
        &before,
        "a failed second Create must not leak the first mutation into scratch"
    );
    assert!(
        !workspace
            .pages(&authority, &request)
            .unwrap()
            .contains_key("prepared/frozen-e0-summary.md")
    );
}

#[test]
fn equivalent_transforms_have_deterministic_result_and_receipt_digests() {
    let (mut first, authority, request) = mounted_workspace(100);
    let (mut second, _, _) = mounted_workspace(100);
    let transform = WikiTransform::new(vec![WikiTransformOperation::create("notes.md", "same")]);

    let first = authority
        .transform(&mut first, &request, transform.clone())
        .unwrap();
    let second = authority
        .transform(&mut second, &request, transform)
        .unwrap();
    assert_eq!(first.result_digest, second.result_digest);
    assert_eq!(
        first.receipt.transform_digest,
        second.receipt.transform_digest
    );
}

#[test]
fn every_read_rechecks_the_temporal_boundary_and_workspaces_are_run_isolated() {
    let (mut first, mut first_authority, request) = mounted_workspace(100);
    let mut repository = InMemoryArtifactRepository::default();
    let stored = repository.append(None, snapshot(100)).unwrap();
    let mut second_authority = authority_for(&stored.reference(), "run-2");
    let second_request = WikiAccess::new(
        "run-2",
        TENANT,
        "investigation",
        "grant-1",
        stored.reference(),
        100,
    );
    let second = second_authority
        .mount(&mut repository, second_request.clone())
        .unwrap();

    assert_ne!(first.workspace_id(), second.workspace_id());
    let stale_clock = WikiAccess::new(
        "run-1",
        TENANT,
        "investigation",
        "grant-1",
        request.snapshot_ref.clone(),
        99,
    );
    assert_eq!(
        first_authority
            .read(&first, &stale_clock, "index.md")
            .unwrap_err(),
        WikiError::FutureSnapshot {
            available_at_unix_seconds: 100,
            allowed_at_unix_seconds: 99,
        }
    );
    first_authority
        .transform(
            &mut first,
            &request,
            WikiTransform::new(vec![WikiTransformOperation::create("run-1.md", "private")]),
        )
        .unwrap();
    assert_eq!(
        second_authority
            .read(&second, &second_request, "run-1.md")
            .unwrap_err(),
        WikiError::PageMissing {
            path: "run-1.md".to_owned()
        }
    );
    assert_eq!(
        second_authority
            .read(&first, &second_request, "index.md")
            .unwrap_err(),
        WikiError::WorkspaceAccessDenied
    );
    assert_eq!(
        second_authority
            .transform(
                &mut first,
                &second_request,
                WikiTransform::new(vec![WikiTransformOperation::create("also-denied.md", "no")]),
            )
            .unwrap_err(),
        WikiError::WorkspaceAccessDenied
    );

    let cross_run = WikiAccess::new(
        "run-2",
        TENANT,
        "investigation",
        "grant-1",
        request.snapshot_ref.clone(),
        100,
    );
    assert_eq!(
        first_authority
            .read(&first, &cross_run, "index.md")
            .unwrap_err(),
        WikiError::AuthorizationDenied
    );
    assert_eq!(
        first_authority
            .transform(
                &mut first,
                &cross_run,
                WikiTransform::new(vec![WikiTransformOperation::create("denied.md", "no")]),
            )
            .unwrap_err(),
        WikiError::AuthorizationDenied
    );
    let mut third_repository = InMemoryArtifactRepository::default();
    let third_snapshot = third_repository.append(None, snapshot(100)).unwrap();
    let denied_mount = WikiAccess::new(
        "run-2",
        TENANT,
        "investigation",
        "grant-1",
        third_snapshot.reference(),
        100,
    );
    assert_eq!(
        first_authority
            .mount(&mut third_repository, denied_mount)
            .unwrap_err(),
        WikiError::AuthorizationDenied
    );
}
