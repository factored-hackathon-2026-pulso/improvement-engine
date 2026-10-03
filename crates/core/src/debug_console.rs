//! Read-only console projection for engineering debugging.
//!
//! This boundary consumes the tenant-bound U07 activity read model.  It does
//! not read artifacts, source data, SQL, prompts, or platform state directly;
//! transports may render its safe projection without gaining a mutation path.

// The authenticated composition is deliberately crate-private until the
// real control-api composition root exists. It is exercised by unit tests but
// cannot become a public alternate authentication path in this slice.
#![allow(dead_code)]

use crate::run_activity::{
    ActivityApiStatus, ActivityCursorSigner, ActivityEntry, ActivityKind, AuthenticatedTenant,
    InMemoryCursorRegistry, ListRunActivityRequest, RunActivityError, RunActivityHandler,
    RunActivityReadModel,
};
use crate::run_timeline_v2::{
    RunTimelinePage, RunTimelineReadError, RunTimelineReadPort, RunTimelineRequest,
};

/// A UI-ready, safe projection of one material run transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleEvent {
    event_id: String,
    kind: ActivityKind,
    evidence_digest: String,
    occurred_at_unix_seconds: u64,
}

impl ConsoleEvent {
    fn from_activity(entry: ActivityEntry) -> Self {
        Self {
            event_id: entry.event_id().to_owned(),
            kind: entry.kind().clone(),
            evidence_digest: entry.evidence_digest().to_owned(),
            occurred_at_unix_seconds: entry.occurred_at_unix_seconds(),
        }
    }

    #[must_use]
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    #[must_use]
    pub fn kind(&self) -> &ActivityKind {
        &self.kind
    }

    #[must_use]
    pub fn evidence_digest(&self) -> &str {
        &self.evidence_digest
    }

    #[must_use]
    pub fn accessible_summary(&self) -> String {
        format!(
            "{} at {}",
            activity_label(&self.kind),
            self.occurred_at_unix_seconds
        )
    }
}

/// Bounded timeline that a debug console can render without direct data-store
/// access. The continuation remains the opaque cursor issued by U07.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleTimeline {
    run_id: String,
    events: Vec<ConsoleEvent>,
    next_cursor: Option<String>,
    revision: u64,
}

impl ConsoleTimeline {
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    #[must_use]
    pub fn events(&self) -> &[ConsoleEvent] {
        &self.events
    }

    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Read-only application boundary used by the future control API/debug UI.
///
/// Its only state is U07's short-lived cursor registry. Calling [`timeline`]
/// cannot project events, mutate a run, invoke a tool, or access source data.
pub struct DebugConsole<'a, ReadModel> {
    activity: RunActivityHandler<'a, ReadModel, InMemoryCursorRegistry>,
}

impl<'a, ReadModel: RunActivityReadModel> DebugConsole<'a, ReadModel> {
    #[must_use]
    pub fn new(read_model: &'a ReadModel, signer: ActivityCursorSigner) -> Self {
        Self {
            activity: RunActivityHandler::new(read_model, signer),
        }
    }

    fn timeline(
        &mut self,
        request: ListRunActivityRequest,
    ) -> Result<ConsoleTimeline, RunActivityError> {
        let run_id = request.job_id().to_owned();
        let page = self.activity.list(request)?;
        Ok(ConsoleTimeline {
            run_id,
            events: page
                .entries()
                .iter()
                .cloned()
                .map(ConsoleEvent::from_activity)
                .collect(),
            next_cursor: page.next_cursor().map(str::to_owned),
            revision: page.revision(),
        })
    }
}

/// Status exposed by the read-only control-API adapter. It deliberately does
/// not return raw domain errors, storage details, or a timeline from another
/// tenant/run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugTimelineStatus {
    Ok,
    BadRequest,
    NotFound,
    Gone,
    Conflict,
    Unavailable,
}

/// Transport-neutral response consumed by the engineering console.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugTimelineResponse {
    status: DebugTimelineStatus,
    timeline: Option<ConsoleTimeline>,
}

