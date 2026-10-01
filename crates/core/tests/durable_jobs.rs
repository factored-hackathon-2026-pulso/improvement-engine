use improvement_engine_core::durable_jobs::{
    DurableJobError, DurableJobRepository, DurableJobStore, JobAdmissionRequest, JobEffectState,
    JobStatus, ReconciliationEvidence, RecoveryDisposition,
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
