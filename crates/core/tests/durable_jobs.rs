use improvement_engine_core::durable_jobs::{
    DurableJobError, DurableJobRepository, DurableJobStore, JobAdmissionRequest, JobControlCommand,
    JobControlKind, JobControlOutcome, JobEffectState, JobStatus, ReconciliationEvidence,
    RecoveryDisposition,
};
use improvement_engine_core::quota_grant::{
    AuthorizedGrant, QuotaLimit, QuotaReservation, QuotaResource, QuotaWindow,
};
use improvement_engine_core::run_config::{Cadence, EligibleSource, RunConfig, ScanBudget};

fn config() -> RunConfig {
    RunConfig::new(
        1,
        Cadence::EventTriggered,
        ScanBudget::new(1, 100, 1_024).expect("valid budget"),
        vec![EligibleSource::new("latam_bank", "contacts").expect("valid source")],
    )
    .expect("valid config")
}

#[test]
fn crash_after_dispatch_is_unknown_and_recovery_never_releases_it_for_retry() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("valid limit"))
        .expect("configured quota");
    let admitted = store
        .admit(request("demo", "contact_spike_003"))
        .expect("admitted");
    let job_id = admitted.job().expect("admitted job").job_id().to_owned();
    let lease = store
        .acquire_lease("demo", &job_id, "worker_a", 1_100, 10)
        .expect("lease");
    store
        .begin_effect_dispatch("demo", &job_id, "worker_a", lease.fence_token(), 1_101)
        .expect("the real boundary records ambiguity before an external effect");
    let before_first_recovery = store.control_version("demo", &job_id).expect("version");

    assert_eq!(
        store
            .recover_after_restart("demo", &job_id, 1_120)
            .expect("recovery result"),
        RecoveryDisposition::ReconciliationRequired,
    );
    assert_eq!(
        store.effect_state("demo", &job_id).expect("visible state"),
        JobEffectState::UnknownPendingReconciliation,
    );
    assert_eq!(
        store.acquire_lease("demo", &job_id, "worker_b", 1_120, 10),
        Err(DurableJobError::ReconciliationRequired),
    );
    let after_first_recovery = store.control_version("demo", &job_id).expect("version");
    assert_eq!(after_first_recovery, before_first_recovery + 1);
    assert_eq!(store.active_fence("demo", &job_id), Ok(None));
    assert_eq!(
        store
            .recover_after_restart("demo", &job_id, 1_121)
            .expect("repeated unknown recovery"),
        RecoveryDisposition::ReconciliationRequired,
    );
    assert_eq!(
        store.control_version("demo", &job_id),
        Ok(after_first_recovery)
    );
}

fn resource() -> QuotaResource {
    QuotaResource::new("job_runs").expect("valid resource")
}

fn window(tenant: &str) -> QuotaWindow {
    QuotaWindow::new(tenant, resource(), 1_000, 2_000).expect("valid window")
}

fn request(tenant: &str, trigger_key: &str) -> JobAdmissionRequest {
    request_with_trigger_digest(
        tenant,
        trigger_key,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
}

fn request_with_trigger_digest(
    tenant: &str,
    trigger_key: &str,
    trigger_digest: &str,
) -> JobAdmissionRequest {
    let run_config = config();
    let quota = QuotaReservation::new(
        trigger_key,
        window(tenant),
        &run_config,
        AuthorizedGrant::issue("grant_1", "policy_1", tenant, resource(), 10, 2_000)
            .expect("valid grant"),
        1,
        1_001,
    )
    .expect("valid reservation");
    JobAdmissionRequest::new(tenant, trigger_key, trigger_digest, run_config, quota)
        .expect("valid request")
}

#[test]
fn equivalent_triggers_admit_one_job_and_reserve_quota_once() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("valid limit"))
        .expect("configured quota");

    let first = store
        .admit(request("demo", "contact_spike_001"))
        .expect("first admission");
    let retry = store
        .admit(request("demo", "contact_spike_001"))
        .expect("idempotent retry");

    assert_eq!(first, retry);
    assert_eq!(store.job_count("demo"), 1);
    assert_eq!(store.reserved_units(&window("demo")), 1);
}

