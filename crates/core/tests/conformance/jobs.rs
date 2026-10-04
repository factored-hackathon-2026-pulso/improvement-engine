//! Backend-neutral durable-job conformance cases.
//!
//! The behavioral assertions use only `DurableJobRepository`. `JobFixture`
//! supplies backend-specific construction, seeding and read-only inspection
//! hooks; a future adapter can run the same cases without changing them.
//! This suite is sequential: it proves the in-memory contract's observable
//! claim behavior, not database atomicity or concurrent-worker guarantees.

use improvement_engine_core::durable_jobs::{
    ClaimedJob, DurableJobError, DurableJobRepository, DurableJobStore, JobAdmissionRequest,
    JobControlCommand, JobControlKind, JobControlOutcome, JobEffectState, JobStatus,
    ReconciliationEvidence,
};
use improvement_engine_core::quota_grant::{
    AuthorizedGrant, QuotaLimit, QuotaReservation, QuotaResource, QuotaWindow,
};
use improvement_engine_core::run_config::{Cadence, EligibleSource, RunConfig, ScanBudget};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const TENANT: &str = "tenant_a";
const WORKER_A: &str = "worker_a";
const WORKER_B: &str = "worker_b";
const NOW: u64 = 100;
const LEASE: u64 = 30;

/// Backend-owned fixture callbacks. The conformance logic below is generic
/// over `DurableJobRepository`; adapters only bridge their test setup and
/// read-only state inspection, neither of which is part of the production
/// repository port.
trait JobFixture {
    type Repository: DurableJobRepository;

    fn new_repository(quota_limit: u64) -> Self::Repository;
    fn admit(repository: &mut Self::Repository, tenant: &str, trigger: &str) -> Option<String>;
    fn inspect(repository: &Self::Repository, tenant: &str, job_id: &str) -> JobSnapshot;
    fn trust_reconciler(repository: &mut Self::Repository, tenant: &str, authority: &str);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JobSnapshot {
    status: JobStatus,
    effect_state: JobEffectState,
    control_version: u64,
    active_fence: Option<u64>,
}

struct MemoryFixture;

impl JobFixture for MemoryFixture {
    type Repository = DurableJobStore;

    fn new_repository(quota_limit: u64) -> Self::Repository {
        let mut repository = DurableJobStore::new();
        for tenant in [TENANT, "tenant-a", "tenant-b", "demo"] {
            let resource = QuotaResource::new("job_runs").expect("resource");
            let window = QuotaWindow::new(tenant, resource, 0, 10_000).expect("window");
            repository
                .configure_quota(QuotaLimit::new(window, quota_limit).expect("limit"))
                .expect("quota configuration");
        }
        repository
    }

    fn admit(repository: &mut Self::Repository, tenant: &str, trigger: &str) -> Option<String> {
        repository
            .admit_atomically(request(tenant, trigger))
            .expect("fixture admission")
            .job()
            .map(|job| job.job_id().to_owned())
    }

    fn inspect(repository: &Self::Repository, tenant: &str, job_id: &str) -> JobSnapshot {
        JobSnapshot {
            status: repository.status(tenant, job_id).expect("visible status"),
            effect_state: repository
                .effect_state(tenant, job_id)
                .expect("visible effect state"),
            control_version: repository
                .control_version(tenant, job_id)
                .expect("visible version"),
            active_fence: repository
                .active_fence(tenant, job_id)
                .expect("visible fence"),
        }
    }

