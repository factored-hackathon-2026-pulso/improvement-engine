//! Read-only console projection for engineering debugging.
//!
//! This boundary consumes the tenant-bound U07 activity read model.  It does
//! not read artifacts, source data, SQL, prompts, or platform state directly;
//! transports may render its safe projection without gaining a mutation path.

use crate::run_activity::{
    ActivityApiStatus, ActivityCursorSigner, ActivityEntry, ActivityKind, InMemoryCursorRegistry,
    ListRunActivityRequest, RunActivityError, RunActivityHandler, RunActivityReadModel,
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

    pub fn timeline(
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
}

/// The read-only `/internal/v1/debug` application adapter.
///
/// Authentication and HTTP serialization are intentionally composition-root
/// concerns. This adapter accepts only the already tenant-bound U07 request,
/// maps transport-safe statuses, and offers no mutation operation.
pub struct DebugConsoleApi<'a, ReadModel> {
    console: DebugConsole<'a, ReadModel>,
}

impl<'a, ReadModel: RunActivityReadModel> DebugConsoleApi<'a, ReadModel> {
    #[must_use]
    pub fn new(console: DebugConsole<'a, ReadModel>) -> Self {
        Self { console }
    }

    #[must_use]
    pub fn read_timeline(&mut self, request: ListRunActivityRequest) -> DebugTimelineResponse {
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