#[test]
fn stale_worker_cannot_dispatch_or_acknowledge_after_its_lease_is_replaced() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("valid limit"))
        .expect("configured quota");
    let admitted = store
        .admit(request("demo", "contact_spike_002"))
        .expect("admitted");
    let job_id = admitted.job().expect("admitted job").job_id().to_owned();

    let stale = store
        .acquire_lease("demo", &job_id, "worker_a", 1_100, 10)
        .expect("first worker leases job");
    let current = store
        .acquire_lease("demo", &job_id, "worker_b", 1_110, 10)
        .expect("expired lease can be recovered by another worker");

    assert_eq!(
        store.begin_effect_dispatch("demo", &job_id, "worker_a", stale.fence_token(), 1_111),
        Err(DurableJobError::StaleFence),
    );
    assert_eq!(
        store.acknowledge_effect(
            "demo",
            &job_id,
            "worker_a",
            stale.fence_token(),
            "effect:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1_111,
        ),
        Err(DurableJobError::StaleFence),
    );
    assert_eq!(
        store
            .effect_state("demo", &job_id)
            .expect("visible to tenant"),
        JobEffectState::NoEffect,
    );
    assert!(current.fence_token() > stale.fence_token());
}

#[test]
fn conflicting_trigger_reuse_and_invalid_grant_leave_no_second_job_or_reservation() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("valid limit"))
        .expect("configured quota");
    store
        .admit(request("demo", "contact_spike_004"))
        .expect("first admission");

    assert_eq!(
        store.admit(request_with_trigger_digest(
            "demo",
            "contact_spike_004",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )),
        Err(DurableJobError::IdempotencyConflict {
            trigger_idempotency_key: "contact_spike_004".to_owned(),
        }),
    );
    assert_eq!(store.job_count("demo"), 1);
    assert_eq!(store.reserved_units(&window("demo")), 1);

    let run_config = config();
    let expired = JobAdmissionRequest::new(
        "demo",
        "expired_trigger",
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        run_config.clone(),
        QuotaReservation::new(
            "expired_trigger",
            window("demo"),
            &run_config,
            AuthorizedGrant::issue("expired_grant", "policy_1", "demo", resource(), 10, 1_001)
                .expect("valid expired grant record"),
            1,
            1_001,
        )
        .expect("syntactically valid quota request"),
    )
    .expect("syntactically valid admission");
    assert!(matches!(
        store.admit(expired),
        Err(DurableJobError::Quota(_))
    ));
    assert_eq!(store.job_count("demo"), 1);
    assert_eq!(store.reserved_units(&window("demo")), 1);
}

#[test]
fn tenant_cannot_read_or_transition_another_tenants_job() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("demo limit"))
        .expect("configured quota");
    let admitted = store
        .admit(request("demo", "contact_spike_005"))
        .expect("admitted");
    let job_id = admitted.job().expect("admitted job").job_id().to_owned();

    assert_eq!(
        store.effect_state("other_tenant", &job_id),
        Err(DurableJobError::TenantAccessDenied),
    );
    assert_eq!(
        store.acquire_lease("other_tenant", &job_id, "worker_b", 1_100, 10),
        Err(DurableJobError::TenantAccessDenied),
    );
}

#[test]
fn deferred_quota_admission_has_no_job_ref_and_cannot_be_leased() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 1).expect("one unit limit"))
        .expect("configured quota");
    store
        .admit(request("demo", "contact_spike_006_first"))
        .expect("first consumes quota");

    let deferred = store
        .admit(request("demo", "contact_spike_006_deferred"))
        .expect("deferred receipt");
    let retry = store
        .admit(request("demo", "contact_spike_006_deferred"))
        .expect("same deferred receipt");

    assert_eq!(deferred, retry);
    assert!(!deferred.is_admitted());
    assert_eq!(deferred.job(), None);
    assert_eq!(store.job_count("demo"), 1);
    assert_eq!(store.reserved_units(&window("demo")), 1);
}

