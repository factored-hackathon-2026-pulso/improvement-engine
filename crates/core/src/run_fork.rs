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
    expected_grant_version: u64,
    expected_control_state: ForkControlState,
    expected_control_version: u64,
}
impl ForkAuthorization {
    pub fn new(
        tenant_id: impl Into<String>,
        actor_id: impl Into<String>,
        grant_id: impl Into<String>,
        reason: impl Into<String>,
        expected_grant_version: u64,
        expected_control_state: ForkControlState,
        expected_control_version: u64,
    ) -> Result<Self, RunForkError> {
        let value = Self {
            tenant_id: tenant_id.into(),
            actor_id: actor_id.into(),
            grant_id: grant_id.into(),
            reason: reason.into(),
            expected_grant_version,
            expected_control_state,
            expected_control_version,
        };
        validate_identifier(&value.tenant_id, "tenant_id")?;
        validate_identifier(&value.actor_id, "actor_id")?;
        validate_identifier(&value.grant_id, "grant_id")?;
        validate_identifier(&value.reason, "reason")?;
        if value.expected_grant_version == 0 {
            return Err(RunForkError::InvalidGrantVersion);
        }
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

/// Current, versioned authorization read by a conditional fork commit. The
/// grant is deliberately resolved by the authority at commit time rather than
/// trusted from a request object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkGrant {
    version: u64,
}
impl ForkGrant {
    pub fn new(version: u64) -> Result<Self, RunForkError> {
        if version == 0 {
            return Err(RunForkError::InvalidGrantVersion);
        }
        Ok(Self { version })
    }
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }
}

/// Authorization authority used by a durable conditional commit. Implementors
/// must return `None` for a revoked or otherwise unavailable grant.
pub trait ForkGrantAuthority {
    fn current_grant(
        &mut self,
        tenant_id: &str,
        actor_id: &str,
        grant_id: &str,
    ) -> Option<ForkGrant>;
}

/// Dynamic run state is at a lifecycle authority, never caller input or a
/// mutable field cached in the fork binding. A durable adapter evaluates this
/// record conditionally with the child insert.
pub trait ForkRunLifecycle {
    fn current(&mut self, tenant_id: &str, run_id: &str) -> Option<RunLifecycle>;
}

/// One durable operation: it must conditionally revalidate grant revision,
/// lifecycle state/version/final lock, every attested live reference and the
/// idempotency digest, then persist child, receipt and audit event together.
/// An unavailable or changed precondition has no durable side effect.
pub trait ForkCommitPort {
    fn commit_conditionally(&mut self, request: ForkRequest) -> Result<ForkReceipt, RunForkError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunLifecycle {
    control_state: ForkControlState,
    control_version: u64,
    final_locked: bool,
}
impl RunLifecycle {
    pub fn new(
        control_state: ForkControlState,
        control_version: u64,
        final_locked: bool,
    ) -> Result<Self, RunForkError> {
        if control_version == 0 {
            return Err(RunForkError::InvalidControlVersion);
        }
        Ok(Self {
            control_state,
            control_version,
            final_locked,
        })
    }
}

#[derive(Default)]
pub struct InMemoryForkRunLifecycle {
    records: BTreeMap<(String, String), RunLifecycle>,
}
impl InMemoryForkRunLifecycle {
    pub fn attest(
        &mut self,
        tenant_id: impl Into<String>,
        run_id: impl Into<String>,
        lifecycle: RunLifecycle,
    ) -> Result<(), RunForkError> {
        let tenant_id = tenant_id.into();
        let run_id = run_id.into();
        validate_identifier(&tenant_id, "tenant_id")?;
        validate_identifier(&run_id, "run_id")?;
        self.records.insert((tenant_id, run_id), lifecycle);
        Ok(())
    }
    pub fn revoke_to_final_lock(&mut self, tenant_id: &str, run_id: &str) {
        if let Some(lifecycle) = self
            .records
            .get_mut(&(tenant_id.to_owned(), run_id.to_owned()))
        {
            lifecycle.final_locked = true;
        }
    }
    pub fn replace(&mut self, tenant_id: &str, run_id: &str, lifecycle: RunLifecycle) {
        self.records
            .insert((tenant_id.to_owned(), run_id.to_owned()), lifecycle);
    }
}
impl ForkRunLifecycle for InMemoryForkRunLifecycle {
    fn current(&mut self, tenant_id: &str, run_id: &str) -> Option<RunLifecycle> {
        self.records
            .get(&(tenant_id.to_owned(), run_id.to_owned()))
            .cloned()
    }
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
    grants: BTreeMap<(String, String, String), u64>,
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
        self.grants.insert((tenant_id, actor_id, grant_id), 1);
        Ok(())
    }
    pub fn authorize_at_version(
        &mut self,
        tenant_id: impl Into<String>,
        actor_id: impl Into<String>,
        grant_id: impl Into<String>,
        version: u64,
    ) -> Result<(), RunForkError> {
        if version == 0 {
            return Err(RunForkError::InvalidGrantVersion);
        }
        let tenant_id = tenant_id.into();
        let actor_id = actor_id.into();
        let grant_id = grant_id.into();
        validate_identifier(&tenant_id, "tenant_id")?;
        validate_identifier(&actor_id, "actor_id")?;
        validate_identifier(&grant_id, "grant_id")?;
        self.grants.insert((tenant_id, actor_id, grant_id), version);
        Ok(())
    }
    pub fn revoke(&mut self, tenant_id: &str, actor_id: &str, grant_id: &str) {
        self.grants.remove(&(
            tenant_id.to_owned(),
            actor_id.to_owned(),
            grant_id.to_owned(),
        ));
    }
}
impl ForkGrantAuthority for InMemoryForkGrantAuthority {
    fn current_grant(
        &mut self,
        tenant_id: &str,
        actor_id: &str,
        grant_id: &str,
    ) -> Option<ForkGrant> {
        self.grants
            .get(&(
                tenant_id.to_owned(),
                actor_id.to_owned(),
                grant_id.to_owned(),
            ))
            .copied()
            .and_then(|version| ForkGrant::new(version).ok())
    }
}

