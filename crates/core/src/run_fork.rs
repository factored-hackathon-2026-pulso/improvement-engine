//! Attested, authorized forks of immutable historical runs.
//!
//! Requests cannot supply source/config/memory/cutoff: the reducer only forks
//! a run previously attested against immutable artifacts. Durable adapters must
//! atomically check idempotency, liveness and control state before inserting the
//! child and its audit event.

use crate::{ArtifactKind, ArtifactReference, ArtifactRepository};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForkControlState {
    Queued,
    Paused,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkAuthorization {
    tenant_id: String,
    actor_id: String,
    grant_id: String,
    reason: String,
    expected_control_state: ForkControlState,
    expected_control_version: u64,
}
impl ForkAuthorization {
    pub fn new(
        tenant_id: impl Into<String>,
        actor_id: impl Into<String>,
        grant_id: impl Into<String>,
        reason: impl Into<String>,
        expected_control_state: ForkControlState,
        expected_control_version: u64,
    ) -> Result<Self, RunForkError> {
        let value = Self {
            tenant_id: tenant_id.into(),
            actor_id: actor_id.into(),
            grant_id: grant_id.into(),
            reason: reason.into(),
            expected_control_state,
            expected_control_version,
        };
        validate_identifier(&value.tenant_id, "tenant_id")?;
        validate_identifier(&value.actor_id, "actor_id")?;
        validate_identifier(&value.grant_id, "grant_id")?;
        validate_identifier(&value.reason, "reason")?;
        if value.expected_control_version == 0 {
            return Err(RunForkError::InvalidControlVersion);
        }
        Ok(value)
    }
    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }
    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkRequest {
    replay_of: String,
    idempotency_key: String,
    authorization: ForkAuthorization,
}
impl ForkRequest {
    pub fn new(
        replay_of: impl Into<String>,
        idempotency_key: impl Into<String>,
        authorization: ForkAuthorization,
    ) -> Result<Self, RunForkError> {
        let value = Self {
            replay_of: replay_of.into(),
            idempotency_key: idempotency_key.into(),
            authorization,
        };
        validate_identifier(&value.replay_of, "replay_of")?;
        validate_identifier(&value.idempotency_key, "idempotency_key")?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AttestedOriginalRun {
    tenant_id: String,
    run_id: String,
    snapshot_ref: ArtifactReference,
    config_ref: ArtifactReference,
    memory_ref: ArtifactReference,
    cutoff_unix_seconds: u64,
    control_state: ForkControlState,
    control_version: u64,
    final_locked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkedRun {
    run_id: String,
    replay_of: String,
    snapshot_ref: ArtifactReference,
    config_ref: ArtifactReference,
    memory_ref: ArtifactReference,
    cutoff_unix_seconds: u64,
}
impl ForkedRun {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn replay_of(&self) -> &str {
        &self.replay_of
    }
    pub fn snapshot_ref(&self) -> &ArtifactReference {
        &self.snapshot_ref
    }
    pub fn config_ref(&self) -> &ArtifactReference {
        &self.config_ref
    }
    pub fn memory_ref(&self) -> &ArtifactReference {
        &self.memory_ref
    }
    pub fn cutoff_unix_seconds(&self) -> u64 {
        self.cutoff_unix_seconds
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkAuditEvent {
    event_id: String,
    tenant_id: String,
    replay_of: String,
    fork_run_id: String,
    actor_id: String,
    grant_id: String,
    reason: String,
    parent_control_version: u64,
}
impl ForkAuditEvent {
    pub fn reason(&self) -> &str {
        &self.reason
    }
    pub fn event_id(&self) -> &str {
        &self.event_id
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkReceipt {
    fork: ForkedRun,
    event: ForkAuditEvent,
}
impl ForkReceipt {
    pub fn fork(&self) -> &ForkedRun {
        &self.fork
    }
    pub fn event(&self) -> &ForkAuditEvent {
        &self.event
    }
}

/// Lifecycle/final-lock checks belong to the source authorities, never to a
/// caller-controlled string list in the fork request.
pub trait ForkReferencePolicy {
    fn is_live_for_fork(
        &mut self,
        tenant_id: &str,
        reference: &ArtifactReference,
        cutoff_unix_seconds: u64,
    ) -> bool;
}
#[derive(Default)]
pub struct InMemoryForkReferencePolicy {
    revoked: BTreeSet<String>,
    final_locked: BTreeSet<String>,
}
impl InMemoryForkReferencePolicy {
    pub fn revoke(&mut self, reference: &ArtifactReference) {
        self.revoked.insert(reference_key(reference));
    }
    pub fn final_lock(&mut self, reference: &ArtifactReference) {
        self.final_locked.insert(reference_key(reference));
    }
}
impl ForkReferencePolicy for InMemoryForkReferencePolicy {
    fn is_live_for_fork(&mut self, tenant_id: &str, reference: &ArtifactReference, _: u64) -> bool {
        reference.tenant_id == tenant_id
            && !self.revoked.contains(&reference_key(reference))
            && !self.final_locked.contains(&reference_key(reference))
    }
}

#[derive(Default)]
pub struct InMemoryForkGrantAuthority {
    grants: BTreeSet<(String, String, String)>,
}
impl InMemoryForkGrantAuthority {
    pub fn authorize(
        &mut self,
        tenant_id: impl Into<String>,
        actor_id: impl Into<String>,
        grant_id: impl Into<String>,
    ) -> Result<(), RunForkError> {
        let tenant_id = tenant_id.into();
        let actor_id = actor_id.into();
        let grant_id = grant_id.into();
        validate_identifier(&tenant_id, "tenant_id")?;
        validate_identifier(&actor_id, "actor_id")?;
        validate_identifier(&grant_id, "grant_id")?;
        self.grants.insert((tenant_id, actor_id, grant_id));
        Ok(())
    }
    fn allows(&self, auth: &ForkAuthorization) -> bool {
        self.grants.contains(&(
            auth.tenant_id.clone(),
            auth.actor_id.clone(),
            auth.grant_id.clone(),
        ))
    }
}

#[derive(Debug, Default)]
pub struct RunForkStore {
    originals: BTreeMap<(String, String), AttestedOriginalRun>,
    forks: BTreeMap<(String, String), ForkedRun>,
    idempotency: BTreeMap<(String, String), (String, ForkReceipt)>,
}
impl RunForkStore {
    pub fn new() -> Self {
        Self::default()
    }
    #[allow(clippy::too_many_arguments)]
    pub fn register_attested_original<R: ArtifactRepository, P: ForkReferencePolicy>(
        &mut self,
        tenant_id: impl Into<String>,
        run_id: impl Into<String>,
        snapshot_ref: ArtifactReference,
        config_ref: ArtifactReference,
        memory_ref: ArtifactReference,
        cutoff_unix_seconds: u64,
        control_state: ForkControlState,
        control_version: u64,
        final_locked: bool,
        artifacts: &mut R,
        policy: &mut P,
    ) -> Result<(), RunForkError> {
        let tenant_id = tenant_id.into();
        let run_id = run_id.into();
        validate_identifier(&tenant_id, "tenant_id")?;
        validate_identifier(&run_id, "run_id")?;
        if cutoff_unix_seconds == 0 {
            return Err(RunForkError::InvalidCutoff);
        }
        if control_version == 0 {
            return Err(RunForkError::InvalidControlVersion);
        }
        let original = AttestedOriginalRun {
            tenant_id: tenant_id.clone(),
            run_id: run_id.clone(),
            snapshot_ref,
            config_ref,
            memory_ref,
            cutoff_unix_seconds,
            control_state,
            control_version,
            final_locked,
        };
        validate_original(&original, artifacts, policy)?;
        if self
            .originals
            .insert((tenant_id, run_id), original)
            .is_some()
        {
            return Err(RunForkError::RunAlreadyRegistered);
        }
        Ok(())
    }
    pub fn fork<R: ArtifactRepository, P: ForkReferencePolicy>(
        &mut self,
        request: ForkRequest,
        artifacts: &mut R,
        policy: &mut P,
        grants: &InMemoryForkGrantAuthority,
    ) -> Result<ForkReceipt, RunForkError> {
        if !grants.allows(&request.authorization) {
            return Err(RunForkError::Unauthorized);
        }
        let key = (
            request.authorization.tenant_id.clone(),
            request.idempotency_key.clone(),
        );
        let digest = request_digest(&request);
        if let Some((recorded, receipt)) = self.idempotency.get(&key) {
            if recorded != &digest {
                return Err(RunForkError::IdempotencyConflict);
            }
            let parent = self.parent(&request)?;
            validate_original(parent, artifacts, policy)?;
            return Ok(receipt.clone());
        }
        let parent = self.parent(&request)?.clone();
        validate_original(&parent, artifacts, policy)?;
        if parent.control_state != request.authorization.expected_control_state
            || parent.control_version != request.authorization.expected_control_version
        {
            return Err(RunForkError::ControlStateConflict);
        }
        let run_id = format!("fork_{digest}");
        if self
            .forks
            .contains_key(&(parent.tenant_id.clone(), run_id.clone()))
        {
            return Err(RunForkError::ForkCollision);
        }
        let fork = ForkedRun {
            run_id: run_id.clone(),
            replay_of: parent.run_id.clone(),
            snapshot_ref: parent.snapshot_ref,
            config_ref: parent.config_ref,
            memory_ref: parent.memory_ref,
            cutoff_unix_seconds: parent.cutoff_unix_seconds,
        };
        let event = ForkAuditEvent {
            event_id: format!("fork_event_{digest}"),
            tenant_id: parent.tenant_id.clone(),
            replay_of: parent.run_id,
            fork_run_id: run_id.clone(),
            actor_id: request.authorization.actor_id.clone(),
            grant_id: request.authorization.grant_id.clone(),
            reason: request.authorization.reason.clone(),
            parent_control_version: parent.control_version,
        };
        let receipt = ForkReceipt {
            fork: fork.clone(),
            event,
        };
        self.forks.insert((parent.tenant_id, run_id), fork);
        self.idempotency.insert(key, (digest, receipt.clone()));
        Ok(receipt)
    }
    fn parent(&self, request: &ForkRequest) -> Result<&AttestedOriginalRun, RunForkError> {
        self.originals
            .get(&(
                request.authorization.tenant_id.clone(),
                request.replay_of.clone(),
            ))
            .ok_or(RunForkError::ParentNotFound)
    }
    pub fn fork_count(&self, tenant_id: &str) -> usize {
        self.forks
            .keys()
            .filter(|(tenant, _)| tenant == tenant_id)
            .count()
    }
}

fn validate_original<R: ArtifactRepository, P: ForkReferencePolicy>(
    original: &AttestedOriginalRun,
    artifacts: &mut R,
    policy: &mut P,
) -> Result<(), RunForkError> {
    if original.final_locked {
        return Err(RunForkError::FinalLocked);
    }
    validate_reference(
        artifacts,
        policy,
        &original.tenant_id,
        &original.snapshot_ref,
        ArtifactKind::SourceSnapshot,
        original.cutoff_unix_seconds,
        true,
    )?;
    validate_reference(
        artifacts,
        policy,
        &original.tenant_id,
        &original.config_ref,
        ArtifactKind::RunConfig,
        original.cutoff_unix_seconds,
        false,
    )?;
    validate_reference(
        artifacts,
        policy,
        &original.tenant_id,
        &original.memory_ref,
        ArtifactKind::MemoryWiki,
        original.cutoff_unix_seconds,
        false,
    )
}
fn validate_reference<R: ArtifactRepository, P: ForkReferencePolicy>(
    artifacts: &mut R,
    policy: &mut P,
    tenant_id: &str,
    reference: &ArtifactReference,
    expected_kind: ArtifactKind,
    cutoff: u64,
    exact_cutoff: bool,
) -> Result<(), RunForkError> {
    if reference.tenant_id != tenant_id || !policy.is_live_for_fork(tenant_id, reference, cutoff) {
        return Err(RunForkError::ReferenceUnavailable);
    }
    let artifact = artifacts
        .get(tenant_id, &reference.id, reference.revision)
        .map_err(|_| RunForkError::ReferenceUnavailable)?
        .ok_or(RunForkError::ReferenceUnavailable)?;
    if artifact.reference() != *reference
        || artifact.kind != expected_kind
        || artifact_is_final_locked(&artifact.payload)
    {
        return Err(RunForkError::ReferenceUnavailable);
    }
    let observed = artifact
        .payload
        .get("observed_cutoff_unix_seconds")
        .and_then(Value::as_u64);
    if (exact_cutoff && observed != Some(cutoff))
        || (!exact_cutoff && observed.is_some_and(|available| available > cutoff))
    {
        return Err(RunForkError::CutoffMismatch);
    }
    Ok(())
}
fn artifact_is_final_locked(payload: &Value) -> bool {
    payload
        .get("final_locked")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}
fn reference_key(reference: &ArtifactReference) -> String {
    format!(
        "{}:{}:{}:{}",
        reference.tenant_id, reference.id, reference.revision, reference.digest
    )
}
fn request_digest(request: &ForkRequest) -> String {
    let mut hasher = Sha256::new();
    for value in [
        &request.authorization.tenant_id,
        &request.replay_of,
        &request.idempotency_key,
        &request.authorization.actor_id,
        &request.authorization.grant_id,
        &request.authorization.reason,
    ] {
        hasher.update(value.len().to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.update([match request.authorization.expected_control_state {
        ForkControlState::Queued => 0,
        ForkControlState::Paused => 1,
        ForkControlState::Completed => 2,
    }]);
    hasher.update(request.authorization.expected_control_version.to_be_bytes());
    format!("sha256:{:x}", hasher.finalize())
}
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RunForkError {
    InvalidIdentifier { field: &'static str },
    InvalidCutoff,
    InvalidControlVersion,
    ParentNotFound,
    RunAlreadyRegistered,
    ReferenceUnavailable,
    CutoffMismatch,
    FinalLocked,
    Unauthorized,
    ControlStateConflict,
    IdempotencyConflict,
    ForkCollision,
}
impl fmt::Display for RunForkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RunForkError {}
fn validate_identifier(value: &str, field: &'static str) -> Result<(), RunForkError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(RunForkError::InvalidIdentifier { field });
    }
    Ok(())
}