#[test]
fn current_fence_also_requires_the_lease_owner() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("limit"))
        .expect("configured quota");
    let admitted = store
        .admit(request("demo", "contact_spike_007"))
        .expect("admitted");
    let job_id = admitted.job().expect("job").job_id().to_owned();
    let lease = store
        .acquire_lease("demo", &job_id, "worker_a", 1_100, 10)
        .expect("lease");

    assert_eq!(
        store.begin_effect_dispatch("demo", &job_id, "worker_b", lease.fence_token(), 1_101),
        Err(DurableJobError::LeaseOwnerMismatch),
    );
    assert_eq!(
        store.status("demo", &job_id).expect("status"),
        JobStatus::Leased {
            attempt: 1,
            fence_token: lease.fence_token(),
            expires_at_unix_seconds: 1_110,
        }
    );
}

#[test]
fn reconciler_can_close_an_unknown_job_only_with_versioned_evidence() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("limit"))
        .expect("configured quota");
    let admitted = store
        .admit(request("demo", "contact_spike_008"))
        .expect("admitted");
    let job_id = admitted.job().expect("job").job_id().to_owned();
    let lease = store
        .acquire_lease("demo", &job_id, "worker_a", 1_100, 10)
        .expect("lease");
    store
        .begin_effect_dispatch("demo", &job_id, "worker_a", lease.fence_token(), 1_101)
        .expect("dispatch boundary");
    store
        .recover_after_restart("demo", &job_id, 1_120)
        .expect("unknown recovery");
    store
        .trust_reconciliation_authority("demo", "reconciler_a")
        .expect("trusted authority");

    store
        .reconcile_unknown(
            "demo",
            &job_id,
            ReconciliationEvidence::no_effect(
                "demo",
                &job_id,
                lease.fence_token(),
                "reconciler_a",
                "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            )
            .expect("valid evidence"),
        )
        .expect("evidence closes unknown state");
    assert_eq!(
        store.effect_state("demo", &job_id).expect("effect"),
        JobEffectState::NoEffect
    );
    assert_eq!(
        store.status("demo", &job_id).expect("status"),
        JobStatus::CompletedNoEffect
    );
    assert_eq!(
        store
            .recover_after_restart("demo", &job_id, 1_130)
            .expect("terminal recovery"),
        RecoveryDisposition::Terminal,
    );
    assert_eq!(
        store.acquire_lease("demo", &job_id, "worker_b", 1_130, 10),
        Err(DurableJobError::InvalidTransition),
    );
}

