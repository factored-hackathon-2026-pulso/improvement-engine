use improvement_engine_core::run_fork::{ForkRequest, ImmutableRunBinding, RunForkStore};

#[test]
fn forking_a_registered_run_creates_a_distinct_immutable_replay_without_mutating_the_parent() {
    let mut store = RunForkStore::new();
    let parent = ImmutableRunBinding::new(
        "demo",
        "run_original",
        "snapshot_a",
        "config_a",
        "memory_a",
        100,
    )
    .expect("valid original");
    store
        .register_original(parent.clone())
        .expect("parent is registered");

    let replay = store
        .fork(ForkRequest::new("demo", parent.run_id(), "fork_001", "reason").unwrap())
        .expect("fork succeeds");

    assert_ne!(replay.run_id(), parent.run_id());
    assert_eq!(replay.replay_of(), Some(parent.run_id()));
    assert_eq!(replay.snapshot_ref(), parent.snapshot_ref());
    assert_eq!(replay.config_ref(), parent.config_ref());
    assert_eq!(replay.memory_ref(), parent.memory_ref());
    assert_eq!(replay.cutoff_unix_seconds(), parent.cutoff_unix_seconds());
    assert_eq!(store.run("demo", parent.run_id()).unwrap(), &parent);
}

#[test]
fn same_fork_request_replays_one_child_but_altered_input_cannot_reuse_its_key() {
    let (mut store, parent) = registered_store();
    let request = ForkRequest::new("demo", parent.run_id(), "fork_002", "debug").unwrap();

    let first = store.fork(request.clone()).expect("first persisted fork");
    let retry = store
        .fork(request)
        .expect("crash retry returns recorded child");
    assert_eq!(first, retry);
    assert_eq!(store.run_count("demo"), 2, "retry must not duplicate a run");

    let altered =
        ForkRequest::new("demo", parent.run_id(), "fork_002", "different_reason").unwrap();
    assert!(format!("{:?}", store.fork(altered).unwrap_err()).contains("IdempotencyConflict"));
    assert_eq!(store.run_count("demo"), 2);
}

#[test]
fn fork_fails_closed_for_unknown_cross_scope_or_revoked_parent_inputs() {
    let (mut store, parent) = registered_store();
    store
        .register_original(
            ImmutableRunBinding::new(
                "other",
                "run_other",
                "snapshot_other",
                "config_other",
                "memory_other",
                100,
            )
            .unwrap(),
        )
        .unwrap();

    assert!(
        format!(
            "{:?}",
            store
                .fork(ForkRequest::new("demo", "missing", "fork_003", "debug").unwrap())
                .unwrap_err()
        )
        .contains("ParentNotFound")
    );
    assert!(
        format!(
            "{:?}",
            store
                .fork(ForkRequest::new("demo", "run_other", "fork_004", "debug").unwrap())
                .unwrap_err()
        )
        .contains("ParentNotFound")
    );

    store.revoke_reference(parent.snapshot_ref()).unwrap();
    assert!(
        format!(
            "{:?}",
            store
                .fork(ForkRequest::new("demo", parent.run_id(), "fork_005", "debug").unwrap())
                .unwrap_err()
        )
        .contains("ReferenceUnavailable")
    );
    assert_eq!(
        store.run_count("demo"),
        1,
        "failed validation cannot leave a child"
    );
}

fn registered_store() -> (RunForkStore, ImmutableRunBinding) {
    let mut store = RunForkStore::new();
    let parent = ImmutableRunBinding::new(
        "demo",
        "run_original",
        "snapshot_a",
        "config_a",
        "memory_a",
        100,
    )
    .unwrap();
    store.register_original(parent.clone()).unwrap();
    (store, parent)
}