impl DebugTimelineResponse {
    #[must_use]
    pub fn status(&self) -> DebugTimelineStatus {
        self.status
    }

    #[must_use]
    pub fn timeline(&self) -> Option<&ConsoleTimeline> {
        self.timeline.as_ref()
    }

    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.timeline
            .as_ref()
            .and_then(ConsoleTimeline::next_cursor)
    }

    /// Stable assistive status with no raw error, tenant, run identifier or
    /// evidence value. A browser can announce it without exposing diagnostics.
    #[must_use]
    pub fn accessible_status_summary(&self) -> String {
        match (self.status, self.timeline.as_ref()) {
            (DebugTimelineStatus::Ok, Some(timeline)) => {
                let count = timeline.events().len();
                let noun = if count == 1 { "event" } else { "events" };
                let continuation = if timeline.next_cursor().is_some() {
                    "continuation available"
                } else {
                    "no continuation"
                };
                format!("Timeline loaded: {count} {noun}; {continuation}.")
            }
            (DebugTimelineStatus::Gone, _) => {
                "Timeline changed; reload the current snapshot to continue.".to_owned()
            }
            (DebugTimelineStatus::NotFound, _) => "Run not available.".to_owned(),
            (DebugTimelineStatus::BadRequest, _) => {
                "Timeline request cannot be processed.".to_owned()
            }
            (DebugTimelineStatus::Conflict, _) => {
                "Timeline request conflicts with current state.".to_owned()
            }
            (DebugTimelineStatus::Unavailable, _) => {
                "Timeline is temporarily unavailable.".to_owned()
            }
            (DebugTimelineStatus::Ok, None) => "Timeline request cannot be processed.".to_owned(),
        }
    }
}

/// V2 debug response over the durable run/sequence ledger. This contract is
/// distinct from `DebugTimelineResponse`, whose continuation is U07 legacy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DebugRunTimelineResponseV2 {
    status: DebugTimelineStatus,
    page: Option<RunTimelinePage>,
}

impl DebugRunTimelineResponseV2 {
    #[must_use]
    pub(crate) fn status(&self) -> DebugTimelineStatus {
        self.status
    }
    #[must_use]
    pub(crate) fn page(&self) -> Option<&RunTimelinePage> {
        self.page.as_ref()
    }
}

/// Untrusted U24 V2 read request. Deliberately contains no tenant selector.
///
/// ```compile_fail
/// use improvement_engine_core::debug_console::DebugRunTimelineRequestV2;
/// let _ = DebugRunTimelineRequestV2::new("00000000-0000-7000-8000-000000000002", 0, 10);
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DebugRunTimelineRequestV2(RunTimelineRequest);

impl DebugRunTimelineRequestV2 {
    pub(crate) fn new(
        run_ref: impl Into<String>,
        after_sequence: i64,
        page_size: usize,
    ) -> Result<Self, ()> {
        RunTimelineRequest::new(run_ref, after_sequence, page_size)
            .map(Self)
            .map_err(|_| ())
    }
}

/// Authenticated and read-only U24 V2 composer over `pulso_run_events`.
/// Authentication is delegated through the same one-call issuer capability
/// as the existing U24 boundary; the tenant comes only from `DebugViewer`.
pub(crate) struct DebugConsoleV2Api<Reader, Identity> {
    reader: Reader,
    identity: Identity,
}

impl<Reader: RunTimelineReadPort, Identity: DebugIdentityPort> DebugConsoleV2Api<Reader, Identity> {
    pub(crate) fn new(reader: Reader, identity: Identity) -> Self {
        Self { reader, identity }
    }

