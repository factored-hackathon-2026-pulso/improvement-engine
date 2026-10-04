use improvement_engine_core::replay_clock::{
    ReplayAvailabilityProfile, ReplayCase, ReplayEvent, ReplayProtocol, plan_replay,
};
use improvement_engine_core::replay_protocol::{
    ReplayCampaign, ReplayCampaignError, ReplayProtocolError, ReplayProtocolMachine,
};

fn campaign_plan(protocol: ReplayProtocol) -> improvement_engine_core::replay_clock::ReplayPlan {
    plan_replay(
        vec![
            ReplayCase::new("case-a", 10),
            ReplayCase::new("case-b", 10),
            ReplayCase::new("case-c", 20),
        ],
        vec![
            ReplayEvent::new("event-a", "case-a", 10, 20, None),
            ReplayEvent::new("event-b", "case-b", 10, 10, None),
        ],
        ReplayAvailabilityProfile::EventTimeZeroLagAssumption,
        protocol,
    )
    .unwrap()
}

fn source_digest() -> String {
    format!("sha256:{}", "a".repeat(64))
}

fn config_digest() -> String {
    format!("sha256:{}", "b".repeat(64))
}

fn receipt_digest() -> String {
    format!("sha256:{}", "c".repeat(64))
}

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
fn campaign_keeps_tied_cases_together_and_exposes_only_planned_events() {
    let plan = campaign_plan(ReplayProtocol::Frozen);
    let mut campaign = ReplayCampaign::new(plan, source_digest(), config_digest(), 4).unwrap();

    let tied = campaign.next_work().unwrap().unwrap();
    assert_eq!(tied.case_ids(), &["case-a", "case-b"]);
    assert!(tied.visible_event_ids().is_empty());
    campaign.seal(&tied, &receipt_digest()).unwrap();

    let later = campaign.next_work().unwrap().unwrap();
    assert_eq!(later.case_ids(), &["case-c"]);
    assert_eq!(later.visible_event_ids(), &["event-a", "event-b"]);
}

#[test]
fn campaign_activates_a_prequential_revision_only_after_a_sealed_cohort() {
    let plan = campaign_plan(ReplayProtocol::Prequential);
    let mut campaign = ReplayCampaign::new(plan, source_digest(), config_digest(), 4).unwrap();

    let first = campaign.next_work().unwrap().unwrap();
    assert_eq!(first.revision(), 4);
    assert!(campaign.next_work().unwrap().unwrap() == first);
    campaign.seal(&first, &receipt_digest()).unwrap();
    campaign
        .propose_revision("candidate-revision-5", 5)
        .unwrap();

    let second = campaign.next_work().unwrap().unwrap();
    assert_eq!(second.revision(), 5);
}

#[test]
fn campaign_checkpoint_resumes_at_next_cohort_and_rejects_changed_bindings() {
    let plan = campaign_plan(ReplayProtocol::Frozen);
    let campaign = ReplayCampaign::new(plan.clone(), source_digest(), config_digest(), 4).unwrap();
    let mut campaign = campaign;
    let first = campaign.next_work().unwrap().unwrap();
    campaign.seal(&first, &receipt_digest()).unwrap();
    let checkpoint = campaign.checkpoint();

    let mut resumed = ReplayCampaign::restore(
        plan.clone(),
        source_digest(),
        config_digest(),
        checkpoint.clone(),
    )
    .unwrap();
    assert_eq!(resumed.next_work().unwrap(), campaign.next_work().unwrap());
    assert_eq!(
        ReplayCampaign::restore(
            plan,
            format!("sha256:{}", "d".repeat(64)),
            config_digest(),
            checkpoint,
        ),
        Err(ReplayCampaignError::CheckpointBindingMismatch)
    );
}