#[test]
fn reconciliation_rejects_evidence_for_another_dispatch_or_untrusted_authority() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("limit"))
        .expect("configured quota");
    let admitted = store
        .admit(request("demo", "contact_spike_012"))
        .expect("admitted");
    let job_id = admitted.job().expect("job").job_id().to_owned();
    let lease = store
        .acquire_lease("demo", &job_id, "worker_a", 1_100, 10)
        .expect("lease");
    store
        .begin_effect_dispatch("demo", &job_id, "worker_a", lease.fence_token(), 1_101)
        .expect("dispatch");
    store
        .recover_after_restart("demo", &job_id, 1_120)
        .expect("recovery");
    store
        .trust_reconciliation_authority("demo", "reconciler_a")
        .expect("trusted authority");

    let wrong_job = ReconciliationEvidence::no_effect(
        "demo",
        "job:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        lease.fence_token(),
        "reconciler_a",
        "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    )
    .expect("well-formed but wrong target evidence");
    assert_eq!(
        store.reconcile_unknown("demo", &job_id, wrong_job),
        Err(DurableJobError::ReconciliationTargetMismatch),
    );
    let wrong_tenant = ReconciliationEvidence::no_effect(
        "other_tenant",
        &job_id,
        lease.fence_token(),
        "reconciler_a",
        "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    )
    .expect("well-formed but wrong tenant evidence");
    assert_eq!(
        store.reconcile_unknown("demo", &job_id, wrong_tenant),
        Err(DurableJobError::ReconciliationTargetMismatch),
    );
    let wrong_fence = ReconciliationEvidence::no_effect(
        "demo",
        &job_id,
        lease.fence_token() + 1,
        "reconciler_a",
        "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    )
    .expect("well-formed but wrong evidence");
    assert_eq!(
        store.reconcile_unknown("demo", &job_id, wrong_fence),
        Err(DurableJobError::ReconciliationDispatchMismatch),
    );
    let untrusted = ReconciliationEvidence::no_effect(
        "demo",
        &job_id,
        lease.fence_token(),
        "untrusted_authority",
        "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    )
    .expect("well-formed but untrusted evidence");
    assert_eq!(
        store.reconcile_unknown("demo", &job_id, untrusted),
        Err(DurableJobError::UntrustedReconciliationAuthority),
    );
    assert_eq!(
        store.status("demo", &job_id).expect("still unknown"),
        JobStatus::UnknownPendingReconciliation
    );
}

#[test]
fn admission_rejects_cross_boundary_quota_mismatch_before_it_can_mutate_any_quota() {
    let run_config = config();
    let mismatched_tenant = JobAdmissionRequest::new(
        "demo",
        "contact_spike_009",
        "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        run_config.clone(),
        QuotaReservation::new(
            "contact_spike_009",
            window("other_tenant"),
            &run_config,
            AuthorizedGrant::issue("grant_1", "policy_1", "other_tenant", resource(), 10, 2_000)
                .expect("grant"),
            1,
            1_001,
        )
        .expect("reservation"),
    );
    assert!(matches!(
        mismatched_tenant,
        Err(DurableJobError::QuotaTenantMismatch)
    ));
}

#[test]
fn same_trigger_and_payload_digest_with_changed_quota_request_is_a_conflict() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("limit"))
        .expect("configured quota");
    store
        .admit(request("demo", "contact_spike_010"))
        .expect("first admission");

    let run_config = config();
    let changed_quota = JobAdmissionRequest::new(
        "demo",
        "contact_spike_010",
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        run_config.clone(),
        QuotaReservation::new(
            "contact_spike_010",
            window("demo"),
            &run_config,
            AuthorizedGrant::issue("grant_1", "policy_1", "demo", resource(), 10, 2_000)
                .expect("grant"),
            2,
            1_001,
        )
        .expect("reservation"),
    )
    .expect("boundary matches");

    assert_eq!(
        store.admit(changed_quota),
        Err(DurableJobError::IdempotencyConflict {
            trigger_idempotency_key: "contact_spike_010".to_owned(),
        }),
    );
    assert_eq!(store.reserved_units(&window("demo")), 1);
}

#[test]
fn reducer_is_consumable_through_the_durable_repository_port() {
    fn admit_via_port(
        repository: &mut impl DurableJobRepository,
        request: JobAdmissionRequest,
    ) -> Result<(), DurableJobError> {
        repository.admit_atomically(request).map(|_| ())
    }

    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("limit"))
        .expect("configured quota");
    admit_via_port(&mut store, request("demo", "contact_spike_011"))
        .expect("admission through port");
}

#[test]
fn claim_next_job_gives_two_workers_distinct_jobs() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("limit"))
        .expect("configured quota");
    let first = store
        .admit(request("demo", "claim_two_001"))
        .expect("first");
    let second = store
        .admit(request("demo", "claim_two_002"))
        .expect("second");
    let first_id = first.job().expect("admitted").job_id().to_owned();
    let second_id = second.job().expect("admitted").job_id().to_owned();
    // These deterministic digests deliberately sort opposite to admission
    // order, proving selection is not accidentally supplied by BTreeMap.
    assert!(
        first_id > second_id,
        "fixture must oppose lexical map order"
    );

    let claim_a = store
        .claim_next_job("demo", "worker_a", 1_100, 10)
        .expect("worker a claim")
        .expect("first job");
    let claim_b = store
        .claim_next_job("demo", "worker_b", 1_100, 10)
        .expect("worker b claim")
        .expect("second job");

    assert_ne!(claim_a.job_id(), claim_b.job_id());
    assert_eq!(claim_a.job_id(), first_id);
    assert_eq!(claim_b.job_id(), second_id);
    assert_eq!(claim_a.lease().worker_id(), "worker_a");
    assert_eq!(
        store.status("demo", &first_id),
        Ok(JobStatus::Leased {
            attempt: 1,
            fence_token: 1,
            expires_at_unix_seconds: 1_110,
        })
    );
    assert_eq!(
        store.status("demo", &second_id),
        Ok(JobStatus::Leased {
            attempt: 1,
            fence_token: 1,
            expires_at_unix_seconds: 1_110,
        })
    );
}