    pub(crate) fn read_events(
        &mut self,
        authentication: DebugAuthenticationRequest,
        request: DebugRunTimelineRequestV2,
    ) -> DebugRunTimelineResponseV2 {
        let issuer = DebugViewerIssuer { _private: () };
        let viewer = match self.identity.authenticate(authentication, &issuer) {
            Ok(viewer) => viewer,
            Err(_) => {
                return DebugRunTimelineResponseV2 {
                    status: DebugTimelineStatus::BadRequest,
                    page: None,
                };
            }
        };
        match self.reader.read_page(&viewer.tenant, &request.0) {
            Ok(page) => DebugRunTimelineResponseV2 {
                status: DebugTimelineStatus::Ok,
                page: Some(page),
            },
            Err(RunTimelineReadError::Storage(_) | RunTimelineReadError::InvalidStoredSequence) => {
                DebugRunTimelineResponseV2 {
                    status: DebugTimelineStatus::Unavailable,
                    page: None,
                }
            }
            Err(RunTimelineReadError::RunNotFound) => DebugRunTimelineResponseV2 {
                status: DebugTimelineStatus::NotFound,
                page: None,
            },
        }
    }
}

/// Opaque browser/session material handed to a trusted identity adapter. It is
/// not a tenant selector and may not be used as an authorization result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DebugAuthenticationRequest {
    session_ref: String,
}

impl DebugAuthenticationRequest {
    pub fn new(session_ref: impl Into<String>) -> Result<Self, DebugAuthenticationError> {
        let session_ref = session_ref.into();
        if !is_identifier(&session_ref) {
            return Err(DebugAuthenticationError::InvalidRequest);
        }
        Ok(Self { session_ref })
    }

    #[must_use]
    pub fn session_ref(&self) -> &str {
        &self.session_ref
    }
}

/// Error returned by the trusted identity composition. The API maps it to a
/// safe transport status and never serializes the raw reason to the console.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DebugAuthenticationError {
    InvalidRequest,
    Denied,
    Unavailable,
}

/// Authenticated principal usable only after a trusted identity adapter has
/// received the non-constructible issuer capability below.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DebugViewer {
    tenant: AuthenticatedTenant,
}

/// Capability created inside the API for one authentication call. External
/// callers cannot construct one and therefore cannot mint a `DebugViewer`.
///
/// ```compile_fail
/// use improvement_engine_core::debug_console::DebugViewerIssuer;
///
/// let _forged = DebugViewerIssuer { _private: () };
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::debug_console::DebugConsoleApi;
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::debug_console::DebugIdentityPort;
/// struct Forged;
/// impl DebugIdentityPort for Forged {}
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::debug_console::DebugViewer;
/// let _ = DebugViewer { tenant: todo!() };
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::debug_console::DebugAuthenticationRequest;
/// let _ = DebugAuthenticationRequest::new("session_a");
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::debug_console::DebugTimelineRequest;
/// let _ = DebugTimelineRequest::new("run_a", None, 10);
/// ```
#[derive(Debug)]
pub(crate) struct DebugViewerIssuer {
    _private: (),
}

impl DebugViewerIssuer {
    /// Trusted identity adapters call this only after verifying their browser
    /// session/role. Possessing the issuer, not a tenant string, is authority.
    #[must_use]
    pub fn issue(&self, tenant: AuthenticatedTenant) -> DebugViewer {
        DebugViewer { tenant }
    }
}

/// Trusted authentication boundary supplied by control-api composition.
///
/// The adapter receives a fresh issuer only while authenticating one request.
/// It can map verified SSO/OIDC identity to a tenant, but an untrusted caller
/// cannot provide a `DebugViewer` or an issuer to the read endpoint.
pub(crate) trait DebugIdentityPort {
    fn authenticate(
        &mut self,
        request: DebugAuthenticationRequest,
        issuer: &DebugViewerIssuer,
    ) -> Result<DebugViewer, DebugAuthenticationError>;
}

/// Untrusted request values for the debug timeline. Tenant identity is
/// deliberately absent and is derived from `DebugViewer` inside the API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DebugTimelineRequest {
    run_id: String,
    cursor: Option<String>,
    page_size: usize,
}