#[test]
fn campaign_seal_is_idempotent_and_conflicting_or_unsealed_work_cannot_advance() {
    let plan = campaign_plan(ReplayProtocol::Frozen);
    let mut campaign = ReplayCampaign::new(plan, source_digest(), config_digest(), 4).unwrap();
    let first = campaign.next_work().unwrap().unwrap();

    assert_eq!(campaign.next_work().unwrap(), Some(first.clone()));
    assert!(campaign.next_work().unwrap().is_some());
    campaign.seal(&first, &receipt_digest()).unwrap();
    let second = campaign.next_work().unwrap().unwrap();
    campaign.seal(&first, &receipt_digest()).unwrap();
    assert_eq!(
        campaign.seal(&first, &format!("sha256:{}", "d".repeat(64))),
        Err(ReplayCampaignError::ConflictingSeal)
    );
    assert_eq!(campaign.next_work().unwrap(), Some(second));
}

#[test]
fn campaign_rejects_work_from_another_source_snapshot() {
    let plan = campaign_plan(ReplayProtocol::Frozen);
    let mut source_a =
        ReplayCampaign::new(plan.clone(), source_digest(), config_digest(), 4).unwrap();
    let mut source_b = ReplayCampaign::new(
        plan,
        format!("sha256:{}", "d".repeat(64)),
        config_digest(),
        4,
    )
    .unwrap();
    let work_a = source_a.next_work().unwrap().unwrap();
    let work_b = source_b.next_work().unwrap().unwrap();

    assert_ne!(
        source_a.summary().campaign_digest(),
        source_b.summary().campaign_digest()
    );
    assert_eq!(
        source_b.seal(&work_a, &receipt_digest()),
        Err(ReplayCampaignError::WorkBindingMismatch)
    );
    assert_eq!(source_b.next_work().unwrap(), Some(work_b.clone()));
    source_b.seal(&work_b, &receipt_digest()).unwrap();
}

#[test]
fn campaign_summary_contains_only_commitments_and_counts() {
    let plan = campaign_plan(ReplayProtocol::Frozen);
    let campaign = ReplayCampaign::new(plan, source_digest(), config_digest(), 4).unwrap();
    let summary = serde_json::to_string(&campaign.summary()).unwrap();
    for private_value in ["case-a", "case-b", "case-c", "event-a", "event-b"] {
        assert!(!summary.contains(private_value));
    }
    assert!(summary.contains("schedule_digest"));
    assert!(summary.contains("cohort_count"));
}

#[test]
fn debug_views_redact_case_and_event_identifiers() {
    let plan = campaign_plan(ReplayProtocol::Frozen);
    let mut campaign = ReplayCampaign::new(plan, source_digest(), config_digest(), 4).unwrap();
    let work = campaign.next_work().unwrap().unwrap();
    let work_debug = format!("{work:?}");
    let checkpoint_debug = format!("{:?}", campaign.checkpoint());
    let mut protocol = ReplayProtocolMachine::new(ReplayProtocol::Frozen, 4);
    protocol
        .begin_cohort("private-cohort-id", 10, &["case-a", "case-b"])
        .unwrap();
    let protocol_debug = format!("{:?}", protocol.checkpoint());

    for output in [work_debug, checkpoint_debug, protocol_debug] {
        for private_value in [
            "private-cohort-id",
            "case-a",
            "case-b",
            "case-c",
            "event-a",
            "event-b",
        ] {
            assert!(
                !output.contains(private_value),
                "debug leaked {private_value}"
            );
        }
    }
}

#[test]
fn debug_errors_redact_case_cohort_and_update_identifiers() {
    let private_id = "private-customer-case-987";
    let protocol_error = ReplayProtocolError::DuplicateCaseId(private_id.to_owned());
    let wrapped_error =
        ReplayCampaignError::Protocol(ReplayProtocolError::CohortIdConflict(private_id.to_owned()));

    for output in [format!("{protocol_error:?}"), format!("{wrapped_error:?}")] {
        assert!(!output.contains(private_id));
        assert!(output.contains("[REDACTED]"));
    }
}

#[test]
fn campaign_replays_eighteen_hundred_cohorts_once_and_completes() {
    let cases = (0..1_800)
        .map(|index| ReplayCase::new(format!("case-{index:04}"), index))
        .collect();
    let plan = plan_replay(
        cases,
        Vec::new(),
        ReplayAvailabilityProfile::EventTimeZeroLagAssumption,
        ReplayProtocol::Frozen,
    )
    .unwrap();
    let mut campaign = ReplayCampaign::new(plan, source_digest(), config_digest(), 9).unwrap();

    for index in 0..1_800 {
        let work = campaign.next_work().unwrap().expect("cohort available");
        assert_eq!(work.cohort_index(), index);
        assert_eq!(work.case_ids().len(), 1);
        campaign
            .seal(&work, &format!("sha256:{index:064x}"))
            .unwrap();
    }

    assert!(campaign.next_work().unwrap().is_none());
    assert_eq!(campaign.summary().cohort_count(), 1_800);
    assert_eq!(campaign.summary().sealed_cohort_count(), 1_800);
    assert!(campaign.summary().is_complete());
}

