use improvement_engine_core::run_fork::{
    ForkAuthorization, ForkCommitPort, ForkControlState, ForkGrant, ForkGrantAuthority,
    ForkReferencePolicy, ForkRequest, ForkRunLifecycle, InMemoryForkCommitPort,
    InMemoryForkGrantAuthority, InMemoryForkReferencePolicy, InMemoryForkRunLifecycle,
    RunForkError, RunForkStore, RunLifecycle,
};
use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository, InMemoryArtifactRepository,
};
use serde_json::json;

const TENANT: &str = "demo";
const OTHER: &str = "other";
const SOURCE: &str = "018f0f4e-7bbd-7000-8000-000000000011";
const CONFIG: &str = "018f0f4e-7bbd-7000-8000-000000000012";
const MEMORY: &str = "018f0f4e-7bbd-7000-8000-000000000013";

#[test]
fn fork_copies_only_attested_live_artifacts_and_emits_audit_receipt() {
    let (mut store, mut artifacts, mut policy, mut grants, mut lifecycle) = fixture();
    let receipt = store
        .fork(
            request("fork_001", "reason"),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        )
        .unwrap();
    assert_eq!(receipt.fork().replay_of(), "run_original");
    assert_eq!(receipt.fork().snapshot_ref(), &source_ref(&mut artifacts));
    assert_eq!(receipt.fork().cutoff_unix_seconds(), 100);
    assert_eq!(receipt.event().reason(), "reason");
    assert!(
        receipt.fork().run_id().contains("sha256:"),
        "full digest retained"
    );
    assert!(receipt.event().event_id().contains("sha256:"));
    assert_eq!(store.fork_count(TENANT), 1);
}

#[test]
fn retry_revalidates_liveness_and_never_returns_a_fork_after_revocation() {
    let (mut store, mut artifacts, mut policy, mut grants, mut lifecycle) = fixture();
    let req = request("fork_002", "reason");
    let first = store
        .fork(
            req.clone(),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        )
        .unwrap();
    policy.revoke(first.fork().snapshot_ref());
    assert_eq!(
        store.fork(
            req,
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle
        ),
        Err(RunForkError::ReferenceUnavailable)
    );
    assert_eq!(
        store.fork_count(TENANT),
        1,
        "retry must not duplicate after crash/revocation"
    );
}

#[test]
fn conditional_port_retries_return_the_same_receipt_without_second_child_or_audit() {
    let (mut store, mut artifacts, mut policy, mut grants, mut lifecycle) = fixture();
    let request = request("fork_atomic_retry", "reason");
    let mut port = InMemoryForkCommitPort::new(
        &mut store,
        &mut artifacts,
        &mut policy,
        &mut grants,
        &mut lifecycle,
    );

    let first = port.commit_conditionally(request.clone()).unwrap();
    let retry = port.commit_conditionally(request).unwrap();

    assert_eq!(retry, first);
    assert_eq!(port.fork_count(TENANT), 1);
    assert_eq!(port.audit_count(TENANT), 1);
}

#[test]
fn conditional_port_leaves_no_child_or_audit_when_grant_changes_before_commit() {
    let (mut store, mut artifacts, mut policy, _, mut lifecycle) = fixture();
    let mut grants = RevokingGrant::default();
    let mut port = InMemoryForkCommitPort::new(
        &mut store,
        &mut artifacts,
        &mut policy,
        &mut grants,
        &mut lifecycle,
    );

    assert_eq!(
        port.commit_conditionally(request("fork_grant_race", "reason")),
        Err(RunForkError::Unauthorized)
    );
    assert_eq!(port.fork_count(TENANT), 0);
    assert_eq!(port.audit_count(TENANT), 0);
}

#[test]
fn conditional_port_requires_the_attested_grant_revision() {
    let (mut store, mut artifacts, mut policy, mut grants, mut lifecycle) = fixture();
    grants
        .authorize_at_version(TENANT, "operator", "grant", 2)
        .unwrap();
    assert_eq!(
        store.fork(
            request("fork_grant_revision", "reason"),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        ),
        Err(RunForkError::Unauthorized)
    );
    assert_eq!(store.fork_count(TENANT), 0);
    assert_eq!(store.audit_count(TENANT), 0);
}