#[derive(Debug, Default)]
pub struct RunForkStore {
    originals: BTreeMap<(String, String), AttestedOriginalRun>,
    forks: BTreeMap<(String, String), ForkedRun>,
    audit_events: BTreeMap<(String, String), ForkAuditEvent>,
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
        let key = (tenant_id.clone(), run_id.clone());
        if self.originals.contains_key(&key) {
            return Err(RunForkError::RunAlreadyRegistered);
        }
        let original = AttestedOriginalRun {
            tenant_id: tenant_id.clone(),
            run_id: run_id.clone(),
            snapshot_ref,
            config_ref,
            memory_ref,
            cutoff_unix_seconds,
        };
        validate_original(&original, artifacts, policy)?;
        self.originals.insert(key, original);
        Ok(())
    }
    pub fn fork<
        R: ArtifactRepository,
        P: ForkReferencePolicy,
        G: ForkGrantAuthority,
        L: ForkRunLifecycle,
    >(
        &mut self,
        request: ForkRequest,
        artifacts: &mut R,
        policy: &mut P,
        grants: &mut G,
        lifecycle: &mut L,
    ) -> Result<ForkReceipt, RunForkError> {
        self.commit_conditional(request, artifacts, policy, grants, lifecycle)
    }

    fn commit_conditional<
        R: ArtifactRepository,
        P: ForkReferencePolicy,
        G: ForkGrantAuthority,
        L: ForkRunLifecycle,
    >(
        &mut self,
        request: ForkRequest,
        artifacts: &mut R,
        policy: &mut P,
        grants: &mut G,
        lifecycle: &mut L,
    ) -> Result<ForkReceipt, RunForkError> {
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
            let lifecycle_record = validate_conditions(
                parent,
                &request.authorization,
                artifacts,
                policy,
                grants,
                lifecycle,
            )?;
            if receipt.event.parent_control_version != lifecycle_record.control_version {
                return Err(RunForkError::ControlStateConflict);
            }
            return Ok(receipt.clone());
        }
        let parent = self.parent(&request)?.clone();
        // The first read is a preflight only. A durable implementation maps
        // the second, identical condition set to one transaction/CAS directly
        // before writing any record; this in-memory port makes that boundary
        // executable and testable without exposing its storage internals.
        validate_conditions(
            &parent,
            &request.authorization,
            artifacts,
            policy,
            grants,
            lifecycle,
        )?;
        let lifecycle_record = validate_conditions(
            &parent,
            &request.authorization,
            artifacts,
            policy,
            grants,
            lifecycle,
        )?;
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
            parent_control_version: lifecycle_record.control_version,
        };
        let receipt = ForkReceipt {
            fork: fork.clone(),
            event: event.clone(),
        };
        // No fallible work remains after the durable condition gate. These
        // three inserts are the in-memory equivalent of one transaction:
        // child, immutable audit event and idempotency receipt either all
        // become visible or none does.
        self.forks.insert((parent.tenant_id, run_id), fork);
        self.audit_events.insert(
            (
                receipt.event.tenant_id.clone(),
                receipt.event.event_id.clone(),
            ),
            event,
        );
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
    pub fn audit_count(&self, tenant_id: &str) -> usize {
        self.audit_events
            .keys()
            .filter(|(tenant, _)| tenant == tenant_id)
            .count()
    }
}

