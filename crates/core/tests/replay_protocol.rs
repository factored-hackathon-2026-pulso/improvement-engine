use improvement_engine_core::replay_clock::ReplayProtocol;
use improvement_engine_core::replay_protocol::{ReplayProtocolError, ReplayProtocolMachine};

#[test]
fn frozen_replay_keeps_one_revision_across_all_cohorts() {
    let mut replay = ReplayProtocolMachine::new(ReplayProtocol::Frozen, 7);

    let first = replay.begin_cohort("cohort-1", 10, &["case-a"]).unwrap();
    replay
        .seal_cohort(&first, "sha256:evaluator-receipt-1")
        .unwrap();
    assert!(replay.propose_update(&first, "update-1", 8).is_err());

    let second = replay.begin_cohort("cohort-2", 11, &["case-b"]).unwrap();
    assert_eq!(first.revision(), 7);
    assert_eq!(second.revision(), 7);
    assert_eq!(replay.active_revision(), 7);
}

#[test]
fn prequential_update_waits_for_seal_and_activates_at_next_cohort() {
    let mut replay = ReplayProtocolMachine::new(ReplayProtocol::Prequential, 7);
    let first = replay.begin_cohort("cohort-1", 10, &["case-a"]).unwrap();

    assert!(replay.propose_update(&first, "update-1", 8).is_err());
    assert_eq!(replay.active_revision(), 7);

    replay
        .seal_cohort(&first, "sha256:evaluator-receipt-1")
        .unwrap();
    replay.propose_update(&first, "update-1", 8).unwrap();
    assert_eq!(replay.active_revision(), 7);
    assert_eq!(first.revision(), 7);

    let next = replay.begin_cohort("cohort-2", 11, &["case-b"]).unwrap();
    assert_eq!(next.revision(), 8);
    assert_eq!(replay.active_revision(), 8);
}

#[test]
fn empty_cohort_does_not_activate_a_pending_prequential_update() {
    let mut replay = ReplayProtocolMachine::new(ReplayProtocol::Prequential, 7);
    let first = replay.begin_cohort("cohort-1", 10, &["case-a"]).unwrap();
    replay
        .seal_cohort(&first, "sha256:evaluator-receipt-1")
        .unwrap();
    replay.propose_update(&first, "update-1", 8).unwrap();

    assert_eq!(
        replay.begin_cohort("empty", 11, &[]),
        Err(ReplayProtocolError::EmptyCohort)
    );
    assert_eq!(replay.active_revision(), 7);

    let next = replay.begin_cohort("cohort-2", 12, &["case-b"]).unwrap();
    assert_eq!(next.revision(), 8);
    assert_eq!(replay.active_revision(), 8);
}

#[test]
fn completed_cohort_identity_cannot_be_reused_after_checkpoint_restore() {
    let mut replay = ReplayProtocolMachine::new(ReplayProtocol::Frozen, 7);
    let first = replay.begin_cohort("cohort-1", 10, &["case-a"]).unwrap();
    replay
        .seal_cohort(&first, "sha256:evaluator-receipt-1")
        .unwrap();

    let second = replay.begin_cohort("cohort-2", 11, &["case-b"]).unwrap();
    replay
        .seal_cohort(&second, "sha256:evaluator-receipt-2")
        .unwrap();

    let mut restored = ReplayProtocolMachine::restore(replay.checkpoint());
    assert!(restored.begin_cohort("cohort-1", 12, &["case-a"]).is_err());
}

#[test]
fn a_case_cannot_be_replayed_in_a_later_cohort_after_checkpoint_restore() {
    let mut replay = ReplayProtocolMachine::new(ReplayProtocol::Prequential, 7);
    let first = replay.begin_cohort("cohort-1", 10, &["case-a"]).unwrap();
    replay
        .seal_cohort(&first, "sha256:evaluator-receipt-1")
        .unwrap();

    let second = replay.begin_cohort("cohort-2", 11, &["case-b"]).unwrap();
    replay
        .seal_cohort(&second, "sha256:evaluator-receipt-2")
        .unwrap();

    let mut restored = ReplayProtocolMachine::restore(replay.checkpoint());
    assert!(restored.begin_cohort("cohort-3", 12, &["case-a"]).is_err());
}
