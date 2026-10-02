use improvement_engine_core::run_fork::{
    ForkAuthorization, ForkCommitPort, ForkControlState, ForkRequest, InMemoryForkCommitPort,
    RunForkError, RunLifecycle,
};
use improvement_engine_core::{ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository};
use serde_json::json;

const TENANT: &str = "demo";
const OTHER: &str = "other";
const SOURCE: &str = "018f0f4e-7bbd-7000-8000-000000000011";
const CONFIG: &str = "018f0f4e-7bbd-7000-8000-000000000012";
const MEMORY: &str = "018f0f4e-7bbd-7000-8000-000000000013";

#[test]
fn atomic_port_copies_attested_artifacts_and_emits_one_audit_receipt() {
    let mut port = fixture();
    let receipt = port
        .commit_conditionally(request("fork_001", "reason"))
        .unwrap();
    assert_eq!(receipt.fork().replay_of(), "run_original");
    assert_eq!(receipt.fork().snapshot_ref(), &source_ref(&mut port));
    assert_eq!(receipt.fork().cutoff_unix_seconds(), 100);
    assert_eq!(receipt.event().reason(), "reason");
    assert_eq!(port.fork_count(TENANT), 1);
    assert_eq!(port.audit_count(TENANT), 1);
}

#[test]
fn retry_returns_same_receipt_but_rechecks_revocation_without_second_effect() {
    let mut port = fixture();
    let request = request("fork_retry", "reason");
    let first = port.commit_conditionally(request.clone()).unwrap();
    assert_eq!(port.commit_conditionally(request.clone()).unwrap(), first);
    port.policy_mut().revoke(first.fork().snapshot_ref());
    assert_eq!(
        port.commit_conditionally(request),
        Err(RunForkError::ReferenceUnavailable)
    );
    assert_eq!(port.fork_count(TENANT), 1);
    assert_eq!(port.audit_count(TENANT), 1);
}

#[test]
fn atomic_port_denies_unauthorized_cross_tenant_and_final_locked_inputs() {
    let mut port = fixture();
    port.grants_mut().revoke(TENANT, "operator", "grant");
    assert_eq!(
        port.commit_conditionally(request("fork_unauthorized", "reason")),
        Err(RunForkError::Unauthorized)
    );
    port.grants_mut()
        .authorize(TENANT, "operator", "grant")
        .unwrap();
    let cross = ForkAuthorization::new(
        OTHER,
        "operator",
        "grant",
        "reason",
        1,
        ForkControlState::Completed,
        7,
    )
    .unwrap();
    port.grants_mut()
        .authorize(OTHER, "operator", "grant")
        .unwrap();
    assert_eq!(
        port.commit_conditionally(ForkRequest::new("run_original", "fork_cross", cross).unwrap()),
        Err(RunForkError::ParentNotFound)
    );
    port.lifecycle_mut()
        .revoke_to_final_lock(TENANT, "run_original");
    assert_eq!(
        port.commit_conditionally(request("fork_final", "reason")),
        Err(RunForkError::FinalLocked)
    );
    assert_eq!(port.fork_count(TENANT), 0);
    assert_eq!(port.audit_count(TENANT), 0);
}

#[test]
fn idempotency_grant_revision_and_lifecycle_changes_fail_closed() {
    let mut port = fixture();
    port.grants_mut()
        .authorize_at_version(TENANT, "operator", "grant", 2)
        .unwrap();
    assert_eq!(
        port.commit_conditionally(request("fork_grant_version", "reason")),
        Err(RunForkError::Unauthorized)
    );
    port.grants_mut()
        .authorize(TENANT, "operator", "grant")
        .unwrap();
    let original = port
        .commit_conditionally(request("fork_idempotent", "reason"))
        .unwrap();
    assert_eq!(
        port.commit_conditionally(request("fork_idempotent", "changed")),
        Err(RunForkError::IdempotencyConflict)
    );
    port.lifecycle_mut().replace(
        TENANT,
        "run_original",
        RunLifecycle::new(ForkControlState::Paused, 8, false).unwrap(),
    );
    assert_eq!(
        port.commit_conditionally(request("fork_idempotent", "reason")),
        Err(RunForkError::ControlStateConflict)
    );
    assert_eq!(port.fork_count(TENANT), 1);
    assert_eq!(port.audit_count(TENANT), 1);
    assert_eq!(original.event().reason(), "reason");
}