/// In-memory realization of the one conditional commit port. Its dependencies
/// are generic authority interfaces, so callers cannot accidentally rely on
/// `InMemoryForkGrantAuthority` semantics when implementing a durable adapter.
pub struct InMemoryForkCommitPort<'a, R, P, G, L> {
    store: &'a mut RunForkStore,
    artifacts: &'a mut R,
    policy: &'a mut P,
    grants: &'a mut G,
    lifecycle: &'a mut L,
}
impl<'a, R, P, G, L> InMemoryForkCommitPort<'a, R, P, G, L>
where
    R: ArtifactRepository,
    P: ForkReferencePolicy,
    G: ForkGrantAuthority,
    L: ForkRunLifecycle,
{
    pub fn new(
        store: &'a mut RunForkStore,
        artifacts: &'a mut R,
        policy: &'a mut P,
        grants: &'a mut G,
        lifecycle: &'a mut L,
    ) -> Self {
        Self {
            store,
            artifacts,
            policy,
            grants,
            lifecycle,
        }
    }
    pub fn fork_count(&self, tenant_id: &str) -> usize {
        self.store.fork_count(tenant_id)
    }
    pub fn audit_count(&self, tenant_id: &str) -> usize {
        self.store.audit_count(tenant_id)
    }
}
impl<R, P, G, L> ForkCommitPort for InMemoryForkCommitPort<'_, R, P, G, L>
where
    R: ArtifactRepository,
    P: ForkReferencePolicy,
    G: ForkGrantAuthority,
    L: ForkRunLifecycle,
{
    fn commit_conditionally(&mut self, request: ForkRequest) -> Result<ForkReceipt, RunForkError> {
        self.store.commit_conditional(
            request,
            self.artifacts,
            self.policy,
            self.grants,
            self.lifecycle,
        )
    }
}

fn validate_original<R: ArtifactRepository, P: ForkReferencePolicy>(
    original: &AttestedOriginalRun,
    artifacts: &mut R,
    policy: &mut P,
) -> Result<(), RunForkError> {
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

fn validate_conditions<
    R: ArtifactRepository,
    P: ForkReferencePolicy,
    G: ForkGrantAuthority,
    L: ForkRunLifecycle,
>(
    original: &AttestedOriginalRun,
    authorization: &ForkAuthorization,
    artifacts: &mut R,
    policy: &mut P,
    grants: &mut G,
    lifecycle: &mut L,
) -> Result<RunLifecycle, RunForkError> {
    validate_grant(authorization, grants)?;
    let lifecycle_record = validate_lifecycle(original, authorization, lifecycle)?;
    validate_original(original, artifacts, policy)?;
    Ok(lifecycle_record)
}
fn validate_grant<G: ForkGrantAuthority>(
    authorization: &ForkAuthorization,
    grants: &mut G,
) -> Result<(), RunForkError> {
    let grant = grants
        .current_grant(
            &authorization.tenant_id,
            &authorization.actor_id,
            &authorization.grant_id,
        )
        .ok_or(RunForkError::Unauthorized)?;
    if grant.version != authorization.expected_grant_version {
        return Err(RunForkError::Unauthorized);
    }
    Ok(())
}
fn validate_lifecycle<L: ForkRunLifecycle>(
    original: &AttestedOriginalRun,
    authorization: &ForkAuthorization,
    lifecycle: &mut L,
) -> Result<RunLifecycle, RunForkError> {
    let live = lifecycle
        .current(&original.tenant_id, &original.run_id)
        .ok_or(RunForkError::LifecycleUnavailable)?;
    if live.final_locked {
        return Err(RunForkError::FinalLocked);
    }
    if live.control_state != authorization.expected_control_state
        || live.control_version != authorization.expected_control_version
    {
        return Err(RunForkError::ControlStateConflict);
    }
    Ok(live)
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
    hasher.update(request.authorization.expected_grant_version.to_be_bytes());
    hasher.update(request.authorization.expected_control_version.to_be_bytes());
    format!("sha256:{:x}", hasher.finalize())
}
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RunForkError {
    InvalidIdentifier { field: &'static str },
    InvalidCutoff,
    InvalidControlVersion,
    InvalidGrantVersion,
    ParentNotFound,
    LifecycleUnavailable,
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
