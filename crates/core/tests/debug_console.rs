use improvement_engine_core::debug_console::{
    DebugAuthenticationError, DebugAuthenticationRequest, DebugConsole, DebugConsoleApi,
    DebugIdentityPort, DebugTimelineRequest, DebugTimelineStatus, DebugViewer, DebugViewerIssuer,
};
use improvement_engine_core::run_activity::{
    ActivityCursorSigner, ActivityEvent, ActivityKind, AuthenticatedTenant, RunActivityProjection,
};
use std::cell::RefCell;

const TENANT: &str = "tenant_a";
const JOB: &str = "job:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct StaticIdentity {
    tenant_id: &'static str,
}

impl DebugIdentityPort for StaticIdentity {
    fn authenticate(
        &mut self,
        _: DebugAuthenticationRequest,
        issuer: &DebugViewerIssuer,
    ) -> Result<DebugViewer, DebugAuthenticationError> {
        Ok(issuer.issue(AuthenticatedTenant::new(self.tenant_id).expect("trusted identity")))
    }
}

struct DenyingIdentity;

impl DebugIdentityPort for DenyingIdentity {
    fn authenticate(
        &mut self,
        _: DebugAuthenticationRequest,
        _: &DebugViewerIssuer,
    ) -> Result<DebugViewer, DebugAuthenticationError> {
        Err(DebugAuthenticationError::Denied)
    }
}

struct SessionIdentity;

impl DebugIdentityPort for SessionIdentity {
    fn authenticate(
        &mut self,
        request: DebugAuthenticationRequest,
        issuer: &DebugViewerIssuer,
    ) -> Result<DebugViewer, DebugAuthenticationError> {
        let tenant_id = match request.session_ref() {
            "debug_session_a" => TENANT,
            "debug_session_b" => "tenant_b",
            _ => return Err(DebugAuthenticationError::Denied),
        };
        Ok(issuer.issue(AuthenticatedTenant::new(tenant_id).expect("trusted identity")))
    }
}

fn auth() -> DebugAuthenticationRequest {
    DebugAuthenticationRequest::new("debug_session_a").expect("valid auth request")
}

fn foreign_auth() -> DebugAuthenticationRequest {
    DebugAuthenticationRequest::new("debug_session_b").expect("valid auth request")
}

fn debug_request(cursor: Option<String>, page_size: usize) -> DebugTimelineRequest {
    DebugTimelineRequest::new(JOB, cursor, page_size).expect("valid debug request")
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
    let console = DebugConsole::new(
        &projection,
        ActivityCursorSigner::new("test-secret").expect("signer"),
    );
    let mut api = DebugConsoleApi::new(console, StaticIdentity { tenant_id: TENANT });

    let response = api.read_timeline(auth(), debug_request(None, 100));
    let timeline = response.timeline().expect("read-only timeline");

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
    let console = DebugConsole::new(
        &projection,
        ActivityCursorSigner::new("test-secret").expect("signer"),
    );
    let mut api = DebugConsoleApi::new(console, SessionIdentity);

    let first = api.read_timeline(auth(), debug_request(None, 1));
    let cursor = first.next_cursor().expect("continuation").to_owned();
    let resumed = api.read_timeline(auth(), debug_request(Some(cursor.clone()), 1));
    let resumed_timeline = resumed.timeline().expect("resumed page");

    assert_eq!(resumed_timeline.events()[0].event_id(), "event_second");
    let foreign = api.read_timeline(foreign_auth(), debug_request(Some(cursor), 1));
    assert_eq!(foreign.status(), DebugTimelineStatus::BadRequest);
    assert!(foreign.timeline().is_none());
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
    let mut api = DebugConsoleApi::new(console, StaticIdentity { tenant_id: TENANT });
    let first = api.read_timeline(auth(), debug_request(None, 1));
    let cursor = first.next_cursor().expect("continuation").to_owned();
    assert_eq!(
        first.accessible_status_summary(),
        "Timeline loaded: 1 event; continuation available."
    );
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

    let gap = api.read_timeline(auth(), debug_request(Some(cursor), 1));

    assert_eq!(gap.status(), DebugTimelineStatus::Gone);
    assert!(gap.timeline().is_none());
    assert!(gap.next_cursor().is_none());
    assert_eq!(
        gap.accessible_status_summary(),
        "Timeline changed; reload the current snapshot to continue."
    );
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
    let mut api = DebugConsoleApi::new(console, StaticIdentity { tenant_id: TENANT });

    let missing = api.read_timeline(
        auth(),
        DebugTimelineRequest::new(
            "job:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            None,
            1,
        )
        .expect("request"),
    );

    assert_eq!(missing.status(), DebugTimelineStatus::NotFound);
    assert!(missing.timeline().is_none());
    assert_eq!(missing.accessible_status_summary(), "Run not available.");
    assert_eq!(projection.revision(TENANT, JOB), Some(revision_before));
}

#[test]
fn debug_transport_derives_tenant_from_trusted_identity_not_a_caller_request() {
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
    let console = DebugConsole::new(
        &projection,
        ActivityCursorSigner::new("test-secret").expect("signer"),
    );
    let mut api = DebugConsoleApi::new(console, StaticIdentity { tenant_id: TENANT });

    // `AuthenticatedTenant::new("tenant_b")` remains U07's transport DTO for
    // existing callers, but DebugConsoleApi has no parameter through which an
    // untrusted caller can inject it.
    let response = api.read_timeline(auth(), debug_request(None, 1));

    assert_eq!(response.status(), DebugTimelineStatus::Ok);
    assert_eq!(
        response.accessible_status_summary(),
        "Timeline loaded: 1 event; no continuation."
    );
}

#[test]
fn denied_identity_returns_a_stable_safe_bad_request_summary() {
    let projection = RunActivityProjection::new();
    let console = DebugConsole::new(
        &projection,
        ActivityCursorSigner::new("test-secret").expect("signer"),
    );
    let mut api = DebugConsoleApi::new(console, DenyingIdentity);

    let response = api.read_timeline(auth(), debug_request(None, 1));

    assert_eq!(response.status(), DebugTimelineStatus::BadRequest);
    assert!(response.timeline().is_none());
    assert_eq!(
        response.accessible_status_summary(),
        "Timeline request cannot be processed."
    );
}