#[test]
fn claim_next_job_reclaims_only_after_expiry_and_advances_fence_once() {
    let (mut store, job_id) = admitted_store("claim_expiry_001");
    let before = store.control_version("demo", &job_id).expect("version");
    let first = store
        .claim_next_job("demo", "worker_a", 100, 30)
        .expect("claim")
        .expect("job");
    assert_eq!(first.lease().fence_token(), 1);
    assert_eq!(first.lease().expires_at_unix_seconds(), 130);
    assert_eq!(store.control_version("demo", &job_id), Ok(before + 1));

    assert_eq!(store.claim_next_job("demo", "worker_b", 129, 30), Ok(None),);
    assert_eq!(store.control_version("demo", &job_id), Ok(before + 1));
    let reclaimed = store
        .claim_next_job("demo", "worker_b", 130, 30)
        .expect("expired claim")
        .expect("reclaimed job");
    assert_eq!(reclaimed.job_id(), job_id);
    assert_eq!(reclaimed.lease().fence_token(), 2);
    assert_eq!(store.control_version("demo", &job_id), Ok(before + 2));
}

#[test]
fn claim_next_job_prefers_expired_earlier_admission_over_later_queued_job() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("limit"))
        .expect("configured quota");
    let earlier = store
        .admit(request("demo", "claim_expired_fifo_earlier"))
        .expect("earlier job");
    let earlier_id = earlier.job().expect("admitted").job_id().to_owned();
    let leased = store
        .claim_next_job("demo", "worker_a", 100, 10)
        .expect("initial claim")
        .expect("earlier job claimed");
    let later = store
        .admit(request("demo", "claim_expired_fifo_later"))
        .expect("later job");
    let later_id = later.job().expect("admitted").job_id().to_owned();

    let reclaimed = store
        .claim_next_job("demo", "worker_b", 110, 30)
        .expect("expired earlier job claim")
        .expect("eligible job");
    assert_eq!(reclaimed.job_id(), earlier_id);
    assert_ne!(reclaimed.job_id(), later_id);
    assert_eq!(
        reclaimed.lease().fence_token(),
        leased.lease().fence_token() + 1
    );
    assert_eq!(
        store.status("demo", &later_id),
        Ok(JobStatus::Queued),
        "the later queued job must remain untouched"
    );
}

#[test]
fn claim_next_job_skips_paused_deferred_and_unknown_effect_jobs() {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 1).expect("limit"))
        .expect("configured quota");
    let admitted = store
        .admit(request("demo", "claim_skips_001"))
        .expect("admitted");
    let paused_id = admitted.job().expect("job").job_id().to_owned();
    let command = JobControlCommand::new(
        "demo",
        &paused_id,
        "operator_a",
        "pause_claim_skips",
        store.control_version("demo", &paused_id).expect("version"),
        None,
        JobControlKind::Pause,
    )
    .expect("valid control");
    store.control_job(command, 50).expect("pause");
    let deferred = store
        .admit(request("demo", "claim_skips_deferred"))
        .expect("deferred");
    assert!(!deferred.is_admitted());
    assert_eq!(store.claim_next_job("demo", "worker_a", 100, 30), Ok(None));

    // A separately admitted job which crossed the effect boundary remains
    // unreclaimable even after its lease expires.
    let mut another = DurableJobStore::new();
    another
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("limit"))
        .expect("configured quota");
    let receipt = another
        .admit(request("demo", "claim_unknown_001"))
        .expect("admitted");
    let unknown_id = receipt.job().expect("job").job_id().to_owned();
    let lease = another
        .claim_next_job("demo", "worker_a", 100, 1)
        .expect("claim")
        .expect("job");
    another
        .begin_effect_dispatch(
            "demo",
            &unknown_id,
            "worker_a",
            lease.lease().fence_token(),
            100,
        )
        .expect("effect boundary");
    assert_eq!(
        another.claim_next_job("demo", "worker_b", 101, 30),
        Ok(None)
    );
}

