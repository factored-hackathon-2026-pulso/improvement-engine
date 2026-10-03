//! Read-only, tenant-bound projection of the durable V2 run-event sequence.
//!
//! This is deliberately separate from U07's legacy job/timestamp cursor. The
//! wire continuation is the last observed run-local sequence and is not an
//! authorization token; every read still binds tenant and run in SQL.
//!
//! The composer is crate-private until a real authenticated control-api root
//! consumes it; public-looking accessors here define that internal read model.
#![allow(dead_code)]

use crate::run_activity::AuthenticatedTenant;
use postgres::{Client, Error as PostgresError};
use std::fmt;

const MAX_PAGE_SIZE: usize = 100;
const DEBUG_EVENT_VOCABULARY_VERSION: u32 = 1;

/// Untrusted continuation request. Tenant identity is never carried here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunTimelineRequest {
    run_ref: String,
    after_sequence: i64,
    page_size: usize,
}

impl RunTimelineRequest {
    pub(crate) fn new(
        run_ref: impl Into<String>,
        after_sequence: i64,
        page_size: usize,
    ) -> Result<Self, RunTimelineRequestError> {
        let run_ref = run_ref.into();
        if !is_uuid_v7(&run_ref) || after_sequence < 0 || !(1..=MAX_PAGE_SIZE).contains(&page_size)
        {
            return Err(RunTimelineRequestError::InvalidRequest);
        }
        Ok(Self {
            run_ref,
            after_sequence,
            page_size,
        })
    }

    #[must_use]
    pub(crate) fn run_ref(&self) -> &str {
        &self.run_ref
    }

    #[must_use]
    pub(crate) fn after_sequence(&self) -> i64 {
        self.after_sequence
    }

    #[must_use]
    pub(crate) fn page_size(&self) -> usize {
        self.page_size
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunTimelineRequestError {
    InvalidRequest,
}

impl fmt::Display for RunTimelineRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid run timeline request")
    }
}

impl std::error::Error for RunTimelineRequestError {}

/// Closed, versioned vocabulary used by the engineering timeline. Values in
/// the database that have not been approved here collapse to `other`; database
/// text is never reflected into a debugging response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DebugEventStage {
    Trigger,
    Job,
    Extract,
    Query,
    Model,
    Signal,
    Proposal,
    Evaluation,
    Release,
    Observation,
    Memory,
    Dependency,
    Run,
    Other,
}