#[test]
fn campaign_requires_canonical_commitment_inputs_and_nonempty_plan() {
    let empty_plan = plan_replay(
        Vec::new(),
        Vec::new(),
        ReplayAvailabilityProfile::EventTimeZeroLagAssumption,
        ReplayProtocol::Frozen,
    )
    .unwrap();
    assert_eq!(
        ReplayCampaign::new(empty_plan, source_digest(), config_digest(), 1),
        Err(ReplayCampaignError::EmptyPlan)
    );

    let plan = campaign_plan(ReplayProtocol::Frozen);
    assert!(matches!(
        ReplayCampaign::new(plan.clone(), "not-a-digest".to_owned(), config_digest(), 1),
        Err(ReplayCampaignError::InvalidSnapshotDigest)
    ));
    assert!(matches!(
        ReplayCampaign::new(plan, source_digest(), "not-a-digest".to_owned(), 1),
        Err(ReplayCampaignError::InvalidConfigDigest)
    ));
}

#[test]
fn campaign_resume_preserves_pending_work_and_queued_prequential_update() {
    let plan = campaign_plan(ReplayProtocol::Prequential);
    let mut campaign =
        ReplayCampaign::new(plan.clone(), source_digest(), config_digest(), 4).unwrap();
    let first = campaign.next_work().unwrap().unwrap();
    let pending_checkpoint = campaign.checkpoint();

    let mut resumed = ReplayCampaign::restore(
        plan.clone(),
        source_digest(),
        config_digest(),
        pending_checkpoint,
    )
    .unwrap();
    assert_eq!(resumed.next_work().unwrap(), Some(first.clone()));
    resumed.seal(&first, &receipt_digest()).unwrap();
    resumed.propose_revision("candidate-revision-5", 5).unwrap();
    let update_checkpoint = resumed.checkpoint();

    let mut resumed =
        ReplayCampaign::restore(plan, source_digest(), config_digest(), update_checkpoint).unwrap();
    assert_eq!(resumed.next_work().unwrap().unwrap().revision(), 5);
}

#[test]
fn completed_prequential_campaign_rejects_an_unactivatable_revision() {
    let plan = plan_replay(
        vec![ReplayCase::new("only-case", 10)],
        Vec::new(),
        ReplayAvailabilityProfile::EventTimeZeroLagAssumption,
        ReplayProtocol::Prequential,
    )
    .unwrap();
    let mut campaign = ReplayCampaign::new(plan, source_digest(), config_digest(), 4).unwrap();
    let only = campaign.next_work().unwrap().unwrap();
    campaign.seal(&only, &receipt_digest()).unwrap();

    assert_eq!(
        campaign.propose_revision("never-activates", 5),
        Err(ReplayCampaignError::CampaignComplete)
    );
    assert!(campaign.summary().is_complete());
    assert_eq!(campaign.summary().active_revision(), 4);
}

#[test]
fn duplicate_cases_in_a_large_cohort_are_rejected_without_state_change() {
    let mut case_ids = (0..1_800)
        .map(|index| format!("case-{index:04}"))
        .collect::<Vec<_>>();
    case_ids.push("case-0900".to_owned());
    let case_refs = case_ids.iter().map(String::as_str).collect::<Vec<_>>();
    let mut replay = ReplayProtocolMachine::new(ReplayProtocol::Frozen, 4);

    assert_eq!(
        replay.begin_cohort("duplicate", 10, &case_refs),
        Err(ReplayProtocolError::DuplicateCaseId("case-0900".to_owned()))
    );
    assert_eq!(replay.active_revision(), 4);
    assert!(replay.begin_cohort("valid", 10, &["case-a"]).is_ok());
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