impl DebugTimelineRequest {
    pub fn new(
        run_id: impl Into<String>,
        cursor: Option<String>,
        page_size: usize,
    ) -> Result<Self, DebugAuthenticationError> {
        let run_id = run_id.into();
        if run_id.is_empty() || page_size == 0 || page_size > 100 {
            return Err(DebugAuthenticationError::InvalidRequest);
        }
        Ok(Self {
            run_id,
            cursor,
            page_size,
        })
    }

    fn into_activity_request(
        self,
        viewer: DebugViewer,
    ) -> Result<ListRunActivityRequest, RunActivityError> {
        ListRunActivityRequest::new(viewer.tenant, self.run_id, self.cursor, self.page_size)
    }
}

/// The read-only `/internal/v1/debug` application adapter.
///
/// Authentication happens through an explicit trusted port. This adapter does
/// not accept a caller-provided U07 tenant DTO, maps transport-safe statuses,
/// and offers no mutation operation.
pub(crate) struct DebugConsoleApi<'a, ReadModel, Identity> {
    console: DebugConsole<'a, ReadModel>,
    identity: Identity,
}

impl<'a, ReadModel: RunActivityReadModel, Identity: DebugIdentityPort>
    DebugConsoleApi<'a, ReadModel, Identity>
{
    #[must_use]
    pub(crate) fn new(console: DebugConsole<'a, ReadModel>, identity: Identity) -> Self {
        Self { console, identity }
    }

    #[must_use]
    pub(crate) fn read_timeline(
        &mut self,
        authentication: DebugAuthenticationRequest,
        request: DebugTimelineRequest,
    ) -> DebugTimelineResponse {
        let issuer = DebugViewerIssuer { _private: () };
        let viewer = match self.identity.authenticate(authentication, &issuer) {
            Ok(viewer) => viewer,
            Err(_) => return bad_request_response(),
        };
        let request = match request.into_activity_request(viewer) {
            Ok(request) => request,
            Err(_) => return bad_request_response(),
        };
        match self.console.timeline(request) {
            Ok(timeline) => DebugTimelineResponse {
                status: DebugTimelineStatus::Ok,
                timeline: Some(timeline),
            },
            Err(error) => DebugTimelineResponse {
                status: status_from_activity_error(&error),
                timeline: None,
            },
        }
    }
}

fn bad_request_response() -> DebugTimelineResponse {
    DebugTimelineResponse {
        status: DebugTimelineStatus::BadRequest,
        timeline: None,
    }
}

fn status_from_activity_error(error: &RunActivityError) -> DebugTimelineStatus {
    match error.status() {
        ActivityApiStatus::BadRequest => DebugTimelineStatus::BadRequest,
        ActivityApiStatus::NotFound => DebugTimelineStatus::NotFound,
        ActivityApiStatus::Conflict => DebugTimelineStatus::Conflict,
        ActivityApiStatus::Gone => DebugTimelineStatus::Gone,
    }
}

