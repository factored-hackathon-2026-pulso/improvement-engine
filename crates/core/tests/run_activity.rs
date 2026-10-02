use improvement_engine_core::run_activity::{
    ActivityApiStatus, ActivityCursorSigner, ActivityEntry, ActivityEvent, ActivityKind,
    ActivityReadQuery, ActivityTimeline, AuthenticatedTenant, InMemoryCursorRegistry,
    ListRunActivityRequest, ProjectionActivityStream, RunActivityError, RunActivityHandler,
    RunActivityProjection, RunActivityReadModel, RunActivityStreamPort, StreamRunActivityRequest,
};
use std::cell::RefCell;

const TENANT: &str = "tenant_a";
const JOB: &str = "job:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn tenant(value: &str) -> AuthenticatedTenant {
    AuthenticatedTenant::new(value).unwrap()
}
fn request(tenant_id: &str, cursor: Option<String>, page_size: usize) -> ListRunActivityRequest {
    ListRunActivityRequest::new(tenant(tenant_id), JOB, cursor, page_size).unwrap()
}
fn event(tenant_id: &str, event_id: &str, at: u64, kind: ActivityKind) -> ActivityEvent {
    ActivityEvent::new(tenant_id, JOB, event_id, at, kind, DIGEST).unwrap()
}
fn handler<'a>(
    projection: &'a RunActivityProjection,
) -> RunActivityHandler<'a, RunActivityProjection> {
    RunActivityHandler::new(
        projection,
        ActivityCursorSigner::new("test-secret").unwrap(),
    )
}

#[test]
fn caller_pages_one_runs_activity_once_in_stable_order_with_server_side_opaque_cursor() {
    let mut projection = RunActivityProjection::new();
    projection
        .project(event(TENANT, "event_b", 20, ActivityKind::LeaseAcquired))
        .unwrap();
    projection
        .project(event(TENANT, "event_a", 10, ActivityKind::Admitted))
        .unwrap();
    projection
        .project(event(
            TENANT,
            "event_c",
            20,
            ActivityKind::EffectDispatchStarted,
        ))
        .unwrap();
    let mut handler = handler(&projection);
    let first = handler.list(request(TENANT, None, 2)).unwrap();
    assert_eq!(
        first
            .entries()
            .iter()
            .map(|entry| entry.event_id())
            .collect::<Vec<_>>(),
        vec!["event_a", "event_b"]
    );
    let cursor = first.next_cursor().unwrap().to_owned();
    assert!(!cursor.contains(TENANT));
    assert!(!cursor.contains(JOB));
    assert_eq!(handler.cursor_count(), 1);
    let second = handler.list(request(TENANT, Some(cursor), 2)).unwrap();
    assert_eq!(
        second
            .entries()
            .iter()
            .map(|entry| entry.event_id())
            .collect::<Vec<_>>(),
        vec!["event_c"]
    );
}

#[test]
fn tenant_binding_prevents_cross_tenant_read_or_cursor_replay_without_leakage() {
    let mut projection = RunActivityProjection::new();
    projection
        .project(event(TENANT, "event_a", 10, ActivityKind::Admitted))
        .unwrap();
    projection
        .project(event(TENANT, "event_b", 20, ActivityKind::LeaseAcquired))
        .unwrap();
    let mut handler = handler(&projection);
    let cursor = handler
        .list(request(TENANT, None, 1))
        .unwrap()
        .next_cursor()
        .unwrap()
        .to_owned();
    assert_eq!(
        handler.list(request("tenant_b", None, 1)),
        Err(RunActivityError::RunNotFound)
    );
    assert_eq!(
        handler.list(request("tenant_b", Some(cursor), 1)),
        Err(RunActivityError::InvalidCursor)
    );
}

#[test]
fn stale_purged_and_unknown_cursor_have_explicit_statuses() {
    let projection = RefCell::new(RunActivityProjection::new());
    projection
        .borrow_mut()
        .project(event(TENANT, "event_a", 10, ActivityKind::Admitted))
        .unwrap();
    projection
        .borrow_mut()
        .project(event(TENANT, "event_b", 20, ActivityKind::LeaseAcquired))
        .unwrap();
    let mut handler = RunActivityHandler::new(
        &projection,
        ActivityCursorSigner::new("test-secret").unwrap(),
    );
    let cursor = handler
        .list(request(TENANT, None, 1))
        .unwrap()
        .next_cursor()
        .unwrap()
        .to_owned();
    projection
        .borrow_mut()
        .project(event(
            TENANT,
            "event_c",
            30,
            ActivityKind::EffectDispatchStarted,
        ))
        .unwrap();
    let expired = handler
        .list(request(TENANT, Some(cursor.clone()), 1))
        .unwrap_err();
    assert_eq!(expired, RunActivityError::CursorExpired);
    assert_eq!(expired.status(), ActivityApiStatus::Gone);
    projection
        .borrow_mut()
        .purge_before_revision(TENANT, JOB, 3)
        .unwrap();
    let purged = handler.list(request(TENANT, Some(cursor), 1)).unwrap_err();
    assert_eq!(purged, RunActivityError::CursorPurged);
    assert_eq!(purged.status(), ActivityApiStatus::Gone);
    assert_eq!(
        handler.list(request(TENANT, Some("cursor:v1:unknown".to_owned()), 1)),
        Err(RunActivityError::InvalidCursor)
    );
}