impl DebugEventStage {
    fn from_db(value: &str) -> Self {
        match value {
            "trigger" => Self::Trigger,
            "job" => Self::Job,
            "extract" => Self::Extract,
            "query" => Self::Query,
            "model" => Self::Model,
            "signal" => Self::Signal,
            "proposal" => Self::Proposal,
            "evaluation" => Self::Evaluation,
            "release" => Self::Release,
            "observation" => Self::Observation,
            "memory" => Self::Memory,
            "dependency" => Self::Dependency,
            "run" => Self::Run,
            _ => Self::Other,
        }
    }

    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Trigger => "trigger",
            Self::Job => "job",
            Self::Extract => "extract",
            Self::Query => "query",
            Self::Model => "model",
            Self::Signal => "signal",
            Self::Proposal => "proposal",
            Self::Evaluation => "evaluation",
            Self::Release => "release",
            Self::Observation => "observation",
            Self::Memory => "memory",
            Self::Dependency => "dependency",
            Self::Run => "run",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DebugEventCode {
    TriggerAccepted,
    JobQueued,
    JobClaimed,
    HeartbeatLost,
    JobCompleted,
    JobDeferred,
    JobDead,
    ExtractStarted,
    ExtractCompleted,
    ExtractRejected,
    QuerySubmitted,
    QueryCompleted,
    QueryDenied,
    ModelInvoked,
    ModelCompleted,
    ModelInvalid,
    ModelUnknown,
    SignalSelected,
    SignalRefuted,
    SignalCorroborated,
    ProposalRevised,
    EvaluationStarted,
    EvaluationCompleted,
    EvaluationGateBlocked,
    ReleaseSent,
    ReleaseAckUnknown,
    ReleaseConfirmed,
    ReleaseRejected,
    ObservationIngested,
    ObservationGap,
    MemoryMounted,
    MemoryPublishRejected,
    MemoryPublished,
    DependencyWaiting,
    DependencyResumed,
    RunCompleted,
    RunCancelled,
    Other,
}

impl DebugEventCode {
    fn from_db(value: &str) -> Self {
        match value {
            "trigger_accepted" => Self::TriggerAccepted,
            "job_queued" => Self::JobQueued,
            "job_claimed" => Self::JobClaimed,
            "heartbeat_lost" => Self::HeartbeatLost,
            "job_completed" => Self::JobCompleted,
            "job_deferred" => Self::JobDeferred,
            "job_dead" => Self::JobDead,
            "extract_started" => Self::ExtractStarted,
            "extract_completed" => Self::ExtractCompleted,
            "extract_rejected" => Self::ExtractRejected,
            "query_submitted" => Self::QuerySubmitted,
            "query_completed" => Self::QueryCompleted,
            "query_denied" => Self::QueryDenied,
            "model_invoked" => Self::ModelInvoked,
            "model_completed" => Self::ModelCompleted,
            "model_invalid" => Self::ModelInvalid,
            "model_unknown" => Self::ModelUnknown,
            "signal_selected" => Self::SignalSelected,
            "signal_refuted" => Self::SignalRefuted,
            "signal_corroborated" => Self::SignalCorroborated,
            "proposal_revised" => Self::ProposalRevised,
            "evaluation_started" => Self::EvaluationStarted,
            "evaluation_completed" => Self::EvaluationCompleted,
            "evaluation_gate_blocked" => Self::EvaluationGateBlocked,
            "release_sent" => Self::ReleaseSent,
            "release_ack_unknown" => Self::ReleaseAckUnknown,
            "release_confirmed" => Self::ReleaseConfirmed,
            "release_rejected" => Self::ReleaseRejected,
            "observation_ingested" => Self::ObservationIngested,
            "observation_gap" => Self::ObservationGap,
            "memory_mounted" => Self::MemoryMounted,
            "memory_publish_rejected" => Self::MemoryPublishRejected,
            "memory_published" => Self::MemoryPublished,
            "dependency_waiting" => Self::DependencyWaiting,
            "dependency_resumed" => Self::DependencyResumed,
            "run_completed" => Self::RunCompleted,
            "run_cancelled" => Self::RunCancelled,
            _ => Self::Other,
        }
    }

    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TriggerAccepted => "trigger_accepted",
            Self::JobQueued => "job_queued",
            Self::JobClaimed => "job_claimed",
            Self::HeartbeatLost => "heartbeat_lost",
            Self::JobCompleted => "job_completed",
            Self::JobDeferred => "job_deferred",
            Self::JobDead => "job_dead",
            Self::ExtractStarted => "extract_started",
            Self::ExtractCompleted => "extract_completed",
            Self::ExtractRejected => "extract_rejected",
            Self::QuerySubmitted => "query_submitted",
            Self::QueryCompleted => "query_completed",
            Self::QueryDenied => "query_denied",
            Self::ModelInvoked => "model_invoked",
            Self::ModelCompleted => "model_completed",
            Self::ModelInvalid => "model_invalid",
            Self::ModelUnknown => "model_unknown",
            Self::SignalSelected => "signal_selected",
            Self::SignalRefuted => "signal_refuted",
            Self::SignalCorroborated => "signal_corroborated",
            Self::ProposalRevised => "proposal_revised",
            Self::EvaluationStarted => "evaluation_started",
            Self::EvaluationCompleted => "evaluation_completed",
            Self::EvaluationGateBlocked => "evaluation_gate_blocked",
            Self::ReleaseSent => "release_sent",
            Self::ReleaseAckUnknown => "release_ack_unknown",
            Self::ReleaseConfirmed => "release_confirmed",
            Self::ReleaseRejected => "release_rejected",
            Self::ObservationIngested => "observation_ingested",
            Self::ObservationGap => "observation_gap",
            Self::MemoryMounted => "memory_mounted",
            Self::MemoryPublishRejected => "memory_publish_rejected",
            Self::MemoryPublished => "memory_published",
            Self::DependencyWaiting => "dependency_waiting",
            Self::DependencyResumed => "dependency_resumed",
            Self::RunCompleted => "run_completed",
            Self::RunCancelled => "run_cancelled",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DebugEventStatus {
    Queued,
    Claimed,
    Running,
    Completed,
    Deferred,
    Dead,
    RetryWait,
    WaitingDependency,
    Cancelled,
    Started,
    Rejected,
    Submitted,
    Denied,
    Invoked,
    Invalid,
    Unknown,
    Selected,
    Refuted,
    Corroborated,
    Revised,
    GateBlocked,
    Sent,
    AckUnknown,
    Confirmed,
    Gap,
    Mounted,
    PublishRejected,
    Published,
    Resumed,
    Other,
}

impl DebugEventStatus {
    fn from_db(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "claimed" => Self::Claimed,
            "running" => Self::Running,
            "completed" | "complete" => Self::Completed,
            "deferred" => Self::Deferred,
            "dead" => Self::Dead,
            "retry_wait" => Self::RetryWait,
            "waiting_dependency" => Self::WaitingDependency,
            "cancelled" => Self::Cancelled,
            "started" => Self::Started,
            "rejected" => Self::Rejected,
            "submitted" => Self::Submitted,
            "denied" => Self::Denied,
            "invoked" => Self::Invoked,
            "invalid" => Self::Invalid,
            "unknown" => Self::Unknown,
            "selected" => Self::Selected,
            "refuted" => Self::Refuted,
            "corroborated" => Self::Corroborated,
            "revised" => Self::Revised,
            "gate_blocked" => Self::GateBlocked,
            "sent" => Self::Sent,
            "ack_unknown" => Self::AckUnknown,
            "confirmed" => Self::Confirmed,
            "gap" => Self::Gap,
            "mounted" => Self::Mounted,
            "publish_rejected" => Self::PublishRejected,
            "published" => Self::Published,
            "resumed" => Self::Resumed,
            _ => Self::Other,
        }
    }

    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Deferred => "deferred",
            Self::Dead => "dead",
            Self::RetryWait => "retry_wait",
            Self::WaitingDependency => "waiting_dependency",
            Self::Cancelled => "cancelled",
            Self::Started => "started",
            Self::Rejected => "rejected",
            Self::Submitted => "submitted",
            Self::Denied => "denied",
            Self::Invoked => "invoked",
            Self::Invalid => "invalid",
            Self::Unknown => "unknown",
            Self::Selected => "selected",
            Self::Refuted => "refuted",
            Self::Corroborated => "corroborated",
            Self::Revised => "revised",
            Self::GateBlocked => "gate_blocked",
            Self::Sent => "sent",
            Self::AckUnknown => "ack_unknown",
            Self::Confirmed => "confirmed",
            Self::Gap => "gap",
            Self::Mounted => "mounted",
            Self::PublishRejected => "publish_rejected",
            Self::Published => "published",
            Self::Resumed => "resumed",
            Self::Other => "other",
        }
    }
}

