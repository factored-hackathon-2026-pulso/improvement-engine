//! Durable V2 run-event ledger.
//!
//! This boundary uses V2 `(tenant_id, run_ref, sequence)` identity and must
//! not be adapted to U07's in-memory `(job_id, timestamp, event_id)` cursor.

use postgres::{Client, Error as PostgresError};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Queued,
    Running,
    RetryWait,
    WaitingDependency,
    Complete,
    Deferred,
    Dead,
    Superseded,
    Cancelled,
}

impl JobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::RetryWait => "retry_wait",
            Self::WaitingDependency => "waiting_dependency",
            Self::Complete => "complete",
            Self::Deferred => "deferred",
            Self::Dead => "dead",
            Self::Superseded => "superseded",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunEventStatus(String);

impl RunEventStatus {
    pub fn new(value: impl Into<String>) -> Result<Self, RunEventError> {
        let value = value.into();
        if !is_code(&value) {
            return Err(RunEventError::InvalidEvent);
        }
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobTransition {
    job_ref: String,
    expected_status: JobStatus,
    next_status: JobStatus,
}

impl JobTransition {
    /// Builds a V2 §15 status edge. Cancellation is deliberately excluded:
    /// it requires revocation and proof that no external effect is pending.
    pub fn new(
        job_ref: impl Into<String>,
        expected_status: JobStatus,
        next_status: JobStatus,
    ) -> Result<Self, RunEventError> {
        let job_ref = job_ref.into();
        if !is_uuid_v7(&job_ref) {
            return Err(RunEventError::InvalidEvent);
        }
        if !is_legal_transition(expected_status, next_status) {
            return Err(RunEventError::IllegalJobTransition);
        }
        Ok(Self {
            job_ref,
            expected_status,
            next_status,
        })
    }
}

/// One requested state transition and its durable timeline fact.
///
/// IDs are UUID strings as defined by V2. The database checks UUID syntax and
/// tenant/run membership again at write time; this constructor validates the
/// event vocabulary, not authorization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableRunEvent {
    id: String,
    tenant_id: String,
    run_ref: String,
    job_transition: Option<JobTransition>,
    stage: String,
    event_code: String,
    status: RunEventStatus,
    reason_code: Option<String>,
    artifact_ref: Option<String>,
    trace_id: Option<String>,
    details_ref: Option<String>,
}

impl DurableRunEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        tenant_id: impl Into<String>,
        run_ref: impl Into<String>,
        job_transition: Option<JobTransition>,
        stage: impl Into<String>,
        event_code: impl Into<String>,
        status: RunEventStatus,
        reason_code: Option<String>,
        artifact_ref: Option<String>,
        trace_id: Option<String>,
        details_ref: Option<String>,
    ) -> Result<Self, RunEventError> {
        let id = id.into();
        let tenant_id = tenant_id.into();
        let run_ref = run_ref.into();
        let stage = stage.into();
        let event_code = event_code.into();
        if !is_tenant_id(&tenant_id)
            || !is_uuid_v7(&id)
            || !is_uuid_v7(&run_ref)
            || !is_code(&stage)
            || !is_code(&event_code)
            || reason_code.as_deref().is_some_and(|value| !is_code(value))
            || artifact_ref
                .as_deref()
                .is_some_and(|value| !is_reference(value))
            || trace_id
                .as_deref()
                .is_some_and(|value| !is_reference(value))
            || details_ref
                .as_deref()
                .is_some_and(|value| !is_reference(value))
        {
            return Err(RunEventError::InvalidEvent);
        }
        Ok(Self {
            id,
            tenant_id,
            run_ref,
            job_transition,
            stage,
            event_code,
            status,
            reason_code,
            artifact_ref,
            trace_id,
            details_ref,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendedRunEvent {
    id: String,
    sequence: i64,
}

impl AppendedRunEvent {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn sequence(&self) -> i64 {
        self.sequence
    }
}

#[derive(Debug)]
pub enum RunEventError {
    InvalidEvent,
    RunNotFound,
    JobNotInRun,
    JobStateConflict,
    IllegalJobTransition,
    SequenceExhausted,
    Storage(PostgresError),
}

impl fmt::Display for RunEventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEvent => formatter.write_str("invalid durable run event"),
            Self::RunNotFound => formatter.write_str("run not found"),
            Self::JobNotInRun => formatter.write_str("job does not belong to run"),
            Self::JobStateConflict => formatter.write_str("job state changed concurrently"),
            Self::IllegalJobTransition => formatter.write_str("illegal job status transition"),
            Self::SequenceExhausted => formatter.write_str("run event sequence exhausted"),
            Self::Storage(_) => formatter.write_str("durable run event storage failed"),
        }
    }
}

impl std::error::Error for RunEventError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}

/// PostgreSQL writer for V2 job transitions and the durable run timeline.
pub struct PostgresRunEventLedger {
    client: Client,
}