#[test]
fn retention_deletes_events_from_new_queries_and_isolated_by_tenant_run() {
    let mut projection = RunActivityProjection::new();
    for tenant_id in [TENANT, "tenant_b"] {
        projection
            .project(event(tenant_id, "event_a", 10, ActivityKind::Admitted))
            .unwrap();
        projection
            .project(event(tenant_id, "event_b", 20, ActivityKind::LeaseAcquired))
            .unwrap();
    }
    projection.purge_before_revision(TENANT, JOB, 1).unwrap();
    let mut handler = handler(&projection);
    assert_eq!(
        handler
            .list(request(TENANT, None, 10))
            .unwrap()
            .entries()
            .iter()
            .map(|e| e.event_id())
            .collect::<Vec<_>>(),
        vec!["event_b"]
    );
    assert_eq!(
        handler
            .list(request("tenant_b", None, 10))
            .unwrap()
            .entries()
            .len(),
        2
    );
}

#[test]
fn duplicate_conflicting_events_and_reads_preserve_projection_invariants() {
    let mut projection = RunActivityProjection::new();
    let later = event(TENANT, "event_b", 20, ActivityKind::LeaseAcquired);
    projection.project(later.clone()).unwrap();
    projection.project(later).unwrap();
    let revision = projection.revision(TENANT, JOB);
    assert_eq!(
        projection.project(event(TENANT, "event_b", 20, ActivityKind::Admitted)),
        Err(RunActivityError::EventConflict)
    );
    let mut handler = handler(&projection);
    handler.list(request(TENANT, None, 10)).unwrap();
    assert_eq!(projection.revision(TENANT, JOB), revision);
}

#[test]
fn concrete_resumable_stream_emits_a_run_activity_batch_without_mutating_projection() {
    let mut projection = RunActivityProjection::new();
    projection
        .project(event(TENANT, "event_a", 10, ActivityKind::Admitted))
        .unwrap();
    let revision = projection.revision(TENANT, JOB);
    let mut stream = ProjectionActivityStream::new(handler(&projection));
    let batch = stream
        .next_batch(StreamRunActivityRequest::new(request(TENANT, None, 10)))
        .unwrap();
    assert_eq!(batch.event, "run.activity");
    assert_eq!(batch.page.entries().len(), 1);
    assert_eq!(projection.revision(TENANT, JOB), revision);
}

#[derive(Default)]
struct ExternalReadModel;
impl RunActivityReadModel for ExternalReadModel {
    fn read_activity(
        &self,
        query: &ActivityReadQuery,
    ) -> Result<ActivityTimeline, RunActivityError> {
        assert_eq!(query.tenant().tenant_id(), TENANT);
        assert_eq!(query.limit(), 1);
        assert!(query.after().is_none());
        ActivityTimeline::new(
            1,
            0,
            vec![ActivityEntry::new(
                "event_a",
                10,
                ActivityKind::Admitted,
                DIGEST,
            )?],
            false,
        )
    }
}

#[test]
fn external_adapters_can_implement_the_public_bounded_read_model_contract() {
    let mut handler = RunActivityHandler::new(
        &ExternalReadModel,
        ActivityCursorSigner::new("test-secret").unwrap(),
    );
    assert_eq!(
        handler.list(request(TENANT, None, 1)).unwrap().entries()[0].event_id(),
        "event_a"
    );
}

#[test]
fn signer_debug_redacts_secret_and_request_rejects_untrusted_tenant_input() {
    assert!(
        !format!("{:?}", ActivityCursorSigner::new("do-not-log").unwrap()).contains("do-not-log")
    );
    assert_eq!(
        AuthenticatedTenant::new("bad tenant"),
        Err(RunActivityError::InvalidTenantBinding)
    );
}

#[test]
fn rejects_impossible_empty_has_more_and_adapter_pages_larger_than_request() {
    assert_eq!(
        ActivityTimeline::new(1, 0, vec![], true),
        Err(RunActivityError::InvalidReadModelResponse)
    );
    struct Oversized;
    impl RunActivityReadModel for Oversized {
        fn read_activity(
            &self,
            _: &ActivityReadQuery,
        ) -> Result<ActivityTimeline, RunActivityError> {
            ActivityTimeline::new(
                1,
                0,
                vec![
                    ActivityEntry::new("event_a", 10, ActivityKind::Admitted, DIGEST)?,
                    ActivityEntry::new("event_b", 20, ActivityKind::LeaseAcquired, DIGEST)?,
                ],
                false,
            )
        }
    }
    let mut handler = RunActivityHandler::new(
        &Oversized,
        ActivityCursorSigner::new("test-secret").unwrap(),
    );
    assert_eq!(
        handler.list(request(TENANT, None, 1)),
        Err(RunActivityError::InvalidReadModelResponse)
    );
}