/// The allowlisted projection of a durable V2 event; payload/detail/artifact
/// refs, trace IDs and reason codes never cross this boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DebugRunEvent {
    event_ref: String,
    run_ref: String,
    job_ref: Option<String>,
    sequence: i64,
    event_at_epoch_micros: i64,
    stage: DebugEventStage,
    event_code: DebugEventCode,
    status: DebugEventStatus,
}

impl DebugRunEvent {
    #[must_use]
    pub(crate) fn event_ref(&self) -> &str {
        &self.event_ref
    }
    #[must_use]
    pub(crate) fn run_ref(&self) -> &str {
        &self.run_ref
    }
    #[must_use]
    pub(crate) fn job_ref(&self) -> Option<&str> {
        self.job_ref.as_deref()
    }
    #[must_use]
    pub(crate) fn sequence(&self) -> i64 {
        self.sequence
    }
    #[must_use]
    pub(crate) fn event_at_epoch_micros(&self) -> i64 {
        self.event_at_epoch_micros
    }
    #[must_use]
    pub(crate) fn stage(&self) -> DebugEventStage {
        self.stage
    }
    #[must_use]
    pub(crate) fn event_code(&self) -> DebugEventCode {
        self.event_code
    }
    #[must_use]
    pub(crate) fn status(&self) -> DebugEventStatus {
        self.status
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunTimelinePage {
    events: Vec<DebugRunEvent>,
    next_after_sequence: Option<i64>,
    vocabulary_version: u32,
}

impl RunTimelinePage {
    #[must_use]
    pub(crate) fn events(&self) -> &[DebugRunEvent] {
        &self.events
    }
    #[must_use]
    pub(crate) fn has_more(&self) -> bool {
        self.next_after_sequence.is_some()
    }
    #[must_use]
    pub(crate) fn next_after_sequence(&self) -> Option<i64> {
        self.next_after_sequence
    }
    #[must_use]
    pub(crate) fn vocabulary_version(&self) -> u32 {
        self.vocabulary_version
    }
}

#[derive(Debug)]
pub(crate) enum RunTimelineReadError {
    RunNotFound,
    Storage(PostgresError),
    InvalidStoredSequence,
}

impl fmt::Display for RunTimelineReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("run timeline unavailable")
    }
}