#[test]
fn registration_rejects_fabricated_references_and_preserves_first_parent() {
    let mut port = fixture();
    let source = source_ref(&mut port);
    let config = artifact_ref(&mut port, CONFIG);
    let memory = artifact_ref(&mut port, MEMORY);
    assert_eq!(
        port.register_attested_original(
            TENANT,
            "run_original",
            config,
            source.clone(),
            memory,
            100
        ),
        Err(RunForkError::RunAlreadyRegistered)
    );
    assert_eq!(
        port.commit_conditionally(request("fork_first_parent", "reason"))
            .unwrap()
            .fork()
            .snapshot_ref(),
        &source
    );

    let mut empty = InMemoryForkCommitPort::new();
    let source = append(
        &mut empty,
        SOURCE,
        ArtifactKind::SourceSnapshot,
        json!({"observed_cutoff_unix_seconds":100}),
    );
    let config = append(&mut empty, CONFIG, ArtifactKind::RunConfig, json!({}));
    let memory = append(&mut empty, MEMORY, ArtifactKind::MemoryWiki, json!({}));
    assert_eq!(
        empty.register_attested_original(
            TENANT,
            "bad_kind",
            source.reference(),
            source.reference(),
            memory.reference(),
            100
        ),
        Err(RunForkError::ReferenceUnavailable)
    );
    let mut wrong_scope = config.reference();
    wrong_scope.tenant_id = OTHER.to_owned();
    assert_eq!(
        empty.register_attested_original(
            TENANT,
            "bad_scope",
            source.reference(),
            wrong_scope,
            memory.reference(),
            100
        ),
        Err(RunForkError::ReferenceUnavailable)
    );
}

#[test]
fn scheduled_internal_changes_during_final_atomic_predicate_leave_no_effects() {
    let mut port = fixture();
    // Registration consumed reads 1-3. Command reads 4-6 capture and 7-9
    // compare source/config/memory. Revoke #8 (config during compare): the global fence generation changes
    // after source was compared, so the command still rejects without effects.
    let source = source_ref(&mut port);
    port.policy_mut()
        .schedule_revoke_on_liveness_read(8, source);
    assert_eq!(
        port.commit_conditionally(request("fork_policy_race", "reason")),
        Err(RunForkError::CommitFenceChanged)
    );
    assert_eq!(port.fork_count(TENANT), 0);
    assert_eq!(port.audit_count(TENANT), 0);

    let mut port = fixture();
    port.lifecycle_mut()
        .schedule_final_lock_on_read(2, TENANT, "run_original");
    assert_eq!(
        port.commit_conditionally(request("fork_lifecycle_race", "reason")),
        Err(RunForkError::FinalLocked)
    );
    assert_eq!(port.fork_count(TENANT), 0);
    assert_eq!(port.audit_count(TENANT), 0);

    let mut port = fixture();
    port.grants_mut()
        .schedule_revoke_on_read(2, TENANT, "operator", "grant");
    assert_eq!(
        port.commit_conditionally(request("fork_grant_race", "reason")),
        Err(RunForkError::Unauthorized)
    );
    assert_eq!(port.fork_count(TENANT), 0);
    assert_eq!(port.audit_count(TENANT), 0);
}

#[test]
fn scheduled_final_lock_at_the_commit_predicate_leaves_no_child_receipt_or_audit() {
    let mut port = fixture();
    // Lifecycle read #1 captures the fence; read #2 is its final predicate.
    port.lifecycle_mut()
        .schedule_final_lock_on_read(2, TENANT, "run_original");

    assert_eq!(
        port.commit_conditionally(request("fork_final_lock_epoch", "reason")),
        Err(RunForkError::FinalLocked)
    );
    assert_eq!(port.fork_count(TENANT), 0);
    assert_eq!(port.audit_count(TENANT), 0);
}

fn fixture() -> InMemoryForkCommitPort {
    let mut port = InMemoryForkCommitPort::new();
    let source = append(
        &mut port,
        SOURCE,
        ArtifactKind::SourceSnapshot,
        json!({"observed_cutoff_unix_seconds":100}),
    );
    let config = append(
        &mut port,
        CONFIG,
        ArtifactKind::RunConfig,
        json!({"observed_cutoff_unix_seconds":1}),
    );
    let memory = append(
        &mut port,
        MEMORY,
        ArtifactKind::MemoryWiki,
        json!({"observed_cutoff_unix_seconds":100}),
    );
    port.register_attested_original(
        TENANT,
        "run_original",
        source.reference(),
        config.reference(),
        memory.reference(),
        100,
    )
    .unwrap();
    port.grants_mut()
        .authorize(TENANT, "operator", "grant")
        .unwrap();
    port.lifecycle_mut()
        .attest(
            TENANT,
            "run_original",
            RunLifecycle::new(ForkControlState::Completed, 7, false).unwrap(),
        )
        .unwrap();
    port
}
fn append(
    port: &mut InMemoryForkCommitPort,
    id: &str,
    kind: ArtifactKind,
    payload: serde_json::Value,
) -> ArtifactDraft {
    port.artifacts_mut()
        .append(None, ArtifactDraft::new(TENANT, id, 1, kind, payload, None))
        .unwrap()
}
fn artifact_ref(port: &mut InMemoryForkCommitPort, id: &str) -> ArtifactReference {
    port.artifacts_mut()
        .get(TENANT, id, 1)
        .unwrap()
        .unwrap()
        .reference()
}
fn source_ref(port: &mut InMemoryForkCommitPort) -> ArtifactReference {
    artifact_ref(port, SOURCE)
}
fn request(key: &str, reason: &str) -> ForkRequest {
    ForkRequest::new(
        "run_original",
        key,
        ForkAuthorization::new(
            TENANT,
            "operator",
            "grant",
            reason,
            1,
            ForkControlState::Completed,
            7,
        )
        .unwrap(),
    )
    .unwrap()
}