#[test]
fn purge_beyond_head_fails_without_poisoning_a_later_event_or_cursor() {
    let mut projection = RunActivityProjection::new();
    projection
        .project(event(TENANT, "event_a", 10, ActivityKind::Admitted))
        .unwrap();
    assert_eq!(
        projection.purge_before_revision(TENANT, JOB, 2),
        Err(RunActivityError::InvalidPurgeRevision)
    );
    projection
        .project(event(TENANT, "event_b", 20, ActivityKind::LeaseAcquired))
        .unwrap();
    let mut handler = handler(&projection);
    assert!(
        handler
            .list(request(TENANT, None, 1))
            .unwrap()
            .next_cursor()
            .is_some()
    );
}

#[test]
fn in_memory_registry_expires_and_evicts_without_unbounded_cursor_growth() {
    let mut projection = RunActivityProjection::new();
    projection
        .project(event(TENANT, "event_a", 10, ActivityKind::Admitted))
        .unwrap();
    projection
        .project(event(TENANT, "event_b", 20, ActivityKind::LeaseAcquired))
        .unwrap();
    let registry =
        InMemoryCursorRegistry::new(ActivityCursorSigner::new("test-secret").unwrap(), 1, 10)
            .unwrap();
    let mut handler = RunActivityHandler::with_cursor_registry(&projection, registry);
    let first = handler
        .list(request(TENANT, None, 1))
        .unwrap()
        .next_cursor()
        .unwrap()
        .to_owned();
    let _second = handler
        .list(request(TENANT, None, 1))
        .unwrap()
        .next_cursor()
        .unwrap()
        .to_owned();
    assert_eq!(handler.cursor_count(), 1);
    assert_eq!(
        handler.list(request(TENANT, Some(first), 1)),
        Err(RunActivityError::CursorExpired)
    );
}

#[test]
fn durable_cursor_record_round_trips_and_invalid_timeline_metadata_is_rejected() {
    let position =
        improvement_engine_core::run_activity::ActivityPosition::new(10, "event_a").unwrap();
    let record = improvement_engine_core::run_activity::CursorRecord::new(
        TENANT.to_owned(),
        JOB.to_owned(),
        1,
        position,
    )
    .unwrap();
    let serialized = serde_json::to_string(&record).unwrap();
    let restored: improvement_engine_core::run_activity::CursorRecord =
        serde_json::from_str(&serialized).unwrap();
    assert_eq!(restored.tenant_id(), TENANT);
    assert_eq!(restored.job_id(), JOB);
    assert_eq!(restored.snapshot_revision(), 1);
    assert_eq!(
        ActivityTimeline::new(1, 2, vec![], false),
        Err(RunActivityError::InvalidReadModelResponse)
    );
    for malformed in [
        format!(
            "{{\"tenant_id\":\"{TENANT}\",\"job_id\":\"{JOB}\",\"snapshot_revision\":0,\"position\":{{\"occurred_at_unix_seconds\":10,\"event_id\":\"event_a\"}}}}"
        ),
        format!(
            "{{\"tenant_id\":\"bad tenant\",\"job_id\":\"{JOB}\",\"snapshot_revision\":1,\"position\":{{\"occurred_at_unix_seconds\":10,\"event_id\":\"event_a\"}}}}"
        ),
        format!(
            "{{\"tenant_id\":\"{TENANT}\",\"job_id\":\"not-a-job\",\"snapshot_revision\":1,\"position\":{{\"occurred_at_unix_seconds\":10,\"event_id\":\"event_a\"}}}}"
        ),
        format!(
            "{{\"tenant_id\":\"{TENANT}\",\"job_id\":\"{JOB}\",\"snapshot_revision\":1,\"position\":{{\"occurred_at_unix_seconds\":10,\"event_id\":\"bad event\"}}}}"
        ),
    ] {
        assert!(
            serde_json::from_str::<improvement_engine_core::run_activity::CursorRecord>(&malformed)
                .is_err()
        );
    }
}

#[test]
fn rejects_adapter_response_that_does_not_strictly_advance_after_cursor() {
    struct Regressive;
    impl RunActivityReadModel for Regressive {
        fn read_activity(
            &self,
            query: &ActivityReadQuery,
        ) -> Result<ActivityTimeline, RunActivityError> {
            let entry = ActivityEntry::new("event_a", 10, ActivityKind::Admitted, DIGEST)?;
            ActivityTimeline::new(1, 0, vec![entry], query.after().is_none())
        }
    }
    let mut handler = RunActivityHandler::new(
        &Regressive,
        ActivityCursorSigner::new("test-secret").unwrap(),
    );
    let cursor = handler
        .list(request(TENANT, None, 1))
        .unwrap()
        .next_cursor()
        .unwrap()
        .to_owned();
    assert_eq!(
        handler.list(request(TENANT, Some(cursor), 1)),
        Err(RunActivityError::InvalidReadModelResponse)
    );
}