impl std::error::Error for RunTimelineReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RunNotFound | Self::InvalidStoredSequence => None,
            Self::Storage(error) => Some(error),
        }
    }
}

pub(crate) trait RunTimelineReadPort {
    fn read_page(
        &mut self,
        tenant: &AuthenticatedTenant,
        request: &RunTimelineRequest,
    ) -> Result<RunTimelinePage, RunTimelineReadError>;
}

/// PostgreSQL read adapter over the existing append-only U07 V2 event table.
pub(crate) struct PostgresRunTimelineReader {
    client: Client,
}

impl PostgresRunTimelineReader {
    #[must_use]
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }
}

impl RunTimelineReadPort for PostgresRunTimelineReader {
    fn read_page(
        &mut self,
        tenant: &AuthenticatedTenant,
        request: &RunTimelineRequest,
    ) -> Result<RunTimelinePage, RunTimelineReadError> {
        let limit = i64::try_from(request.page_size + 1)
            .map_err(|_| RunTimelineReadError::InvalidStoredSequence)?;
        let visible_run = self
            .client
            .query_opt(
                "SELECT 1 FROM public.pulso_jobs \
                 WHERE tenant_id=$1 AND id=$2::text::uuid AND run_ref=id",
                &[&tenant.tenant_id(), &request.run_ref],
            )
            .map_err(RunTimelineReadError::Storage)?;
        if visible_run.is_none() {
            return Err(RunTimelineReadError::RunNotFound);
        }
        let rows = self
            .client
            .query(
                "SELECT id::text, run_ref::text, job_ref::text, sequence, \
                    (EXTRACT(EPOCH FROM event_at) * 1000000)::bigint, stage, event_code, status \
             FROM public.pulso_run_events \
             WHERE tenant_id=$1 AND run_ref=$2::text::uuid AND sequence>$3 \
             ORDER BY sequence ASC LIMIT $4",
                &[
                    &tenant.tenant_id(),
                    &request.run_ref,
                    &request.after_sequence,
                    &limit,
                ],
            )
            .map_err(RunTimelineReadError::Storage)?;
        let has_more = rows.len() > request.page_size;
        let mut events = Vec::with_capacity(rows.len().min(request.page_size));
        for row in rows.into_iter().take(request.page_size) {
            let sequence: i64 = row.get(3);
            if sequence <= request.after_sequence {
                return Err(RunTimelineReadError::InvalidStoredSequence);
            }
            events.push(DebugRunEvent {
                event_ref: row.get(0),
                run_ref: row.get(1),
                job_ref: row.get(2),
                sequence,
                event_at_epoch_micros: row.get(4),
                stage: DebugEventStage::from_db(row.get::<_, String>(5).as_str()),
                event_code: DebugEventCode::from_db(row.get::<_, String>(6).as_str()),
                status: DebugEventStatus::from_db(row.get::<_, String>(7).as_str()),
            });
        }
        let next_after_sequence = has_more
            .then(|| events.last().map(|event| event.sequence))
            .flatten();
        Ok(RunTimelinePage {
            events,
            next_after_sequence,
            vocabulary_version: DEBUG_EVENT_VOCABULARY_VERSION,
        })
    }
}

fn is_uuid_v7(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes[14] == b'7'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index) || matches!(*byte, b'0'..=b'9' | b'a'..=b'f')
        })
}

#[cfg(test)]
mod tests {
    use super::{
        DebugEventCode, DebugEventStage, DebugEventStatus, PostgresRunTimelineReader,
        RunTimelineReadError, RunTimelineReadPort, RunTimelineRequest,
    };