#[test]
fn conditional_port_leaves_no_child_or_audit_when_policy_or_lifecycle_changes_before_commit() {
    let (mut store, mut artifacts, _, mut grants, mut lifecycle) = fixture();
    let mut policy = RevokingPolicy::default();
    let mut port = InMemoryForkCommitPort::new(
        &mut store,
        &mut artifacts,
        &mut policy,
        &mut grants,
        &mut lifecycle,
    );
    assert_eq!(
        port.commit_conditionally(request("fork_policy_race", "reason")),
        Err(RunForkError::ReferenceUnavailable)
    );
    assert_eq!(port.fork_count(TENANT), 0);
    assert_eq!(port.audit_count(TENANT), 0);

    let (mut store, mut artifacts, mut policy, mut grants, _) = fixture();
    let mut lifecycle = FinalLockOnCommit::default();
    let mut port = InMemoryForkCommitPort::new(
        &mut store,
        &mut artifacts,
        &mut policy,
        &mut grants,
        &mut lifecycle,
    );
    assert_eq!(
        port.commit_conditionally(request("fork_lifecycle_race", "reason")),
        Err(RunForkError::FinalLocked)
    );
    assert_eq!(port.fork_count(TENANT), 0);
    assert_eq!(port.audit_count(TENANT), 0);
}

#[test]
fn fork_denies_unauthorized_cross_tenant_and_final_locked_inputs() {
    let (mut store, mut artifacts, mut policy, mut grants, mut lifecycle) = fixture();
    let mut unauth = InMemoryForkGrantAuthority::default();
    assert_eq!(
        store.fork(
            request("fork_003", "reason"),
            &mut artifacts,
            &mut policy,
            &mut unauth,
            &mut lifecycle,
        ),
        Err(RunForkError::Unauthorized)
    );
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
    grants.authorize(OTHER, "operator", "grant").unwrap();
    assert_eq!(
        store.fork(
            ForkRequest::new("run_original", "fork_004", cross).unwrap(),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        ),
        Err(RunForkError::ParentNotFound)
    );
    policy.final_lock(&source_ref(&mut artifacts));
    assert_eq!(
        store.fork(
            request("fork_005", "reason"),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        ),
        Err(RunForkError::ReferenceUnavailable)
    );

    let (
        mut fresh_store,
        mut fresh_artifacts,
        mut fresh_policy,
        mut fresh_grants,
        mut fresh_lifecycle,
    ) = fixture();
    fresh_lifecycle.revoke_to_final_lock(TENANT, "run_original");
    assert_eq!(
        fresh_store.fork(
            request("fork_final", "reason"),
            &mut fresh_artifacts,
            &mut fresh_policy,
            &mut fresh_grants,
            &mut fresh_lifecycle,
        ),
        Err(RunForkError::FinalLocked)
    );
}

#[test]
fn altered_key_stale_state_and_cutoff_mismatch_fail_without_child() {
    let (mut store, mut artifacts, mut policy, mut grants, mut lifecycle) = fixture();
    store
        .fork(
            request("fork_006", "reason"),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        )
        .unwrap();
    assert_eq!(
        store.fork(
            request("fork_006", "changed"),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        ),
        Err(RunForkError::IdempotencyConflict)
    );
    assert_eq!(store.fork_count(TENANT), 1);
    let stale = ForkAuthorization::new(
        TENANT,
        "operator",
        "grant",
        "reason",
        1,
        ForkControlState::Paused,
        6,
    )
    .unwrap();
    assert_eq!(
        store.fork(
            ForkRequest::new("run_original", "fork_007", stale).unwrap(),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        ),
        Err(RunForkError::ControlStateConflict)
    );
    let mut artifacts = InMemoryArtifactRepository::default();
    let source = append(
        &mut artifacts,
        SOURCE,
        ArtifactKind::SourceSnapshot,
        json!({"observed_cutoff_unix_seconds": 99}),
    );
    let config = append(
        &mut artifacts,
        CONFIG,
        ArtifactKind::RunConfig,
        json!({"observed_cutoff_unix_seconds": 1}),
    );
    let memory = append(
        &mut artifacts,
        MEMORY,
        ArtifactKind::MemoryWiki,
        json!({"observed_cutoff_unix_seconds": 100}),
    );
    let mut policy = InMemoryForkReferencePolicy::default();
    assert_eq!(
        RunForkStore::new().register_attested_original(
            TENANT,
            "run_original",
            source.reference(),
            config.reference(),
            memory.reference(),
            100,
            &mut artifacts,
            &mut policy
        ),
        Err(RunForkError::CutoffMismatch)
    );
}

