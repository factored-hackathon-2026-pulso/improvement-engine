use improvement_engine_core::debug_console::{DebugConsole, DebugConsoleApi, DebugTimelineStatus};
use improvement_engine_core::run_activity::{
    ActivityCursorSigner, ActivityEvent, ActivityKind, AuthenticatedTenant, ListRunActivityRequest,
    RunActivityProjection,
};
use std::cell::RefCell;

const TENANT: &str = "tenant_a";
const JOB: &str = "job:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn tenant() -> AuthenticatedTenant {
    AuthenticatedTenant::new(TENANT).expect("valid authenticated tenant")
}

#[test]
fn engineer_reads_a_tenant_bound_timeline_without_raw_evidence() {
    let mut projection = RunActivityProjection::new();
    projection
        .project(
            ActivityEvent::new(
                TENANT,
                JOB,
                "event_admitted",
                10,
                ActivityKind::Admitted,
                DIGEST,
            )
            .expect("valid activity event"),
        )
        .expect("project event");
    let request = ListRunActivityRequest::new(tenant(), JOB, None, 100).expect("valid request");
    let mut console = DebugConsole::new(
        &projection,
        ActivityCursorSigner::new("test-secret").expect("signer"),
    );

    let timeline = console.timeline(request).expect("read-only timeline");

    assert_eq!(timeline.run_id(), JOB);
    assert_eq!(timeline.events().len(), 1);
    assert_eq!(timeline.events()[0].event_id(), "event_admitted");
    assert_eq!(timeline.events()[0].kind(), &ActivityKind::Admitted);
    assert_eq!(timeline.events()[0].evidence_digest(), DIGEST);
    assert!(
        timeline.events()[0]
            .accessible_summary()
            .contains("admitted")
    );
}

#[test]
fn console_resumes_only_with_its_opaque_cursor_and_never_leaks_another_tenant() {
    let mut projection = RunActivityProjection::new();
    for (event_id, at) in [("event_first", 10), ("event_second", 20)] {
        projection
            .project(
                ActivityEvent::new(TENANT, JOB, event_id, at, ActivityKind::Admitted, DIGEST)
                    .expect("valid activity event"),
            )
            .expect("project event");
    }
    let mut console = DebugConsole::new(
        &projection,
        ActivityCursorSigner::new("test-secret").expect("signer"),
    );

    let first = console
        .timeline(ListRunActivityRequest::new(tenant(), JOB, None, 1).expect("request"))
        .expect("first page");
    let cursor = first.next_cursor().expect("continuation").to_owned();
    let resumed = console
        .timeline(
            ListRunActivityRequest::new(tenant(), JOB, Some(cursor.clone()), 1).expect("request"),
        )
        .expect("resumed page");

    assert_eq!(resumed.events()[0].event_id(), "event_second");
    let foreign = AuthenticatedTenant::new("tenant_b").expect("other authenticated tenant");
    assert!(
        console
            .timeline(ListRunActivityRequest::new(foreign, JOB, Some(cursor), 1).expect("request"))
            .is_err()
    );
}

#[test]
fn control_api_reports_a_resumable_gap_instead_of_inventing_a_timeline() {
    let projection = RefCell::new(RunActivityProjection::new());
    for (event_id, at) in [("event_first", 10), ("event_second", 20)] {
        projection
            .borrow_mut()
            .project(
                ActivityEvent::new(TENANT, JOB, event_id, at, ActivityKind::Admitted, DIGEST)
                    .expect("valid activity event"),
            )
            .expect("project event");
    }
    let console = DebugConsole::new(
        &projection,
        ActivityCursorSigner::new("test-secret").expect("signer"),
    );
    let mut api = DebugConsoleApi::new(console);
    let first =
        api.read_timeline(ListRunActivityRequest::new(tenant(), JOB, None, 1).expect("request"));
    let cursor = first.next_cursor().expect("continuation").to_owned();
    projection
        .borrow_mut()
        .project(
            ActivityEvent::new(
                TENANT,
                JOB,
                "event_third",
                30,
                ActivityKind::Admitted,
                DIGEST,
            )
            .expect("valid activity event"),
        )
        .expect("advance projection revision");

    let gap = api.read_timeline(
        ListRunActivityRequest::new(tenant(), JOB, Some(cursor), 1).expect("request"),
    );

    assert_eq!(gap.status(), DebugTimelineStatus::Gone);
    assert!(gap.timeline().is_none());
    assert!(gap.next_cursor().is_none());
}

#[test]
fn read_only_api_neither_projects_events_nor_exposes_a_missing_run() {
    let mut projection = RunActivityProjection::new();
    projection
        .project(
            ActivityEvent::new(
                TENANT,
                JOB,
                "event_admitted",
                10,
                ActivityKind::Admitted,
                DIGEST,
            )
            .expect("valid activity event"),
        )
        .expect("project event");
    let revision_before = projection.revision(TENANT, JOB).expect("run revision");
    let console = DebugConsole::new(
        &projection,
        ActivityCursorSigner::new("test-secret").expect("signer"),
    );
    let mut api = DebugConsoleApi::new(console);

    let missing = api.read_timeline(
        ListRunActivityRequest::new(
            tenant(),
            "job:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            None,
            1,
        )
        .expect("request"),
    );

    assert_eq!(missing.status(), DebugTimelineStatus::NotFound);
    assert!(missing.timeline().is_none());
    assert_eq!(projection.revision(TENANT, JOB), Some(revision_before));
}