    const TENANT_A: &str = "tenant_a";
    const TENANT_B: &str = "tenant_b";
    const RUN_A: &str = "00000000-0000-7000-8000-000000000002";
    const RUN_B: &str = "00000000-0000-7000-8000-000000000008";
    const CHILD_A: &str = "00000000-0000-7000-8000-000000000003";
    const EVENT_1: &str = "00000000-0000-7000-8000-000000000004";
    const EVENT_2: &str = "00000000-0000-7000-8000-000000000005";
    const EVENT_3: &str = "00000000-0000-7000-8000-000000000009";
    const EVENT_4: &str = "00000000-0000-7000-8000-000000000010";

    fn connect() -> postgres::Client {
        assert_eq!(
            std::env::var("PULSO_ALLOW_DESTRUCTIVE_TEST_DB").as_deref(),
            Ok("1")
        );
        let url = std::env::var("PULSO_TEST_POSTGRES_URL").unwrap();
        postgres::Client::connect(&url, postgres::NoTls).unwrap()
    }

    fn prepare() -> postgres::Client {
        let mut client = connect();
        let database_name: String = client
            .query_one("SELECT current_database()", &[])
            .unwrap()
            .get(0);
        validate_test_database_name(&database_name)
            .expect("destructive PostgreSQL fixture must target exactly pulso_test");
        client
            .batch_execute(include_str!(
                "../../../migrations/0003_pulso_run_events.sql"
            ))
            .unwrap();
        client
            .batch_execute("TRUNCATE pulso_run_events, pulso_jobs CASCADE;")
            .unwrap();
        client
            .execute(
                "INSERT INTO pulso_jobs (id, tenant_id, run_ref, kind, logical_key, generation, \
                 parent_job_id, status, lane, due_at, input_ref, config_ref) VALUES \
                 ($1::uuid, $2, $1::uuid, 'detect', 'run-a', 0, $1::uuid, 'queued', 'default', now(), 'input:a', 'config:a'), \
                 ($3::uuid, $4, $3::uuid, 'detect', 'run-b', 0, $3::uuid, 'queued', 'default', now(), 'input:b', 'config:b'), \
                 ($5::uuid, $2, $1::uuid, 'verify', 'child-a', 0, $1::uuid, 'queued', 'default', now(), 'input:a', 'config:a')",
                &[&RUN_A, &TENANT_A, &RUN_B, &TENANT_B, &CHILD_A],
            )
            .unwrap();
        client
    }