#[test]
fn attestation_rejects_fabricated_kind_digest_and_scope_before_registration() {
    let mut artifacts = InMemoryArtifactRepository::default();
    let source = append(
        &mut artifacts,
        SOURCE,
        ArtifactKind::SourceSnapshot,
        json!({"observed_cutoff_unix_seconds": 100}),
    );
    let config = append(
        &mut artifacts,
        CONFIG,
        ArtifactKind::RunConfig,
        json!({"observed_cutoff_unix_seconds": 1}),
    );
    let memory = append(
        &mut artifacts,
        MEMORY,
        ArtifactKind::MemoryWiki,
        json!({"observed_cutoff_unix_seconds": 100}),
    );
    let mut policy = InMemoryForkReferencePolicy::default();
    let mut store = RunForkStore::new();
    assert_eq!(
        store.register_attested_original(
            TENANT,
            "wrong_kind",
            source.reference(),
            source.reference(),
            memory.reference(),
            100,
            &mut artifacts,
            &mut policy,
        ),
        Err(RunForkError::ReferenceUnavailable)
    );
    let mut bad_digest = config.reference();
    bad_digest.digest =
        "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned();
    assert_eq!(
        store.register_attested_original(
            TENANT,
            "bad_digest",
            source.reference(),
            bad_digest,
            memory.reference(),
            100,
            &mut artifacts,
            &mut policy,
        ),
        Err(RunForkError::ReferenceUnavailable)
    );
    let mut cross_scope = memory.reference();
    cross_scope.tenant_id = OTHER.to_owned();
    assert_eq!(
        store.register_attested_original(
            TENANT,
            "cross_scope",
            source.reference(),
            config.reference(),
            cross_scope,
            100,
            &mut artifacts,
            &mut policy,
        ),
        Err(RunForkError::ReferenceUnavailable)
    );
}

#[test]
fn duplicate_attestation_is_rejected_before_it_can_replace_the_first_parent() {
    let (mut store, mut artifacts, mut policy, mut grants, mut lifecycle) = fixture();
    let source = source_ref(&mut artifacts);
    let config = artifacts
        .get(TENANT, CONFIG, 1)
        .unwrap()
        .unwrap()
        .reference();
    let memory = artifacts
        .get(TENANT, MEMORY, 1)
        .unwrap()
        .unwrap()
        .reference();
    assert_eq!(
        store.register_attested_original(
            TENANT,
            "run_original",
            config,
            source.clone(),
            memory,
            100,
            &mut artifacts,
            &mut policy,
        ),
        Err(RunForkError::RunAlreadyRegistered)
    );
    let receipt = store
        .fork(
            request("fork_duplicate", "reason"),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        )
        .unwrap();
    assert_eq!(receipt.fork().snapshot_ref(), &source);
}

