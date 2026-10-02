//! Durable-job admission and execution-state boundary.
//!
//! This in-memory implementation is the executable reducer contract for a
//! later durable adapter. It owns no scheduler, network provider, Agent Core
//! execution, or mutable banking effect.

use crate::quota_grant::{
    QuotaGrantError, QuotaLedger, QuotaLimit, QuotaReceipt, QuotaReservation, QuotaWindow,
};
use crate::run_config::{RunConfig, RunConfigIdentity};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone)]
pub struct JobAdmissionRequest {
    tenant_id: String,
    trigger_idempotency_key: String,
    trigger_digest: String,
    run_config_identity: RunConfigIdentity,
    quota_reservation: QuotaReservation,
}

impl JobAdmissionRequest {
    pub fn new(
        tenant_id: impl Into<String>,
        trigger_idempotency_key: impl Into<String>,
        trigger_digest: impl Into<String>,
        run_config: RunConfig,
        quota_reservation: QuotaReservation,
    ) -> Result<Self, DurableJobError> {
        let tenant_id = tenant_id.into();
        let trigger_idempotency_key = trigger_idempotency_key.into();
        let trigger_digest = trigger_digest.into();
        validate_identifier(&tenant_id, "tenant_id")?;
        validate_identifier(&trigger_idempotency_key, "trigger_idempotency_key")?;
        if !is_sha256_digest(&trigger_digest) {
            return Err(DurableJobError::InvalidTriggerDigest);
        }
        if quota_reservation.tenant_id() != tenant_id {
            return Err(DurableJobError::QuotaTenantMismatch);
        }
        if quota_reservation.idempotency_key() != trigger_idempotency_key {
            return Err(DurableJobError::QuotaIdempotencyKeyMismatch);
        }
        if quota_reservation.config_identity() != run_config.identity() {
            return Err(DurableJobError::QuotaRunConfigMismatch);
        }
        Ok(Self {
            tenant_id,
            trigger_idempotency_key,
            trigger_digest,
            run_config_identity: run_config.identity().clone(),
            quota_reservation,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRef {
    job_id: String,
    tenant_id: String,
    run_config_identity: RunConfigIdentity,
}

impl JobRef {
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobAdmissionReceipt {
    job: Option<JobRef>,
    quota_receipt: QuotaReceipt,
}

impl JobAdmissionReceipt {
    /// `None` means admission was deferred by the governing quota/grant; it
    /// is intentionally not a job that a worker can lease or execute.
    pub fn job(&self) -> Option<&JobRef> {
        self.job.as_ref()
    }
    pub fn quota_receipt(&self) -> &QuotaReceipt {
        &self.quota_receipt
    }
    pub fn is_admitted(&self) -> bool {
        self.job.is_some()
    }
}

#[derive(Debug, Default)]
pub struct DurableJobStore {
    quota: QuotaLedger,
    jobs: BTreeMap<String, StoredJob>,
    idempotency: BTreeMap<(String, String), IdempotentAdmission>,
    trusted_reconciliation_authorities: BTreeSet<(String, String)>,
    control_commands: BTreeMap<(String, String, String), (String, JobControlReceipt)>,
}

/// Persistence seam for the U06 reducer. Production adapters must make
/// `admit_atomically` one transaction spanning the U05 quota reservation,
/// idempotency record and durable job record. They must perform lease/fence
/// predicates in the same conditional update, not as read-then-write steps.
///
/// The in-memory implementation below is the executable reference model; it
/// does not claim process-crash durability or distributed locking.
pub trait DurableJobRepository {
    fn admit_atomically(
        &mut self,
        request: JobAdmissionRequest,
    ) -> Result<JobAdmissionReceipt, DurableJobError>;
    fn acquire_job_lease(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        worker_id: &str,
        now_unix_seconds: u64,
        lease_seconds: u64,
    ) -> Result<JobLease, DurableJobError>;
    fn begin_job_effect_dispatch(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        worker_id: &str,
        fence_token: u64,
        now_unix_seconds: u64,
    ) -> Result<(), DurableJobError>;
    fn acknowledge_job_effect(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        worker_id: &str,
        fence_token: u64,
        effect_receipt: &str,
        now_unix_seconds: u64,
    ) -> Result<(), DurableJobError>;
    fn recover_job_after_restart(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        now_unix_seconds: u64,
    ) -> Result<RecoveryDisposition, DurableJobError>;
    fn reconcile_unknown_job(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        evidence: ReconciliationEvidence,
    ) -> Result<(), DurableJobError>;
    fn control_job_atomically(
        &mut self,
        command: JobControlCommand,
        recorded_at_unix_seconds: u64,
    ) -> Result<JobControlReceipt, DurableJobError>;
}

#[derive(Debug, Clone)]
struct StoredJob {
    receipt: JobAdmissionReceipt,
    next_fence_token: u64,
    attempt_count: u64,
    active_lease: Option<JobLease>,
    effect_state: JobEffectState,
    status: JobStatus,
    dispatch_fence_token: Option<u64>,
    reconciliation_evidence: Option<ReconciliationEvidence>,
    control_version: u64,
}
#[derive(Debug, Clone)]
struct IdempotentAdmission {
    request_digest: String,
    receipt: JobAdmissionReceipt,
}

impl DurableJobStore {
    pub fn control_job(
        &mut self,
        command: JobControlCommand,
        recorded_at_unix_seconds: u64,
    ) -> Result<JobControlReceipt, DurableJobError> {
        let fingerprint = command.fingerprint();
        let key = (
            command.tenant_id.clone(),
            command.job_id.clone(),
            command.idempotency_key.clone(),
        );
        if let Some((existing, receipt)) = self.control_commands.get(&key) {
            return if existing == &fingerprint {
                Ok(receipt.clone())
            } else {
                Err(DurableJobError::ControlIdempotencyConflict)
            };
        }
        let job = match self.job_mut(&command.tenant_id, &command.job_id) {
            // A control-plane caller must not learn that another tenant owns
            // an otherwise valid job identifier.
            Err(DurableJobError::TenantAccessDenied) => return Err(DurableJobError::JobNotFound),
            result => result?,
        };
        if job.control_version != command.expected_version
            || job.active_lease.as_ref().map(JobLease::fence_token) != command.expected_fence
        {
            return Err(DurableJobError::ControlVersionConflict);
        }
        let observed_status = job.status.clone();
        let observed_effect = job.effect_state.clone();
        let outcome = match (&command.kind, &job.status, &job.effect_state) {
            (_, _, JobEffectState::UnknownPendingReconciliation)
            | (_, JobStatus::UnknownPendingReconciliation, _)
            | (_, JobStatus::Leased { .. }, _) => JobControlOutcome::ReconciliationRequired,
            (JobControlKind::Pause, JobStatus::Queued, JobEffectState::NoEffect) => {
                job.status = JobStatus::Paused;
                JobControlOutcome::Paused
            }
            (
                JobControlKind::Cancel,
                JobStatus::Queued | JobStatus::Paused,
                JobEffectState::NoEffect,
            ) => {
                job.status = JobStatus::CancelledBeforeEffect;
                JobControlOutcome::CancelledBeforeEffect
            }
            _ => JobControlOutcome::AlreadyTerminal,
        };
        if matches!(
            outcome,
            JobControlOutcome::Paused | JobControlOutcome::CancelledBeforeEffect
        ) {
            advance_control_version(job)?;
        }
        let receipt = JobControlReceipt {
            tenant_id: command.tenant_id.clone(),
            job_id: command.job_id.clone(),
            operator_id: command.operator_id.clone(),
            idempotency_key: command.idempotency_key.clone(),
            expected_fence: command.expected_fence,
            requested: command.kind.clone(),
            observed_status,
            observed_effect,
            confirmed_status: job.status.clone(),
            confirmed_effect: job.effect_state.clone(),
            target_version: command.expected_version,
            confirmed_version: job.control_version,
            recorded_at_unix_seconds,
            fingerprint: fingerprint.clone(),
            outcome,
        };
        self.control_commands
            .insert(key, (fingerprint, receipt.clone()));
        Ok(receipt)
    }
    pub fn new() -> Self {
        Self::default()
    }

    pub fn configure_quota(&mut self, limit: QuotaLimit) -> Result<(), DurableJobError> {
        self.quota
            .configure_limit(limit)
            .map_err(DurableJobError::Quota)
    }

    /// Admits one durable job and its quota reservation through the same
    /// serialized reducer. The in-memory adapter is atomic within this store;
    /// a durable adapter must preserve this with one transaction.
    pub fn admit(
        &mut self,
        request: JobAdmissionRequest,
    ) -> Result<JobAdmissionReceipt, DurableJobError> {
        let scope = (
            request.tenant_id.clone(),
            request.trigger_idempotency_key.clone(),
        );
        let request_digest = admission_digest(&request);
        if let Some(existing) = self.idempotency.get(&scope) {
            if existing.request_digest == request_digest {
                return Ok(existing.receipt.clone());
            }
            return Err(DurableJobError::IdempotencyConflict {
                trigger_idempotency_key: request.trigger_idempotency_key,
            });
        }
        let quota_receipt = self
            .quota
            .reserve(request.quota_reservation.clone())
            .map_err(DurableJobError::Quota)?;
        if !quota_receipt.is_reserved() {
            let receipt = JobAdmissionReceipt {
                job: None,
                quota_receipt,
            };
            self.idempotency.insert(
                scope,
                IdempotentAdmission {
                    request_digest,
                    receipt: receipt.clone(),
                },
            );
            return Ok(receipt);
        }
        let job_id = job_id_for(&request);
        let job = JobRef {
            job_id: job_id.clone(),
            tenant_id: request.tenant_id.clone(),
            run_config_identity: request.run_config_identity,
        };
        let receipt = JobAdmissionReceipt {
            job: Some(job),
            quota_receipt,
        };
        self.jobs.insert(
            job_id,
            StoredJob {
                receipt: receipt.clone(),
                next_fence_token: 0,
                attempt_count: 0,
                active_lease: None,
                effect_state: JobEffectState::NoEffect,
                status: JobStatus::Queued,
                dispatch_fence_token: None,
                reconciliation_evidence: None,
                control_version: 1,
            },
        );
        self.idempotency.insert(
            scope,
            IdempotentAdmission {
                request_digest,
                receipt: receipt.clone(),
            },
        );
        Ok(receipt)
    }

    pub fn job_count(&self, tenant_id: &str) -> usize {
        self.jobs
            .values()
            .filter(|job| {
                job.receipt
                    .job
                    .as_ref()
                    .is_some_and(|job| job.tenant_id == tenant_id)
            })
            .count()
    }

    pub fn reserved_units(&self, window: &QuotaWindow) -> u64 {
        self.quota.reserved_units(window)
    }

    /// Test/reference authority registry. A production persistence adapter
    /// replaces this fixture seam with its policy/signature verifier; this
    /// store never invents an authority from an evidence string.
    pub fn trust_reconciliation_authority(
        &mut self,
        tenant_id: impl Into<String>,
        authority_ref: impl Into<String>,
    ) -> Result<(), DurableJobError> {
        let tenant_id = tenant_id.into();
        let authority_ref = authority_ref.into();
        validate_identifier(&tenant_id, "tenant_id")?;
        validate_identifier(&authority_ref, "authority_ref")?;
        self.trusted_reconciliation_authorities
            .insert((tenant_id, authority_ref));
        Ok(())
    }

    /// Acquires a time-bounded lease. Every successful replacement gets a
    /// strictly larger fence, so an earlier worker cannot mutate the job.
    pub fn acquire_lease(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        worker_id: impl Into<String>,
        now_unix_seconds: u64,
        lease_seconds: u64,
    ) -> Result<JobLease, DurableJobError> {
        let worker_id = worker_id.into();
        validate_identifier(&worker_id, "worker_id")?;
        if lease_seconds == 0 {
            return Err(DurableJobError::InvalidLeaseDuration);
        }
        let job = self.job_mut(tenant_id, job_id)?;
        if job.effect_state != JobEffectState::NoEffect {
            return Err(DurableJobError::ReconciliationRequired);
        }
        if !matches!(job.status, JobStatus::Queued | JobStatus::Leased { .. }) {
            return Err(DurableJobError::InvalidTransition);
        }
        if let Some(lease) = &job.active_lease
            && now_unix_seconds < lease.expires_at_unix_seconds
        {
            return Err(DurableJobError::LeaseStillActive);
        }
        job.next_fence_token = job
            .next_fence_token
            .checked_add(1)
            .ok_or(DurableJobError::FenceExhausted)?;
        job.attempt_count = job
            .attempt_count
            .checked_add(1)
            .ok_or(DurableJobError::AttemptExhausted)?;
        let lease = JobLease {
            worker_id,
            fence_token: job.next_fence_token,
            expires_at_unix_seconds: now_unix_seconds
                .checked_add(lease_seconds)
                .ok_or(DurableJobError::InvalidLeaseDuration)?,
        };
        job.active_lease = Some(lease.clone());
        job.status = JobStatus::Leased {
            attempt: job.attempt_count,
            fence_token: lease.fence_token,
            expires_at_unix_seconds: lease.expires_at_unix_seconds,
        };
        advance_control_version(job)?;
        Ok(lease)
    }

    /// Records the only safe pre-dispatch fact: an effect may have happened.
    /// The state is deliberately `UnknownPendingReconciliation`, never an
    /// implicit retry, until an acknowledged receipt is attached.
    pub fn begin_effect_dispatch(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        worker_id: &str,
        fence_token: u64,
        now_unix_seconds: u64,
    ) -> Result<(), DurableJobError> {
        let job = self.job_mut(tenant_id, job_id)?;
        require_active_fence(job, worker_id, fence_token, now_unix_seconds)?;
        if job.effect_state != JobEffectState::NoEffect {
            return Err(DurableJobError::InvalidTransition);
        }
        job.effect_state = JobEffectState::UnknownPendingReconciliation;
        job.status = JobStatus::UnknownPendingReconciliation;
        job.dispatch_fence_token = Some(fence_token);
        advance_control_version(job)?;
        Ok(())
    }

    pub fn acknowledge_effect(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        worker_id: &str,
        fence_token: u64,
        effect_receipt: impl Into<String>,
        now_unix_seconds: u64,
    ) -> Result<(), DurableJobError> {
        let effect_receipt = effect_receipt.into();
        if !is_effect_receipt(&effect_receipt) {
            return Err(DurableJobError::InvalidEffectReceipt);
        }
        let job = self.job_mut(tenant_id, job_id)?;
        require_active_fence(job, worker_id, fence_token, now_unix_seconds)?;
        if job.effect_state != JobEffectState::UnknownPendingReconciliation {
            return Err(DurableJobError::InvalidTransition);
        }
        job.effect_state = JobEffectState::AppliedAcknowledged { effect_receipt };
        job.active_lease = None;
        job.status = JobStatus::AppliedAcknowledged;
        advance_control_version(job)?;
        Ok(())
    }

    pub fn effect_state(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<JobEffectState, DurableJobError> {
        Ok(self.job(tenant_id, job_id)?.effect_state.clone())
    }

    pub fn status(&self, tenant_id: &str, job_id: &str) -> Result<JobStatus, DurableJobError> {
        Ok(self.job(tenant_id, job_id)?.status.clone())
    }

    /// Monotonic version of lifecycle/effect facts. A caller reads it before
    /// issuing a control command; the repository checks it in the same atomic
    /// transition as the command.
    pub fn control_version(&self, tenant_id: &str, job_id: &str) -> Result<u64, DurableJobError> {
        Ok(self.job(tenant_id, job_id)?.control_version)
    }

    pub fn active_fence(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<Option<u64>, DurableJobError> {
        Ok(self
            .job(tenant_id, job_id)?
            .active_lease
            .as_ref()
            .map(JobLease::fence_token))
    }

    /// Applies a durable reconciler observation to an unknown dispatch. This
    /// method is intentionally the only route out of `Unknown`; callers must
    /// retain a content-addressed evidence reference rather than retrying.
    pub fn reconcile_unknown(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        evidence: ReconciliationEvidence,
    ) -> Result<(), DurableJobError> {
        if evidence.tenant_id != tenant_id || evidence.job_id != job_id {
            return Err(DurableJobError::ReconciliationTargetMismatch);
        }
        if !self
            .trusted_reconciliation_authorities
            .contains(&(tenant_id.to_owned(), evidence.authority_ref.clone()))
        {
            return Err(DurableJobError::UntrustedReconciliationAuthority);
        }
        let job = self.job_mut(tenant_id, job_id)?;
        if job.effect_state != JobEffectState::UnknownPendingReconciliation {
            return Err(DurableJobError::InvalidTransition);
        }
        if job.dispatch_fence_token != Some(evidence.dispatch_fence_token) {
            return Err(DurableJobError::ReconciliationDispatchMismatch);
        }
        match &evidence.observed_effect {
            ReconciledEffect::NoEffect => {
                job.effect_state = JobEffectState::NoEffect;
                job.status = JobStatus::CompletedNoEffect;
            }
            ReconciledEffect::Applied { effect_receipt } => {
                job.effect_state = JobEffectState::AppliedAcknowledged {
                    effect_receipt: effect_receipt.clone(),
                };
                job.status = JobStatus::AppliedAcknowledged;
            }
        }
        job.active_lease = None;
        job.reconciliation_evidence = Some(evidence);
        advance_control_version(job)?;
        Ok(())
    }

    pub fn reconciliation_evidence(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<Option<ReconciliationEvidence>, DurableJobError> {
        Ok(self.job(tenant_id, job_id)?.reconciliation_evidence.clone())
    }

    /// Restart recovery is conservative: it can release only an expired lease
    /// whose reducer record proves `NoEffect`. A pre-dispatch record is already
    /// ambiguous and remains so until a separate reconciler supplies evidence.
    pub fn recover_after_restart(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        now_unix_seconds: u64,
    ) -> Result<RecoveryDisposition, DurableJobError> {
        let job = self.job_mut(tenant_id, job_id)?;
        if matches!(
            job.status,
            JobStatus::CompletedNoEffect
                | JobStatus::AppliedAcknowledged
                | JobStatus::Paused
                | JobStatus::CancelledBeforeEffect
        ) {
            return Ok(RecoveryDisposition::Terminal);
        }
        if job.effect_state == JobEffectState::UnknownPendingReconciliation {
            let changed =
                job.active_lease.is_some() || job.status != JobStatus::UnknownPendingReconciliation;
            job.active_lease = None;
            job.status = JobStatus::UnknownPendingReconciliation;
            if changed {
                advance_control_version(job)?;
            }
            return Ok(RecoveryDisposition::ReconciliationRequired);
        }
        if job.effect_state != JobEffectState::NoEffect {
            return Ok(RecoveryDisposition::Terminal);
        }
        match &job.active_lease {
            Some(lease) if now_unix_seconds < lease.expires_at_unix_seconds => {
                Ok(RecoveryDisposition::LeaseStillActive)
            }
            _ => {
                let changed = job.active_lease.is_some() || job.status != JobStatus::Queued;
                job.active_lease = None;
                job.status = JobStatus::Queued;
                if changed {
                    advance_control_version(job)?;
                }
                Ok(RecoveryDisposition::ReadyForLease)
            }
        }
    }

    fn job(&self, tenant_id: &str, job_id: &str) -> Result<&StoredJob, DurableJobError> {
        let job = self.jobs.get(job_id).ok_or(DurableJobError::JobNotFound)?;
        if job
            .receipt
            .job
            .as_ref()
            .is_none_or(|reference| reference.tenant_id != tenant_id)
        {
            return Err(DurableJobError::TenantAccessDenied);
        }
        Ok(job)
    }
    fn job_mut(
        &mut self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<&mut StoredJob, DurableJobError> {
        let job = self
            .jobs
            .get_mut(job_id)
            .ok_or(DurableJobError::JobNotFound)?;
        if job
            .receipt
            .job
            .as_ref()
            .is_none_or(|reference| reference.tenant_id != tenant_id)
        {
            return Err(DurableJobError::TenantAccessDenied);
        }
        Ok(job)
    }
}

impl DurableJobRepository for DurableJobStore {
    fn admit_atomically(
        &mut self,
        request: JobAdmissionRequest,
    ) -> Result<JobAdmissionReceipt, DurableJobError> {
        self.admit(request)
    }
    fn acquire_job_lease(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        worker_id: &str,
        now_unix_seconds: u64,
        lease_seconds: u64,
    ) -> Result<JobLease, DurableJobError> {
        self.acquire_lease(
            tenant_id,
            job_id,
            worker_id,
            now_unix_seconds,
            lease_seconds,
        )
    }
    fn begin_job_effect_dispatch(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        worker_id: &str,
        fence_token: u64,
        now_unix_seconds: u64,
    ) -> Result<(), DurableJobError> {
        self.begin_effect_dispatch(tenant_id, job_id, worker_id, fence_token, now_unix_seconds)
    }
    fn acknowledge_job_effect(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        worker_id: &str,
        fence_token: u64,
        effect_receipt: &str,
        now_unix_seconds: u64,
    ) -> Result<(), DurableJobError> {
        self.acknowledge_effect(
            tenant_id,
            job_id,
            worker_id,
            fence_token,
            effect_receipt,
            now_unix_seconds,
        )
    }
    fn recover_job_after_restart(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        now_unix_seconds: u64,
    ) -> Result<RecoveryDisposition, DurableJobError> {
        self.recover_after_restart(tenant_id, job_id, now_unix_seconds)
    }
    fn reconcile_unknown_job(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        evidence: ReconciliationEvidence,
    ) -> Result<(), DurableJobError> {
        self.reconcile_unknown(tenant_id, job_id, evidence)
    }
    fn control_job_atomically(
        &mut self,
        command: JobControlCommand,
        recorded_at_unix_seconds: u64,
    ) -> Result<JobControlReceipt, DurableJobError> {
        self.control_job(command, recorded_at_unix_seconds)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLease {
    worker_id: String,
    fence_token: u64,
    expires_at_unix_seconds: u64,
}
impl JobLease {
    pub fn fence_token(&self) -> u64 {
        self.fence_token
    }
    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }
}

/// Three mutually exclusive effect facts. `UnknownPendingReconciliation`
/// means no caller may replay or re-lease work as if the effect were absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobEffectState {
    NoEffect,
    AppliedAcknowledged { effect_receipt: String },
    UnknownPendingReconciliation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Leased {
        attempt: u64,
        fence_token: u64,
        expires_at_unix_seconds: u64,
    },
    UnknownPendingReconciliation,
    CompletedNoEffect,
    AppliedAcknowledged,
    Paused,
    CancelledBeforeEffect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobControlKind {
    Pause,
    Cancel,
}
#[derive(Debug, Clone)]
pub struct JobControlCommand {
    tenant_id: String,
    job_id: String,
    operator_id: String,
    idempotency_key: String,
    expected_version: u64,
    expected_fence: Option<u64>,
    kind: JobControlKind,
}
impl JobControlCommand {
    pub fn new(
        tenant_id: impl Into<String>,
        job_id: impl Into<String>,
        operator_id: impl Into<String>,
        idempotency_key: impl Into<String>,
        expected_version: u64,
        expected_fence: Option<u64>,
        kind: JobControlKind,
    ) -> Result<Self, DurableJobError> {
        let tenant_id = tenant_id.into();
        let job_id = job_id.into();
        let operator_id = operator_id.into();
        let idempotency_key = idempotency_key.into();
        if validate_identifier(&tenant_id, "tenant_id").is_err()
            || !is_job_id(&job_id)
            || validate_identifier(&operator_id, "operator_id").is_err()
            || validate_identifier(&idempotency_key, "idempotency_key").is_err()
            || expected_version == 0
            || expected_fence.is_some_and(|fence| fence == 0)
        {
            return Err(DurableJobError::InvalidControlCommand);
        }
        Ok(Self {
            tenant_id,
            job_id,
            operator_id,
            idempotency_key,
            expected_version,
            expected_fence,
            kind,
        })
    }
    fn fingerprint(&self) -> String {
        format!(
            "sha256:{:x}",
            Sha256::digest(
                format!(
                    "{}\n{}\n{}\n{}\n{}\n{:?}\n{:?}",
                    self.tenant_id,
                    self.job_id,
                    self.operator_id,
                    self.idempotency_key,
                    self.expected_version,
                    self.expected_fence,
                    self.kind
                )
                .as_bytes()
            )
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobControlOutcome {
    Paused,
    CancelledBeforeEffect,
    ReconciliationRequired,
    AlreadyTerminal,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobControlReceipt {
    tenant_id: String,
    job_id: String,
    operator_id: String,
    idempotency_key: String,
    expected_fence: Option<u64>,
    requested: JobControlKind,
    observed_status: JobStatus,
    observed_effect: JobEffectState,
    confirmed_status: JobStatus,
    confirmed_effect: JobEffectState,
    target_version: u64,
    confirmed_version: u64,
    recorded_at_unix_seconds: u64,
    fingerprint: String,
    outcome: JobControlOutcome,
}
impl JobControlReceipt {
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    pub fn operator_id(&self) -> &str {
        &self.operator_id
    }
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
    pub fn expected_fence(&self) -> Option<u64> {
        self.expected_fence
    }
    pub fn outcome(&self) -> &JobControlOutcome {
        &self.outcome
    }
    pub fn requested(&self) -> &JobControlKind {
        &self.requested
    }
    pub fn observed_status(&self) -> &JobStatus {
        &self.observed_status
    }
    pub fn observed_effect(&self) -> &JobEffectState {
        &self.observed_effect
    }
    pub fn confirmed_status(&self) -> &JobStatus {
        &self.confirmed_status
    }
    pub fn confirmed_effect(&self) -> &JobEffectState {
        &self.confirmed_effect
    }
    pub fn target_version(&self) -> u64 {
        self.target_version
    }
    pub fn confirmed_version(&self) -> u64 {
        self.confirmed_version
    }
    pub fn recorded_at_unix_seconds(&self) -> u64 {
        self.recorded_at_unix_seconds
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationEvidence {
    tenant_id: String,
    job_id: String,
    dispatch_fence_token: u64,
    authority_ref: String,
    evidence_digest: String,
    observed_effect: ReconciledEffect,
}

impl ReconciliationEvidence {
    pub fn no_effect(
        tenant_id: impl Into<String>,
        job_id: impl Into<String>,
        dispatch_fence_token: u64,
        authority_ref: impl Into<String>,
        evidence_digest: impl Into<String>,
    ) -> Result<Self, DurableJobError> {
        let tenant_id = tenant_id.into();
        let job_id = job_id.into();
        let authority_ref = authority_ref.into();
        let evidence_digest = evidence_digest.into();
        if !is_reconciliation_target(&tenant_id, &job_id, dispatch_fence_token, &authority_ref)
            || !is_sha256_digest(&evidence_digest)
        {
            return Err(DurableJobError::InvalidReconciliationEvidence);
        }
        Ok(Self {
            tenant_id,
            job_id,
            dispatch_fence_token,
            authority_ref,
            evidence_digest,
            observed_effect: ReconciledEffect::NoEffect,
        })
    }

    pub fn applied(
        tenant_id: impl Into<String>,
        job_id: impl Into<String>,
        dispatch_fence_token: u64,
        authority_ref: impl Into<String>,
        evidence_digest: impl Into<String>,
        effect_receipt: impl Into<String>,
    ) -> Result<Self, DurableJobError> {
        let tenant_id = tenant_id.into();
        let job_id = job_id.into();
        let authority_ref = authority_ref.into();
        let evidence_digest = evidence_digest.into();
        let effect_receipt = effect_receipt.into();
        if !is_reconciliation_target(&tenant_id, &job_id, dispatch_fence_token, &authority_ref)
            || !is_sha256_digest(&evidence_digest)
            || !is_effect_receipt(&effect_receipt)
        {
            return Err(DurableJobError::InvalidReconciliationEvidence);
        }
        Ok(Self {
            tenant_id,
            job_id,
            dispatch_fence_token,
            authority_ref,
            evidence_digest,
            observed_effect: ReconciledEffect::Applied { effect_receipt },
        })
    }

    pub fn evidence_digest(&self) -> &str {
        &self.evidence_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciledEffect {
    NoEffect,
    Applied { effect_receipt: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryDisposition {
    ReadyForLease,
    LeaseStillActive,
    ReconciliationRequired,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableJobError {
    InvalidIdentifier { field: &'static str },
    InvalidTriggerDigest,
    QuotaTenantMismatch,
    QuotaIdempotencyKeyMismatch,
    QuotaRunConfigMismatch,
    InvalidLeaseDuration,
    FenceExhausted,
    AttemptExhausted,
    InvalidEffectReceipt,
    InvalidReconciliationEvidence,
    ReconciliationTargetMismatch,
    ReconciliationDispatchMismatch,
    UntrustedReconciliationAuthority,
    JobNotFound,
    TenantAccessDenied,
    LeaseStillActive,
    StaleFence,
    LeaseOwnerMismatch,
    ReconciliationRequired,
    InvalidTransition,
    InvalidControlCommand,
    ControlIdempotencyConflict,
    ControlVersionConflict,
    IdempotencyConflict { trigger_idempotency_key: String },
    Quota(QuotaGrantError),
}

impl fmt::Display for DurableJobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field } => {
                write!(f, "{field} must be a lowercase identifier")
            }
            Self::InvalidTriggerDigest => f.write_str("trigger digest must be sha256"),
            Self::QuotaTenantMismatch => {
                f.write_str("quota reservation tenant differs from job tenant")
            }
            Self::QuotaIdempotencyKeyMismatch => {
                f.write_str("quota reservation idempotency key differs from trigger key")
            }
            Self::QuotaRunConfigMismatch => {
                f.write_str("quota reservation config differs from job config")
            }
            Self::InvalidLeaseDuration => {
                f.write_str("lease duration must be positive and non-overflowing")
            }
            Self::FenceExhausted => f.write_str("fence token exhausted"),
            Self::AttemptExhausted => f.write_str("attempt number exhausted"),
            Self::InvalidEffectReceipt => f.write_str("effect receipt must be effect:sha256"),
            Self::InvalidReconciliationEvidence => f.write_str(
                "reconciliation evidence must be sha256 and carry a valid observed effect",
            ),
            Self::ReconciliationTargetMismatch => {
                f.write_str("reconciliation evidence is for another tenant or job")
            }
            Self::ReconciliationDispatchMismatch => {
                f.write_str("reconciliation evidence is for another dispatch fence")
            }
            Self::UntrustedReconciliationAuthority => {
                f.write_str("reconciliation authority is not trusted for this tenant")
            }
            Self::JobNotFound => f.write_str("job not found"),
            Self::TenantAccessDenied => f.write_str("tenant cannot access this job"),
            Self::LeaseStillActive => f.write_str("job already has an active lease"),
            Self::StaleFence => f.write_str("worker fence is stale or expired"),
            Self::LeaseOwnerMismatch => f.write_str("worker does not own the active lease"),
            Self::ReconciliationRequired => {
                f.write_str("job effect is unknown and requires reconciliation")
            }
            Self::InvalidTransition => f.write_str("job state transition is not allowed"),
            Self::InvalidControlCommand => f.write_str(
                "control command has invalid tenant, job, operator, idempotency or version fields",
            ),
            Self::ControlIdempotencyConflict => {
                f.write_str("control idempotency key has a different semantic command")
            }
            Self::ControlVersionConflict => f.write_str(
                "control command is stale for the job lifecycle/effect version or fence",
            ),
            Self::IdempotencyConflict {
                trigger_idempotency_key,
            } => write!(
                f,
                "trigger idempotency key {trigger_idempotency_key} has a different request"
            ),
            Self::Quota(error) => write!(f, "quota admission failed: {error}"),
        }
    }
}
impl std::error::Error for DurableJobError {}

fn job_id_for(request: &JobAdmissionRequest) -> String {
    digest(&[
        ("kind", "durable_job"),
        ("tenant_id", &request.tenant_id),
        ("run_config_identity", request.run_config_identity.as_str()),
        ("trigger_idempotency_key", &request.trigger_idempotency_key),
    ])
}
fn advance_control_version(job: &mut StoredJob) -> Result<(), DurableJobError> {
    job.control_version = job
        .control_version
        .checked_add(1)
        .ok_or(DurableJobError::ControlVersionConflict)?;
    Ok(())
}
fn admission_digest(request: &JobAdmissionRequest) -> String {
    let quota_digest = request.quota_reservation.request_digest();
    digest(&[
        ("kind", "job_admission"),
        ("tenant_id", &request.tenant_id),
        ("run_config_identity", request.run_config_identity.as_str()),
        ("trigger_idempotency_key", &request.trigger_idempotency_key),
        ("trigger_digest", &request.trigger_digest),
        ("quota_request_digest", &quota_digest),
    ])
}
fn digest(parts: &[(&str, &str)]) -> String {
    let mut canonical = String::new();
    for (name, value) in parts {
        canonical.push_str(name);
        canonical.push(':');
        canonical.push_str(&value.len().to_string());
        canonical.push(':');
        canonical.push_str(value);
        canonical.push('\n');
    }
    format!("job:sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}
fn validate_identifier(value: &str, field: &'static str) -> Result<(), DurableJobError> {
    let mut chars = value.chars();
    if !matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(DurableJobError::InvalidIdentifier { field });
    }
    Ok(())
}
fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|c| matches!(*c, b'0'..=b'9' | b'a'..=b'f'))
}
fn is_effect_receipt(value: &str) -> bool {
    value.len() == 78
        && value.starts_with("effect:sha256:")
        && value.as_bytes()[14..]
            .iter()
            .all(|c| matches!(*c, b'0'..=b'9' | b'a'..=b'f'))
}
fn is_reconciliation_target(
    tenant_id: &str,
    job_id: &str,
    dispatch_fence_token: u64,
    authority_ref: &str,
) -> bool {
    validate_identifier(tenant_id, "tenant_id").is_ok()
        && validate_identifier(authority_ref, "authority_ref").is_ok()
        && is_job_id(job_id)
        && dispatch_fence_token > 0
}
fn is_job_id(value: &str) -> bool {
    value.len() == 75
        && value.starts_with("job:sha256:")
        && value.as_bytes()[11..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}
fn require_active_fence(
    job: &StoredJob,
    worker_id: &str,
    fence_token: u64,
    now_unix_seconds: u64,
) -> Result<(), DurableJobError> {
    match &job.active_lease {
        Some(lease)
            if lease.fence_token == fence_token
                && now_unix_seconds < lease.expires_at_unix_seconds
                && lease.worker_id == worker_id =>
        {
            Ok(())
        }
        Some(lease)
            if lease.fence_token == fence_token
                && now_unix_seconds < lease.expires_at_unix_seconds =>
        {
            Err(DurableJobError::LeaseOwnerMismatch)
        }
        _ => Err(DurableJobError::StaleFence),
    }
}