    fn validate_test_database_name(name: &str) -> Result<(), &'static str> {
        if name == "pulso_test" {
            Ok(())
        } else {
            Err("refusing destructive fixture outside pulso_test")
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_event(
        client: &mut postgres::Client,
        id: &str,
        tenant: &str,
        run_ref: &str,
        job_ref: Option<&str>,
        sequence: i64,
        stage: &str,
        event_code: &str,
        status: &str,
        reason_code: Option<&str>,
    ) {
        client
            .execute(
                "INSERT INTO pulso_run_events (id, tenant_id, run_ref, job_ref, sequence, event_at, stage, event_code, status, reason_code, artifact_ref, trace_id, details_ref) \
                 VALUES ($1::uuid, $2, $3::uuid, $4::uuid, $5, now(), $6, $7, $8, $9, 'secret/artifact/ref', 'secret-trace-id', 'secret/details/ref')",
                &[&id, &tenant, &run_ref, &job_ref, &sequence, &stage, &event_code, &status, &reason_code],
            )
            .unwrap();
    }

    #[test]
    fn unknown_database_vocabulary_never_round_trips_as_debug_text() {
        assert_eq!(DebugEventStage::from_db("job").as_str(), "job");
        assert_eq!(
            DebugEventCode::from_db("job_claimed").as_str(),
            "job_claimed"
        );
        assert_eq!(DebugEventStatus::from_db("running").as_str(), "running");
        assert_eq!(
            DebugEventStage::from_db("customer_411_case").as_str(),
            "other"
        );
        assert_eq!(
            DebugEventCode::from_db("customer_411_case").as_str(),
            "other"
        );
        assert_eq!(
            DebugEventStatus::from_db("customer_411_status").as_str(),
            "other"
        );
    }

    #[test]
    fn request_rejects_invalid_run_sequence_and_page_bounds() {
        assert!(RunTimelineRequest::new("not-a-uuid", 0, 1).is_err());
        assert!(RunTimelineRequest::new(RUN_A, -1, 1).is_err());
        assert!(RunTimelineRequest::new(RUN_A, 0, 0).is_err());
        assert!(RunTimelineRequest::new(RUN_A, 0, 101).is_err());
        assert!(RunTimelineRequest::new(RUN_A, 0, 100).is_ok());
    }

    #[test]
    fn destructive_fixture_refuses_any_database_other_than_pulso_test() {
        assert!(validate_test_database_name("pulso_test").is_ok());
        assert!(validate_test_database_name("production").is_err());
        assert!(validate_test_database_name("pulso_test_copy").is_err());
    }

    #[test]
    #[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
    fn authenticated_scope_sequence_pagination_and_sanitization_use_real_postgres() {
        let mut client = prepare();
        insert_event(
            &mut client,
            EVENT_1,
            TENANT_A,
            RUN_A,
            Some(CHILD_A),
            1,
            "job",
            "job_claimed",
            "running",
            Some("sensitive_reason"),
        );
        insert_event(
            &mut client,
            EVENT_2,
            TENANT_A,
            RUN_A,
            None,
            2,
            "run",
            "run_completed",
            "completed",
            None,
        );
        insert_event(
            &mut client,
            EVENT_3,
            TENANT_A,
            RUN_A,
            None,
            3,
            "customer_411_stage",
            "customer_411_case",
            "customer_411_status",
            Some("sensitive_reason"),
        );
        insert_event(
            &mut client,
            EVENT_4,
            TENANT_B,
            RUN_B,
            None,
            1,
            "job",
            "job_queued",
            "queued",
            None,
        );

        let tenant = crate::run_activity::AuthenticatedTenant::new(TENANT_A).unwrap();
        let mut reader = PostgresRunTimelineReader::new(client);
        let first = reader
            .read_page(&tenant, &RunTimelineRequest::new(RUN_A, 0, 1).unwrap())
            .unwrap();
        assert_eq!(first.events().len(), 1);
        assert_eq!(first.events()[0].sequence(), 1);
        assert!(first.has_more());
        assert_eq!(first.next_after_sequence(), Some(1));
        assert_eq!(first.events()[0].stage().as_str(), "job");
        assert_eq!(first.events()[0].event_code().as_str(), "job_claimed");
        assert_eq!(first.events()[0].status().as_str(), "running");
        assert_eq!(first.vocabulary_version(), 1);
        let first_wire = format!("{:?}", first.events()[0]);
        for forbidden in [
            "sensitive_reason",
            "secret/artifact/ref",
            "secret-trace-id",
            "secret/details/ref",
            "customer_411",
        ] {
            assert!(!first_wire.contains(forbidden));
        }

        let second = reader
            .read_page(&tenant, &RunTimelineRequest::new(RUN_A, 1, 1).unwrap())
            .unwrap();
        assert_eq!(
            second
                .events()
                .iter()
                .map(|event| event.sequence())
                .collect::<Vec<_>>(),
            [2]
        );
        assert!(second.has_more());
        assert_eq!(second.next_after_sequence(), Some(2));

        let third = reader
            .read_page(&tenant, &RunTimelineRequest::new(RUN_A, 2, 1).unwrap())
            .unwrap();
        assert_eq!(third.events().len(), 1);
        assert_eq!(third.events()[0].stage().as_str(), "other");
        assert_eq!(third.events()[0].event_code().as_str(), "other");
        assert_eq!(third.events()[0].status().as_str(), "other");
        assert!(!third.has_more());
        assert_eq!(third.next_after_sequence(), None);
        let third_wire = format!("{:?}", third.events()[0]);
        for forbidden in [
            "sensitive_reason",
            "secret/artifact/ref",
            "secret-trace-id",
            "secret/details/ref",
            "customer_411",
        ] {
            assert!(!third_wire.contains(forbidden));
        }

        let empty = reader
            .read_page(&tenant, &RunTimelineRequest::new(RUN_A, 3, 1).unwrap())
            .unwrap();
        assert!(empty.events().is_empty());
        assert!(!empty.has_more());
        assert_eq!(empty.next_after_sequence(), None);

        let foreign = reader
            .read_page(&tenant, &RunTimelineRequest::new(RUN_B, 0, 10).unwrap())
            .unwrap_err();
        assert!(matches!(foreign, RunTimelineReadError::RunNotFound));
    }
}