#[test]
fn retry_fails_closed_when_grant_is_revoked_or_lifecycle_changes_after_first_reply() {
    let (mut store, mut artifacts, mut policy, mut grants, mut lifecycle) = fixture();
    let request = request("fork_grant_retry", "reason");
    store
        .fork(
            request.clone(),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        )
        .unwrap();
    grants.revoke(TENANT, "operator", "grant");
    assert_eq!(
        store.fork(
            request.clone(),
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        ),
        Err(RunForkError::Unauthorized)
    );
    grants.authorize(TENANT, "operator", "grant").unwrap();
    lifecycle.replace(
        TENANT,
        "run_original",
        RunLifecycle::new(ForkControlState::Paused, 8, false).unwrap(),
    );
    assert_eq!(
        store.fork(
            request,
            &mut artifacts,
            &mut policy,
            &mut grants,
            &mut lifecycle,
        ),
        Err(RunForkError::ControlStateConflict)
    );
    assert_eq!(store.fork_count(TENANT), 1);
}

#[derive(Default)]
struct RevokingGrant {
    reads: u8,
}
impl ForkGrantAuthority for RevokingGrant {
    fn current_grant(&mut self, _: &str, _: &str, _: &str) -> Option<ForkGrant> {
        self.reads += 1;
        (self.reads == 1).then(|| ForkGrant::new(1).unwrap())
    }
}

#[derive(Default)]
struct RevokingPolicy {
    checks: u8,
}
impl ForkReferencePolicy for RevokingPolicy {
    fn is_live_for_fork(&mut self, _: &str, _: &ArtifactReference, _: u64) -> bool {
        self.checks += 1;
        self.checks <= 3
    }
}

#[derive(Default)]
struct FinalLockOnCommit {
    reads: u8,
}
impl ForkRunLifecycle for FinalLockOnCommit {
    fn current(&mut self, _: &str, _: &str) -> Option<RunLifecycle> {
        self.reads += 1;
        RunLifecycle::new(ForkControlState::Completed, 7, self.reads > 1).ok()
    }
}

fn fixture() -> (
    RunForkStore,
    InMemoryArtifactRepository,
    InMemoryForkReferencePolicy,
    InMemoryForkGrantAuthority,
    InMemoryForkRunLifecycle,
) {
    fixture_with_snapshot_cutoff(100)
}
fn fixture_with_snapshot_cutoff(
    cutoff: u64,
) -> (
    RunForkStore,
    InMemoryArtifactRepository,
    InMemoryForkReferencePolicy,
    InMemoryForkGrantAuthority,
    InMemoryForkRunLifecycle,
) {
    let mut artifacts = InMemoryArtifactRepository::default();
    let source = append(
        &mut artifacts,
        SOURCE,
        ArtifactKind::SourceSnapshot,
        json!({"observed_cutoff_unix_seconds": cutoff}),
    );
    let config = append(
        &mut artifacts,
        CONFIG,
        ArtifactKind::RunConfig,
        json!({"observed_cutoff_unix_seconds": 1}),
    );
    let memory = append(
        &mut artifacts,
        MEMORY,
        ArtifactKind::MemoryWiki,
        json!({"observed_cutoff_unix_seconds": 100}),
    );
    let mut policy = InMemoryForkReferencePolicy::default();
    let mut store = RunForkStore::new();
    store
        .register_attested_original(
            TENANT,
            "run_original",
            source.reference(),
            config.reference(),
            memory.reference(),
            100,
            &mut artifacts,
            &mut policy,
        )
        .unwrap();
    let mut grants = InMemoryForkGrantAuthority::default();
    grants.authorize(TENANT, "operator", "grant").unwrap();
    let mut lifecycle = InMemoryForkRunLifecycle::default();
    lifecycle
        .attest(
            TENANT,
            "run_original",
            RunLifecycle::new(ForkControlState::Completed, 7, false).unwrap(),
        )
        .unwrap();
    (store, artifacts, policy, grants, lifecycle)
}
fn append(
    repo: &mut InMemoryArtifactRepository,
    id: &str,
    kind: ArtifactKind,
    payload: serde_json::Value,
) -> ArtifactDraft {
    repo.append(None, ArtifactDraft::new(TENANT, id, 1, kind, payload, None))
        .unwrap()
}
fn source_ref(repo: &mut InMemoryArtifactRepository) -> ArtifactReference {
    repo.get(TENANT, SOURCE, 1).unwrap().unwrap().reference()
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