fn activity_label(kind: &ActivityKind) -> &'static str {
    match kind {
        ActivityKind::Admitted => "admitted",
        ActivityKind::LeaseAcquired => "lease acquired",
        ActivityKind::EffectDispatchStarted => "effect dispatch started",
        ActivityKind::EffectAcknowledged => "effect acknowledged",
        ActivityKind::ReconciliationRequired => "reconciliation required",
        ActivityKind::ReconciledNoEffect => "reconciled with no effect",
        ActivityKind::ReconciledApplied => "reconciled with effect applied",
    }
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_activity::{ActivityEvent, RunActivityProjection};
    use std::cell::RefCell;

    const TENANT: &str = "tenant_a";
    const JOB: &str = "job:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    struct SessionIdentity;

    impl DebugIdentityPort for SessionIdentity {
        fn authenticate(
            &mut self,
            request: DebugAuthenticationRequest,
            issuer: &DebugViewerIssuer,
        ) -> Result<DebugViewer, DebugAuthenticationError> {
            let tenant = match request.session_ref() {
                "session_a" => TENANT,
                "session_b" => "tenant_b",
                _ => return Err(DebugAuthenticationError::Denied),
            };
            Ok(issuer.issue(AuthenticatedTenant::new(tenant).expect("fixed tenant")))
        }
    }

    fn auth(session: &str) -> DebugAuthenticationRequest {
        DebugAuthenticationRequest::new(session).expect("fixed auth request")
    }

    fn request(cursor: Option<String>, page_size: usize) -> DebugTimelineRequest {
        DebugTimelineRequest::new(JOB, cursor, page_size).expect("fixed read request")
    }

    fn event(event_id: &str, at: u64) -> ActivityEvent {
        ActivityEvent::new(TENANT, JOB, event_id, at, ActivityKind::Admitted, DIGEST)
            .expect("fixed event")
    }

    #[test]
    fn authenticated_debug_read_projects_only_safe_timeline_fields_without_mutation() {
        let mut projection = RunActivityProjection::new();
        projection.project(event("event_admitted", 10)).unwrap();
        let revision = projection.revision(TENANT, JOB).unwrap();
        let console = DebugConsole::new(
            &projection,
            ActivityCursorSigner::new("test-secret").unwrap(),
        );
        let mut api = DebugConsoleApi::new(console, SessionIdentity);

        let response = api.read_timeline(auth("session_a"), request(None, 100));
        let timeline = response.timeline().expect("timeline");

        assert_eq!(response.status(), DebugTimelineStatus::Ok);
        assert_eq!(timeline.events().len(), 1);
        assert_eq!(timeline.events()[0].event_id(), "event_admitted");
        assert_eq!(timeline.events()[0].kind(), &ActivityKind::Admitted);
        assert_eq!(timeline.events()[0].evidence_digest(), DIGEST);
        assert_eq!(
            response.accessible_status_summary(),
            "Timeline loaded: 1 event; no continuation."
        );
        assert_eq!(projection.revision(TENANT, JOB), Some(revision));
    }

    #[test]
    fn request_cannot_select_a_tenant_and_cross_tenant_cursor_is_rejected() {
        let mut projection = RunActivityProjection::new();
        projection.project(event("event_first", 10)).unwrap();
        projection.project(event("event_second", 20)).unwrap();
        let console = DebugConsole::new(
            &projection,
            ActivityCursorSigner::new("test-secret").unwrap(),
        );
        let mut api = DebugConsoleApi::new(console, SessionIdentity);

        let first = api.read_timeline(auth("session_a"), request(None, 1));
        let cursor = first.next_cursor().expect("continuation").to_owned();
        let foreign = api.read_timeline(auth("session_b"), request(Some(cursor), 1));

        assert_eq!(foreign.status(), DebugTimelineStatus::BadRequest);
        assert!(foreign.timeline().is_none());
        assert_eq!(
            foreign.accessible_status_summary(),
            "Timeline request cannot be processed."
        );
    }

    #[test]
    fn gone_and_not_found_responses_are_safe_and_never_invent_a_timeline() {
        let projection = RefCell::new(RunActivityProjection::new());
        projection
            .borrow_mut()
            .project(event("event_first", 10))
            .unwrap();
        projection
            .borrow_mut()
            .project(event("event_second", 20))
            .unwrap();
        let console = DebugConsole::new(
            &projection,
            ActivityCursorSigner::new("test-secret").unwrap(),
        );
        let mut api = DebugConsoleApi::new(console, SessionIdentity);
        let first = api.read_timeline(auth("session_a"), request(None, 1));
        let cursor = first.next_cursor().expect("continuation").to_owned();
        assert_eq!(
            first.accessible_status_summary(),
            "Timeline loaded: 1 event; continuation available."
        );
        projection
            .borrow_mut()
            .project(event("event_third", 30))
            .unwrap();

        let gone = api.read_timeline(auth("session_a"), request(Some(cursor), 1));
        let missing = api.read_timeline(
            auth("session_a"),
            DebugTimelineRequest::new(
                "job:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                None,
                1,
            )
            .unwrap(),
        );

        assert_eq!(gone.status(), DebugTimelineStatus::Gone);
        assert!(gone.timeline().is_none());
        assert_eq!(
            gone.accessible_status_summary(),
            "Timeline changed; reload the current snapshot to continue."
        );
        assert_eq!(missing.status(), DebugTimelineStatus::NotFound);
        assert!(missing.timeline().is_none());
        assert_eq!(missing.accessible_status_summary(), "Run not available.");
    }

    #[test]
    fn denied_identity_has_a_stable_safe_summary() {
        struct Denied;
        impl DebugIdentityPort for Denied {
            fn authenticate(
                &mut self,
                _: DebugAuthenticationRequest,
                _: &DebugViewerIssuer,
            ) -> Result<DebugViewer, DebugAuthenticationError> {
                Err(DebugAuthenticationError::Denied)
            }
        }

        let projection = RunActivityProjection::new();
        let console = DebugConsole::new(
            &projection,
            ActivityCursorSigner::new("test-secret").unwrap(),
        );
        let mut api = DebugConsoleApi::new(console, Denied);

        let response = api.read_timeline(auth("session_a"), request(None, 1));

        assert_eq!(response.status(), DebugTimelineStatus::BadRequest);
        assert!(response.timeline().is_none());
        assert_eq!(
            response.accessible_status_summary(),
            "Timeline request cannot be processed."
        );
    }

    #[test]
    fn v2_composer_derives_tenant_from_authenticated_viewer_and_maps_storage_errors_safely() {
        use crate::run_timeline_v2::{
            RunTimelinePage, RunTimelineReadError, RunTimelineReadPort, RunTimelineRequest,
        };

        struct RecordingReader(Option<String>);
        impl RunTimelineReadPort for RecordingReader {
            fn read_page(
                &mut self,
                tenant: &AuthenticatedTenant,
                _: &RunTimelineRequest,
            ) -> Result<RunTimelinePage, RunTimelineReadError> {
                self.0 = Some(tenant.tenant_id().to_owned());
                Err(RunTimelineReadError::InvalidStoredSequence)
            }
        }

        let mut api = DebugConsoleV2Api::new(RecordingReader(None), SessionIdentity);
        let request =
            DebugRunTimelineRequestV2::new("00000000-0000-7000-8000-000000000002", 17, 20).unwrap();
        let response = api.read_events(auth("session_b"), request);

        assert_eq!(response.status(), DebugTimelineStatus::Unavailable);
        assert!(response.page().is_none());
        assert_eq!(api.reader.0.as_deref(), Some("tenant_b"));
    }

    #[test]
    fn v2_composer_denial_does_not_touch_the_read_port() {
        use crate::run_timeline_v2::{
            RunTimelinePage, RunTimelineReadError, RunTimelineReadPort, RunTimelineRequest,
        };

        struct RecordingReader(bool);
        impl RunTimelineReadPort for RecordingReader {
            fn read_page(
                &mut self,
                _: &AuthenticatedTenant,
                _: &RunTimelineRequest,
            ) -> Result<RunTimelinePage, RunTimelineReadError> {
                self.0 = true;
                Err(RunTimelineReadError::InvalidStoredSequence)
            }
        }
        struct Denied;
        impl DebugIdentityPort for Denied {
            fn authenticate(
                &mut self,
                _: DebugAuthenticationRequest,
                _: &DebugViewerIssuer,
            ) -> Result<DebugViewer, DebugAuthenticationError> {
                Err(DebugAuthenticationError::Denied)
            }
        }

        let mut api = DebugConsoleV2Api::new(RecordingReader(false), Denied);
        let response = api.read_events(
            auth("session_a"),
            DebugRunTimelineRequestV2::new("00000000-0000-7000-8000-000000000002", 0, 10).unwrap(),
        );
        assert_eq!(response.status(), DebugTimelineStatus::BadRequest);
        assert!(!api.reader.0);
    }
}