impl PostgresRunEventLedger {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Changes job status and appends its run event in one PostgreSQL
    /// transaction. The root row lock serializes concurrent writers per run.
    pub fn append_transition(
        &mut self,
        event: DurableRunEvent,
    ) -> Result<AppendedRunEvent, RunEventError> {
        let mut transaction = self.client.transaction().map_err(RunEventError::Storage)?;
        let root = transaction
            .query_opt(
                "SELECT run_ref::text, last_event_sequence FROM pulso_jobs \
                 WHERE tenant_id=$1 AND id=$2::uuid FOR UPDATE",
                &[&event.tenant_id, &event.run_ref],
            )
            .map_err(RunEventError::Storage)?
            .ok_or(RunEventError::RunNotFound)?;
        let root_ref: String = root.get(0);
        if root_ref != event.run_ref {
            return Err(RunEventError::RunNotFound);
        }
        let sequence: i64 = root.get(1);
        let next_sequence = sequence
            .checked_add(1)
            .filter(|value| *value > 0)
            .ok_or(RunEventError::SequenceExhausted)?;

        if let Some(transition) = event.job_transition.as_ref() {
            let job = transaction
                .query_opt(
                    "SELECT run_ref::text, status FROM pulso_jobs \
                     WHERE tenant_id=$1 AND id=$2::uuid FOR UPDATE",
                    &[&event.tenant_id, &transition.job_ref],
                )
                .map_err(RunEventError::Storage)?
                .ok_or(RunEventError::JobNotInRun)?;
            let job_run_ref: String = job.get(0);
            let current_status: String = job.get(1);
            if job_run_ref != event.run_ref {
                return Err(RunEventError::JobNotInRun);
            }
            if current_status != transition.expected_status.as_str() {
                return Err(RunEventError::JobStateConflict);
            }

            let updated = transaction
                .execute(
                    "UPDATE pulso_jobs SET status=$3, updated_at=CURRENT_TIMESTAMP \
                     WHERE tenant_id=$1 AND id=$2::uuid AND status=$4",
                    &[
                        &event.tenant_id,
                        &transition.job_ref,
                        &transition.next_status.as_str(),
                        &transition.expected_status.as_str(),
                    ],
                )
                .map_err(RunEventError::Storage)?;
            if updated != 1 {
                return Err(RunEventError::JobStateConflict);
            }
        }
        transaction
            .execute(
                "UPDATE pulso_jobs SET last_event_sequence=$3 \
                 WHERE tenant_id=$1 AND id=$2::uuid",
                &[&event.tenant_id, &event.run_ref, &next_sequence],
            )
            .map_err(RunEventError::Storage)?;
        let row = transaction
            .query_one(
                "INSERT INTO pulso_run_events \
                 (id, tenant_id, run_ref, job_ref, sequence, event_at, stage, event_code, status, \
                  reason_code, artifact_ref, trace_id, details_ref) \
                 VALUES ($1::uuid, $2, $3::uuid, $4::uuid, $5, clock_timestamp(), $6, $7, $8, \
                         $9, $10, $11, $12) RETURNING id::text",
                &[
                    &event.id,
                    &event.tenant_id,
                    &event.run_ref,
                    &event
                        .job_transition
                        .as_ref()
                        .map(|transition| &transition.job_ref),
                    &next_sequence,
                    &event.stage,
                    &event.event_code,
                    &event.status.as_str(),
                    &event.reason_code,
                    &event.artifact_ref,
                    &event.trace_id,
                    &event.details_ref,
                ],
            )
            .map_err(RunEventError::Storage)?;
        transaction.commit().map_err(RunEventError::Storage)?;
        Ok(AppendedRunEvent {
            id: row.get(0),
            sequence: next_sequence,
        })
    }
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn is_tenant_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn is_uuid_v7(value: &str) -> bool {
    is_uuid(value)
        && value.as_bytes().get(14) == Some(&b'7')
        && matches!(
            value.as_bytes().get(19),
            Some(b'8' | b'9' | b'a' | b'A' | b'b' | b'B')
        )
}

fn is_legal_transition(from: JobStatus, to: JobStatus) -> bool {
    matches!(
        (from, to),
        (JobStatus::Queued, JobStatus::Running | JobStatus::Deferred)
            | (
                JobStatus::Running,
                JobStatus::WaitingDependency
                    | JobStatus::RetryWait
                    | JobStatus::Complete
                    | JobStatus::Dead
            )
            | (
                JobStatus::Deferred | JobStatus::WaitingDependency | JobStatus::RetryWait,
                JobStatus::Queued | JobStatus::Superseded
            )
    )
}

fn is_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.' | b'/')
        })
}

#[cfg(test)]
mod tests {
    use super::{DurableRunEvent, JobStatus, JobTransition, RunEventError, RunEventStatus};

    #[test]
    fn durable_event_requires_uuid_v7_identity_and_allowlisted_codes() {
        let valid = DurableRunEvent::new(
            "00000000-0000-7000-8000-000000000001",
            "tenant_a",
            "00000000-0000-7000-8000-000000000002",
            Some(
                JobTransition::new(
                    "00000000-0000-7000-8000-000000000003",
                    JobStatus::Queued,
                    JobStatus::Running,
                )
                .unwrap(),
            ),
            "execution",
            "job_claimed",
            RunEventStatus::new("running").unwrap(),
            None,
            None,
            None,
            None,
        );
        assert!(valid.is_ok());

        let invalid_uuid_version = DurableRunEvent::new(
            "00000000-0000-4000-8000-000000000001",
            "tenant_a",
            "00000000-0000-7000-8000-000000000002",
            Some(
                JobTransition::new(
                    "00000000-0000-7000-8000-000000000003",
                    JobStatus::Queued,
                    JobStatus::Running,
                )
                .unwrap(),
            ),
            "execution",
            "job_claimed",
            RunEventStatus::new("running").unwrap(),
            None,
            None,
            None,
            None,
        );
        assert!(matches!(
            invalid_uuid_version,
            Err(RunEventError::InvalidEvent)
        ));

        assert!(RunEventStatus::new("contains spaces").is_err());
        assert!(matches!(
            JobTransition::new(
                "00000000-0000-7000-8000-000000000003",
                JobStatus::Complete,
                JobStatus::Queued,
            ),
            Err(RunEventError::IllegalJobTransition)
        ));
        assert!(matches!(
            JobTransition::new(
                "00000000-0000-7000-8000-000000000003",
                JobStatus::Queued,
                JobStatus::Cancelled,
            ),
            Err(RunEventError::IllegalJobTransition)
        ));
    }
}