    fn trust_reconciler(repository: &mut Self::Repository, tenant: &str, authority: &str) {
        repository
            .trust_reconciliation_authority(tenant, authority)
            .expect("fixture reconciliation authority");
    }
}

fn config() -> RunConfig {
    RunConfig::new(
        1,
        Cadence::EventTriggered,
        ScanBudget::new(1, 100, 1_024).expect("budget"),
        vec![EligibleSource::new("latam_bank", "contacts").expect("source")],
    )
    .expect("run config")
}

fn request(tenant: &str, trigger: &str) -> JobAdmissionRequest {
    let config = config();
    let resource = QuotaResource::new("job_runs").expect("resource");
    let window = QuotaWindow::new(tenant, resource.clone(), 0, 10_000).expect("window");
    let grant = AuthorizedGrant::issue("grant_1", "policy_1", tenant, resource, 100, 10_000)
        .expect("grant");
    let reservation =
        QuotaReservation::new(trigger, window, &config, grant, 1, 1).expect("reservation");
    JobAdmissionRequest::new(
        tenant,
        trigger,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        config,
        reservation,
    )
    .expect("admission request")
}

fn claim<R: DurableJobRepository>(
    repository: &mut R,
    tenant: &str,
    worker: &str,
    now: u64,
    lease: u64,
) -> Result<Option<ClaimedJob>, DurableJobError> {
    repository.claim_next_job(tenant, worker, now, lease)
}

fn pause_or_cancel<R: DurableJobRepository>(
    repository: &mut R,
    tenant: &str,
    job_id: &str,
    kind: JobControlKind,
    key: &str,
) -> JobControlOutcome {
    let command = JobControlCommand::new(tenant, job_id, "operator_a", key, 1, None, kind)
        .expect("control command");
    repository
        .control_job_atomically(command, NOW)
        .expect("control transition")
        .outcome()
        .clone()
}

#[test]
fn two_sequential_workers_claim_distinct_jobs_in_admission_order() {
    conformance_two_sequential_workers_claim_distinct_jobs::<MemoryFixture>();
}

fn conformance_two_sequential_workers_claim_distinct_jobs<B: JobFixture>() {
    let mut repository = B::new_repository(10);
    let first_id = B::admit(&mut repository, TENANT, "first_trigger").expect("first admitted");
    let second_id = B::admit(&mut repository, TENANT, "second_trigger").expect("second admitted");
    assert_ne!(first_id, second_id);
    let before_first = B::inspect(&repository, TENANT, &first_id);
    let before_second = B::inspect(&repository, TENANT, &second_id);

    let first = claim(&mut repository, TENANT, WORKER_A, NOW, LEASE)
        .expect("first claim result")
        .expect("oldest job claimed");
    let second = claim(&mut repository, TENANT, WORKER_B, NOW, LEASE)
        .expect("second claim result")
        .expect("next job claimed");
    assert_eq!(
        first.job_id(),
        first_id,
        "claim order follows admission, not ID ordering"
    );
    assert_eq!(second.job_id(), second_id);
    assert_ne!(
        first.job_id(),
        second.job_id(),
        "sequential claims cannot double-claim"
    );
    assert_eq!(first.lease().worker_id(), WORKER_A);
    assert_eq!(second.lease().worker_id(), WORKER_B);
    assert_eq!(first.lease().fence_token(), 1);
    assert_eq!(second.lease().fence_token(), 1);

    let first_after = B::inspect(&repository, TENANT, &first_id);
    let second_after = B::inspect(&repository, TENANT, &second_id);
    assert_eq!(leased_attempt(&first_after.status), Some(1));
    assert_eq!(leased_attempt(&second_after.status), Some(1));
    assert_eq!(
        first_after.control_version,
        before_first.control_version + 1
    );
    assert_eq!(
        second_after.control_version,
        before_second.control_version + 1
    );
    assert_eq!(
        claim(&mut repository, TENANT, "worker_c", NOW, LEASE),
        Ok(None)
    );
    assert_eq!(B::inspect(&repository, TENANT, &first_id), first_after);
    assert_eq!(B::inspect(&repository, TENANT, &second_id), second_after);
}

#[test]
fn expired_lease_is_reclaimed_once_with_monotonic_fence_attempt_and_version() {
    conformance_expired_lease_reclaim::<MemoryFixture>();
}

fn conformance_expired_lease_reclaim<B: JobFixture>() {
    let mut repository = B::new_repository(10);
    let job_id = B::admit(&mut repository, TENANT, "reclaim_trigger").expect("admitted");
    let before = B::inspect(&repository, TENANT, &job_id);
    let first = claim(&mut repository, TENANT, WORKER_A, NOW, LEASE)
        .expect("initial claim")
        .expect("job");
    assert_eq!(first.lease().expires_at_unix_seconds(), NOW + LEASE);
    assert_eq!(first.lease().fence_token(), 1);
    let first_snapshot = B::inspect(&repository, TENANT, &job_id);
    assert_eq!(leased_attempt(&first_snapshot.status), Some(1));
    assert_eq!(first_snapshot.control_version, before.control_version + 1);

    assert_eq!(
        claim(&mut repository, TENANT, WORKER_B, NOW + LEASE - 1, LEASE),
        Ok(None),
        "lease remains active immediately before expiry"
    );
    assert_eq!(B::inspect(&repository, TENANT, &job_id), first_snapshot);

    let later_id =
        B::admit(&mut repository, TENANT, "later_queued_trigger").expect("later job admitted");
    let later_before = B::inspect(&repository, TENANT, &later_id);
    let reclaimed = claim(&mut repository, TENANT, WORKER_B, NOW + LEASE, LEASE)
        .expect("expired claim")
        .expect("reclaimed job");
    assert_eq!(reclaimed.job_id(), job_id);
    assert_eq!(reclaimed.lease().fence_token(), 2);
    let reclaimed_snapshot = B::inspect(&repository, TENANT, &job_id);
    assert_eq!(leased_attempt(&reclaimed_snapshot.status), Some(2));
    assert_eq!(
        reclaimed_snapshot.control_version,
        before.control_version + 2
    );
    assert_eq!(reclaimed_snapshot.active_fence, Some(2));
    assert_eq!(B::inspect(&repository, TENANT, &later_id), later_before);
}

#[test]
fn invalid_inputs_foreign_tenant_and_empty_queue_are_non_mutating() {
    conformance_invalid_inputs_and_noop::<MemoryFixture>();
}

fn conformance_invalid_inputs_and_noop<B: JobFixture>() {
    let mut repository = B::new_repository(10);
    let job_id = B::admit(&mut repository, TENANT, "validation_trigger").expect("admitted");
    let before = B::inspect(&repository, TENANT, &job_id);

    assert_eq!(
        claim(&mut repository, "", WORKER_A, NOW, LEASE),
        Err(DurableJobError::InvalidIdentifier { field: "tenant_id" })
    );
    assert_eq!(
        claim(&mut repository, TENANT, "", NOW, LEASE),
        Err(DurableJobError::InvalidIdentifier { field: "worker_id" })
    );
    assert_eq!(
        claim(&mut repository, TENANT, WORKER_A, NOW, 0),
        Err(DurableJobError::InvalidLeaseDuration)
    );
    assert_eq!(
        claim(&mut repository, TENANT, WORKER_A, u64::MAX, 1),
        Err(DurableJobError::InvalidLeaseDuration)
    );
    assert_eq!(
        claim(&mut repository, "tenant_b", WORKER_A, NOW, LEASE),
        Ok(None)
    );
    assert_eq!(B::inspect(&repository, TENANT, &job_id), before);

    let empty = B::new_repository(10);
    let mut empty = empty;
    assert_eq!(claim(&mut empty, TENANT, WORKER_A, NOW, LEASE), Ok(None));
}

#[test]
fn paused_cancelled_unknown_completed_acknowledged_and_deferred_jobs_are_filtered() {
    conformance_state_filters::<MemoryFixture>();
}

fn conformance_state_filters<B: JobFixture>() {
    assert_unclaimable::<B>("paused", 10, |repository, job_id| {
        assert_eq!(
            pause_or_cancel(
                repository,
                TENANT,
                job_id,
                JobControlKind::Pause,
                "pause_filter"
            ),
            JobControlOutcome::Paused
        );
    });
    assert_unclaimable::<B>("cancelled", 10, |repository, job_id| {
        assert_eq!(
            pause_or_cancel(
                repository,
                TENANT,
                job_id,
                JobControlKind::Cancel,
                "cancel_filter"
            ),
            JobControlOutcome::CancelledBeforeEffect
        );
    });
    assert_unclaimable::<B>("unknown", 10, |repository, job_id| {
        let lease = claim(repository, TENANT, WORKER_A, NOW, 1)
            .expect("claim")
            .expect("job");
        repository
            .begin_job_effect_dispatch(TENANT, job_id, WORKER_A, lease.lease().fence_token(), NOW)
            .expect("dispatch boundary");
        assert_eq!(
            repository
                .recover_job_after_restart(TENANT, job_id, NOW + 2)
                .expect("recovery"),
            improvement_engine_core::durable_jobs::RecoveryDisposition::ReconciliationRequired
        );
    });
    assert_unclaimable::<B>("completed", 10, |repository, job_id| {
        let lease = claim(repository, TENANT, WORKER_A, NOW, 1)
            .expect("claim")
            .expect("job");
        repository
            .begin_job_effect_dispatch(TENANT, job_id, WORKER_A, lease.lease().fence_token(), NOW)
            .expect("dispatch boundary");
        repository
            .recover_job_after_restart(TENANT, job_id, NOW + 2)
            .expect("recovery");
        B::trust_reconciler(repository, TENANT, "reconciler_a");
        let evidence = ReconciliationEvidence::no_effect(
            TENANT,
            job_id,
            lease.lease().fence_token(),
            "reconciler_a",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .expect("reconciliation evidence");
        repository
            .reconcile_unknown_job(TENANT, job_id, evidence)
            .expect("no-effect reconciliation");
    });
    assert_unclaimable::<B>("acknowledged", 10, |repository, job_id| {
        let lease = claim(repository, TENANT, WORKER_A, NOW, LEASE)
            .expect("claim")
            .expect("job");
        repository
            .begin_job_effect_dispatch(TENANT, job_id, WORKER_A, lease.lease().fence_token(), NOW)
            .expect("dispatch boundary");
        repository
            .acknowledge_job_effect(
                TENANT,
                job_id,
                WORKER_A,
                lease.lease().fence_token(),
                "effect:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                NOW + 1,
            )
            .expect("effect receipt");
    });
    let mut repository = B::new_repository(1);
    let first_id = B::admit(&mut repository, TENANT, "deferred_first").expect("first admitted");
    assert!(B::admit(&mut repository, TENANT, "deferred_second").is_none());
    assert_eq!(
        pause_or_cancel(
            &mut repository,
            TENANT,
            &first_id,
            JobControlKind::Pause,
            "deferred_pause"
        ),
        JobControlOutcome::Paused
    );
    let before = B::inspect(&repository, TENANT, &first_id);
    assert_eq!(
        claim(&mut repository, TENANT, WORKER_B, 500, LEASE),
        Ok(None)
    );
    assert_eq!(B::inspect(&repository, TENANT, &first_id), before);
}

fn assert_unclaimable<B: JobFixture>(
    trigger_suffix: &str,
    quota_limit: u64,
    prepare: impl FnOnce(&mut B::Repository, &str),
) {
    let mut repository = B::new_repository(quota_limit);
    let first_trigger = format!("filter_{trigger_suffix}");
    let job_id = B::admit(&mut repository, TENANT, &first_trigger).expect("admitted job");
    prepare(&mut repository, &job_id);
    let before = B::inspect(&repository, TENANT, &job_id);
    assert_eq!(
        claim(&mut repository, TENANT, WORKER_B, 500, LEASE),
        Ok(None)
    );
    assert_eq!(B::inspect(&repository, TENANT, &job_id), before);
}

fn leased_attempt(status: &JobStatus) -> Option<u64> {
    match status {
        JobStatus::Leased { attempt, .. } => Some(*attempt),
        _ => None,
    }
}

#[derive(Debug)]
struct Trace {
    name: String,
    contract_version: String,
    kind: String,
    recorded: bool,
    steps: Vec<Value>,
}

fn parse_trace(path: &PathBuf) -> Trace {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let value: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    Trace {
        name: value["name"].as_str().expect("trace name").to_owned(),
        contract_version: value["contract_version"]
            .as_str()
            .expect("contract version")
            .to_owned(),
        kind: value["kind"].as_str().expect("trace kind").to_owned(),
        recorded: value["recorded"].as_bool().expect("recorded flag"),
        steps: value["steps"].as_array().expect("trace steps").to_vec(),
    }
}

#[test]
fn all_six_frz0_c7_specification_traces_replay_against_memory_reference() {
    replay_c7_traces_against_memory_reference::<MemoryFixture>();
}

fn replay_c7_traces_against_memory_reference<B: JobFixture>() {
    let trace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/engine-steps/pack/parts/c7_claim_next/traces");
    let names = [
        "claim_oldest_first",
        "expired_lease_reclaimed_with_higher_fence",
        "invalid_lease_duration",
        "paused_and_deferred_not_claimable",
        "tenant_isolation",
        "unknown_effect_not_claimable",
    ];
    for name in names {
        let path = trace_dir.join(format!("{name}.json"));
        let trace = parse_trace(&path);
        assert_eq!(trace.name, name, "{}", path.display());
        assert_eq!(
            trace.contract_version,
            "engine-steps-pack/0",
            "{}",
            path.display()
        );
        assert_eq!(trace.kind, "specification_trace", "{}", path.display());
        assert!(
            !trace.recorded,
            "preserve FRZ0 provenance: {}",
            path.display()
        );
        replay_trace::<B>(&trace);
    }
}

fn replay_trace<B: JobFixture>(trace: &Trace) {
    let quota_limit = if trace.name == "paused_and_deferred_not_claimable" {
        1
    } else {
        10
    };
    let mut repository = B::new_repository(quota_limit);
    let mut aliases = BTreeMap::<String, (String, String)>::new();
    let mut last_admitted: Option<(String, String)> = None;
    let mut last_claim: Option<(String, String, u64)> = None;

    for (index, step) in trace.steps.iter().enumerate() {
        let operation = step["op"].as_str().expect("operation");
        match operation {
            "admit" => {
                let tenant = step["tenant"].as_str().unwrap_or("tenant-a");
                let trigger = step["trigger"].as_str().expect("trigger");
                let job_id = B::admit(&mut repository, tenant, trigger);
                let expected_admitted = step["expect"]["admitted"].as_bool().expect("admitted");
                assert_eq!(
                    job_id.is_some(),
                    expected_admitted,
                    "{} step {index}",
                    trace.name
                );
                if let Some(job_id) = job_id {
                    if let Some(alias) = step["expect"]["job"].as_str() {
                        aliases.insert(alias.to_owned(), (tenant.to_owned(), job_id.clone()));
                    }
                    last_admitted = Some((tenant.to_owned(), job_id));
                }
            }
            "control_job_atomically" => {
                let (tenant, job_id) = last_admitted.clone().expect("admit before control");
                let expected_version = B::inspect(&repository, &tenant, &job_id).control_version;
                let control_kind = match step["kind"].as_str().expect("control kind") {
                    "pause" => JobControlKind::Pause,
                    "cancel" => JobControlKind::Cancel,
                    other => panic!("unsupported control kind {other:?} in {}", trace.name),
                };
                let command = JobControlCommand::new(
                    &tenant,
                    &job_id,
                    "operator_a",
                    format!("trace_{}_step_{index}", trace.name),
                    expected_version,
                    None,
                    control_kind,
                )
                .expect("trace control command");
                let receipt = repository
                    .control_job_atomically(command, NOW)
                    .expect("trace control");
                assert_eq!(
                    status_name(receipt.confirmed_status()),
                    step["expect"]["status"].as_str().expect("expected status")
                );
            }
            "claim_next_job" => {
                let tenant = step["tenant"].as_str().unwrap_or("tenant-a");
                let worker = step["worker"].as_str().expect("worker");
                let now = step["now"].as_u64().expect("claim timestamp");
                let lease = step["lease"].as_u64().expect("lease duration");
                let expected = &step["expect"];
                let result = claim(&mut repository, tenant, worker, now, lease);
                if let Some(error) = expected.get("error").and_then(Value::as_str) {
                    let actual = result.expect_err("trace expects an error");
                    assert_eq!(format!("{actual:?}"), error, "{} step {index}", trace.name);
                } else if expected.is_null() {
                    assert_eq!(
                        result.expect("claim result"),
                        None,
                        "{} step {index}",
                        trace.name
                    );
                } else {
                    let claimed = result
                        .expect("claim result")
                        .expect("trace expects a claimed job");
                    let alias = expected["job"].as_str().expect("job alias");
                    let (alias_tenant, expected_job_id) = aliases.get(alias).expect("known alias");
                    assert_eq!(alias_tenant, tenant, "trace alias must stay tenant scoped");
                    assert_eq!(
                        claimed.job_id(),
                        expected_job_id,
                        "{} step {index}",
                        trace.name
                    );
                    assert_number_if_present(
                        expected,
                        "fence_token",
                        claimed.lease().fence_token(),
                    );
                    assert_number_if_present(
                        expected,
                        "expires_at",
                        claimed.lease().expires_at_unix_seconds(),
                    );
                    if let Some(expected_attempt) = expected.get("attempt").and_then(Value::as_u64)
                    {
                        let snapshot = B::inspect(&repository, tenant, claimed.job_id());
                        assert_eq!(leased_attempt(&snapshot.status), Some(expected_attempt));
                    }
                    last_claim = Some((
                        tenant.to_owned(),
                        claimed.job_id().to_owned(),
                        claimed.lease().fence_token(),
                    ));
                }
            }
            "begin_job_effect_dispatch" => {
                let (tenant, job_id, prior_fence) =
                    last_claim.clone().expect("claim before dispatch");
                let worker = step["worker"].as_str().expect("worker");
                let fence = step["fence_token"].as_u64().unwrap_or(prior_fence);
                let now = step["now"].as_u64().expect("dispatch time");
                let result =
                    repository.begin_job_effect_dispatch(&tenant, &job_id, worker, fence, now);
                if let Some(error) = step["expect"]["error"].as_str() {
                    assert_eq!(
                        format!("{:?}", result.expect_err("expected dispatch error")),
                        error
                    );
                } else {
                    result.expect("dispatch");
                }
            }
            "recover_job_after_restart" => {
                let (tenant, job_id) = last_admitted.clone().expect("admission before recovery");
                let now = step["now"].as_u64().expect("recovery timestamp");
                repository
                    .recover_job_after_restart(&tenant, &job_id, now)
                    .expect("recovery");
                if let Some(expected_status) = step["expect"]["status"].as_str() {
                    assert_eq!(
                        status_name(&B::inspect(&repository, &tenant, &job_id).status),
                        expected_status
                    );
                }
            }
            other => panic!(
                "unsupported operation {other:?} in {} step {index}",
                trace.name
            ),
        }
    }
}

fn assert_number_if_present(expected: &Value, field: &str, actual: u64) {
    if let Some(value) = expected.get(field).and_then(Value::as_u64) {
        assert_eq!(actual, value, "trace field {field}");
    }
}

fn status_name(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "Queued",
        JobStatus::Leased { .. } => "Leased",
        JobStatus::UnknownPendingReconciliation => "UnknownPendingReconciliation",
        JobStatus::CompletedNoEffect => "CompletedNoEffect",
        JobStatus::AppliedAcknowledged => "AppliedAcknowledged",
        JobStatus::Paused => "Paused",
        JobStatus::CancelledBeforeEffect => "CancelledBeforeEffect",
    }
}