#[test]
fn claim_next_job_skips_cancelled_completed_and_acknowledged_jobs() {
    let (mut cancelled, cancelled_id) = admitted_store("claim_cancelled_001");
    let cancel = JobControlCommand::new(
        "demo",
        &cancelled_id,
        "operator_a",
        "cancel_claim_skip",
        cancelled
            .control_version("demo", &cancelled_id)
            .expect("version"),
        None,
        JobControlKind::Cancel,
    )
    .expect("valid control");
    cancelled.control_job(cancel, 50).expect("cancel");
    assert_eq!(
        cancelled.claim_next_job("demo", "worker_a", 100, 30),
        Ok(None)
    );

    let (mut completed, completed_id) = admitted_store("claim_completed_001");
    let claim = completed
        .claim_next_job("demo", "worker_a", 100, 1)
        .expect("claim")
        .expect("job");
    completed
        .begin_effect_dispatch(
            "demo",
            &completed_id,
            "worker_a",
            claim.lease().fence_token(),
            100,
        )
        .expect("dispatch");
    completed
        .recover_after_restart("demo", &completed_id, 101)
        .expect("recovery");
    completed
        .trust_reconciliation_authority("demo", "reconciler_a")
        .expect("trusted reconciler");
    completed
        .reconcile_unknown(
            "demo",
            &completed_id,
            ReconciliationEvidence::no_effect(
                "demo",
                &completed_id,
                claim.lease().fence_token(),
                "reconciler_a",
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("evidence"),
        )
        .expect("no-effect reconciliation");
    assert_eq!(
        completed.claim_next_job("demo", "worker_b", 102, 30),
        Ok(None)
    );

    let (mut acknowledged, acknowledged_id) = admitted_store("claim_acknowledged_001");
    let claim = acknowledged
        .claim_next_job("demo", "worker_a", 100, 30)
        .expect("claim")
        .expect("job");
    acknowledged
        .begin_effect_dispatch(
            "demo",
            &acknowledged_id,
            "worker_a",
            claim.lease().fence_token(),
            101,
        )
        .expect("dispatch");
    acknowledged
        .acknowledge_effect(
            "demo",
            &acknowledged_id,
            "worker_a",
            claim.lease().fence_token(),
            "effect:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            102,
        )
        .expect("acknowledgment");
    assert_eq!(
        acknowledged.claim_next_job("demo", "worker_b", 103, 30),
        Ok(None)
    );
}

#[test]
fn claim_next_job_rejects_invalid_scope_worker_and_duration_without_mutation() {
    let (mut store, job_id) = admitted_store("claim_validation_001");
    let before = store.control_version("demo", &job_id).expect("version");
    assert_eq!(
        store.claim_next_job("", "worker_a", 100, 30),
        Err(DurableJobError::InvalidIdentifier { field: "tenant_id" }),
    );
    assert_eq!(
        store.claim_next_job("demo", "", 100, 30),
        Err(DurableJobError::InvalidIdentifier { field: "worker_id" }),
    );
    assert_eq!(
        store.claim_next_job("demo", "worker_a", 100, 0),
        Err(DurableJobError::InvalidLeaseDuration),
    );
    assert_eq!(store.control_version("demo", &job_id), Ok(before));
    assert_eq!(store.active_fence("demo", &job_id), Ok(None));
    assert!(
        store
            .claim_next_job("demo", "worker_a", u64::MAX, 1)
            .is_err()
    );
    assert_eq!(store.control_version("demo", &job_id), Ok(before));
    assert_eq!(store.active_fence("demo", &job_id), Ok(None));
}

#[test]
fn empty_claim_queue_is_a_state_noop() {
    let mut store = DurableJobStore::new();
    assert_eq!(store.claim_next_job("demo", "worker_a", 100, 30), Ok(None));
    assert_eq!(store.job_count("demo"), 0);
}

#[test]
fn claim_next_job_is_tenant_scoped_and_does_not_reveal_foreign_jobs() {
    let (mut store, job_id) = admitted_store("claim_tenant_001");
    assert_eq!(store.claim_next_job("other", "worker_a", 100, 30), Ok(None));
    assert_eq!(store.active_fence("demo", &job_id), Ok(None));
    let claim = store
        .claim_next_job("demo", "worker_a", 100, 30)
        .expect("tenant claim")
        .expect("own job");
    assert_eq!(claim.job_id(), job_id);
}

#[test]
fn pause_is_an_atomic_durable_transition_with_idempotent_audit_receipt() {
    let (mut store, job_id) = admitted_store("control_pause_001");
    let command = control(
        &store,
        &job_id,
        "operator_a",
        "pause_001",
        JobControlKind::Pause,
    );

    let first = DurableJobRepository::control_job_atomically(&mut store, command.clone(), 1_010)
        .expect("durable pause");
    let replay = DurableJobRepository::control_job_atomically(&mut store, command, 1_999)
        .expect("same semantic command replays its recorded receipt");

    assert_eq!(first, replay);
    assert_eq!(first.outcome(), &JobControlOutcome::Paused);
    assert_eq!(first.tenant_id(), "demo");
    assert_eq!(first.job_id(), job_id);
    assert_eq!(first.operator_id(), "operator_a");
    assert_eq!(first.idempotency_key(), "pause_001");
    assert_eq!(first.expected_fence(), None);
    assert_eq!(first.confirmed_status(), &JobStatus::Paused);
    assert_eq!(first.target_version() + 1, first.confirmed_version());
    assert_eq!(store.status("demo", &job_id), Ok(JobStatus::Paused));
}

#[test]
fn unknown_or_in_dispatch_job_never_claims_cancelled_before_effect() {
    let (mut store, job_id) = admitted_store("control_unknown_001");
    let lease = store
        .acquire_lease("demo", &job_id, "worker_a", 1_010, 30)
        .expect("lease");
    let leased_cancel = control(
        &store,
        &job_id,
        "operator_a",
        "cancel_leased_001",
        JobControlKind::Cancel,
    );
    let leased_receipt = store
        .control_job(leased_cancel, 1_010)
        .expect("leased run stays truthful");
    assert_eq!(
        leased_receipt.outcome(),
        &JobControlOutcome::ReconciliationRequired
    );
    assert_eq!(
        leased_receipt.target_version(),
        leased_receipt.confirmed_version()
    );
    assert!(matches!(
        store.status("demo", &job_id),
        Ok(JobStatus::Leased { .. })
    ));
    store
        .begin_effect_dispatch("demo", &job_id, "worker_a", lease.fence_token(), 1_011)
        .expect("unknown before effect");
    let command = control(
        &store,
        &job_id,
        "operator_a",
        "cancel_001",
        JobControlKind::Cancel,
    );

    let receipt = store
        .control_job(command, 1_012)
        .expect("truthful command receipt");
    assert_eq!(
        receipt.outcome(),
        &JobControlOutcome::ReconciliationRequired
    );
    assert_eq!(receipt.target_version(), receipt.confirmed_version());
    assert_eq!(
        receipt.confirmed_effect(),
        &JobEffectState::UnknownPendingReconciliation
    );
    assert_eq!(
        store.status("demo", &job_id),
        Ok(JobStatus::UnknownPendingReconciliation)
    );
}

#[test]
fn stale_control_read_loses_to_a_lease_transition_and_cross_tenant_is_not_inferred() {
    let (mut store, job_id) = admitted_store("control_race_001");
    let stale = control(
        &store,
        &job_id,
        "operator_a",
        "cancel_002",
        JobControlKind::Cancel,
    );
    store
        .acquire_lease("demo", &job_id, "worker_a", 1_010, 30)
        .expect("concurrent lease wins");
    assert_eq!(
        store.control_job(stale, 1_011),
        Err(DurableJobError::ControlVersionConflict)
    );

    let foreign = JobControlCommand::new(
        "other",
        &job_id,
        "operator_a",
        "cancel_003",
        1,
        None,
        JobControlKind::Cancel,
    )
    .expect("well formed foreign command");
    assert_eq!(
        store.control_job(foreign, 1_012),
        Err(DurableJobError::JobNotFound)
    );
}

#[test]
fn control_idempotency_key_binds_all_semantic_fields_and_terminal_cancel_does_not_pause() {
    let (mut store, job_id) = admitted_store("control_key_001");
    let cancel = control(
        &store,
        &job_id,
        "operator_a",
        "shared_key",
        JobControlKind::Cancel,
    );
    store
        .control_job(cancel, 1_010)
        .expect("cancel before effect");
    let after_cancel = store.control_version("demo", &job_id).expect("version");
    let changed_operator = JobControlCommand::new(
        "demo",
        &job_id,
        "operator_b",
        "shared_key",
        after_cancel,
        None,
        JobControlKind::Pause,
    )
    .expect("formed command");
    assert_eq!(
        store.control_job(changed_operator, 1_011),
        Err(DurableJobError::ControlIdempotencyConflict)
    );
    let changed_fence = JobControlCommand::new(
        "demo",
        &job_id,
        "operator_a",
        "shared_key",
        after_cancel,
        Some(1),
        JobControlKind::Pause,
    )
    .expect("formed command");
    assert_eq!(
        store.control_job(changed_fence, 1_011),
        Err(DurableJobError::ControlIdempotencyConflict)
    );
    let pause = JobControlCommand::new(
        "demo",
        &job_id,
        "operator_a",
        "new_key",
        after_cancel,
        None,
        JobControlKind::Pause,
    )
    .expect("formed command");
    assert_eq!(
        store
            .control_job(pause, 1_012)
            .expect("terminal receipt")
            .outcome(),
        &JobControlOutcome::AlreadyTerminal
    );
}

#[test]
fn recovery_of_an_already_queued_job_is_a_noop_for_control_version() {
    let (mut store, job_id) = admitted_store("recovery_queued_001");
    let before = store.control_version("demo", &job_id).expect("version");
    assert_eq!(
        store
            .recover_after_restart("demo", &job_id, 1_010)
            .expect("queued recovery"),
        RecoveryDisposition::ReadyForLease,
    );
    assert_eq!(store.control_version("demo", &job_id), Ok(before));
}

#[test]
fn expired_no_effect_lease_recovery_bumps_once_then_repeated_recovery_is_a_noop() {
    let (mut store, job_id) = admitted_store("recovery_expired_001");
    store
        .acquire_lease("demo", &job_id, "worker_a", 1_010, 10)
        .expect("lease");
    let before_recovery = store.control_version("demo", &job_id).expect("version");
    assert_eq!(
        store
            .recover_after_restart("demo", &job_id, 1_020)
            .expect("expired lease recovery"),
        RecoveryDisposition::ReadyForLease,
    );
    let after_first_recovery = store.control_version("demo", &job_id).expect("version");
    assert_eq!(after_first_recovery, before_recovery + 1);
    assert_eq!(store.active_fence("demo", &job_id), Ok(None));
    assert_eq!(
        store
            .recover_after_restart("demo", &job_id, 1_021)
            .expect("queued repeat recovery"),
        RecoveryDisposition::ReadyForLease,
    );
    assert_eq!(
        store.control_version("demo", &job_id),
        Ok(after_first_recovery)
    );
}

fn admitted_store(trigger: &str) -> (DurableJobStore, String) {
    let mut store = DurableJobStore::new();
    store
        .configure_quota(QuotaLimit::new(window("demo"), 10).expect("limit"))
        .expect("quota");
    let receipt = store.admit(request("demo", trigger)).expect("admitted");
    let job_id = receipt.job().expect("admitted job").job_id().to_owned();
    (store, job_id)
}

fn control(
    store: &DurableJobStore,
    job_id: &str,
    operator: &str,
    key: &str,
    kind: JobControlKind,
) -> JobControlCommand {
    JobControlCommand::new(
        "demo",
        job_id,
        operator,
        key,
        store.control_version("demo", job_id).expect("version"),
        store.active_fence("demo", job_id).expect("fence"),
        kind,
    )
    .expect("valid control command")
}
