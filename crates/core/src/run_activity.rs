//! Tenant-bound run-activity API contract and bounded read projection.
//!
//! No HTTP server is selected here. A transport authenticates a caller, builds
//! [`AuthenticatedTenant`], and delegates pagination/stream batches to this
//! module; it must not recreate cursor or tenant checks at the edge.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::Sha256;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Bound::{Excluded, Unbounded};

type HmacSha256 = Hmac<Sha256>;
const MAX_PAGE_SIZE: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityKind {
    Admitted,
    LeaseAcquired,
    EffectDispatchStarted,
    EffectAcknowledged,
    ReconciliationRequired,
    ReconciledNoEffect,
    ReconciledApplied,
}

/// Tenant identity already authenticated by the transport/policy layer.
/// This type is intentionally not proof of identity by itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedTenant(String);

impl AuthenticatedTenant {
    pub fn new(tenant_id: impl Into<String>) -> Result<Self, RunActivityError> {
        let tenant_id = tenant_id.into();
        if !is_identifier(&tenant_id) {
            return Err(RunActivityError::InvalidTenantBinding);
        }
        Ok(Self(tenant_id))
    }
    pub fn tenant_id(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityEvent {
    tenant_id: String,
    job_id: String,
    event_id: String,
    occurred_at_unix_seconds: u64,
    kind: ActivityKind,
    evidence_digest: String,
}

impl ActivityEvent {
    pub fn new(
        tenant_id: impl Into<String>,
        job_id: impl Into<String>,
        event_id: impl Into<String>,
        occurred_at_unix_seconds: u64,
        kind: ActivityKind,
        evidence_digest: impl Into<String>,
    ) -> Result<Self, RunActivityError> {
        let tenant_id = tenant_id.into();
        let job_id = job_id.into();
        let event_id = event_id.into();
        let evidence_digest = evidence_digest.into();
        if !is_identifier(&tenant_id)
            || !is_job_id(&job_id)
            || !is_identifier(&event_id)
            || !is_sha256_digest(&evidence_digest)
        {
            return Err(RunActivityError::InvalidEvent);
        }
        Ok(Self {
            tenant_id,
            job_id,
            event_id,
            occurred_at_unix_seconds,
            kind,
            evidence_digest,
        })
    }
    pub fn event_id(&self) -> &str {
        &self.event_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityEntry {
    event_id: String,
    occurred_at_unix_seconds: u64,
    kind: ActivityKind,
    evidence_digest: String,
}

impl ActivityEntry {
    pub fn new(
        event_id: impl Into<String>,
        occurred_at_unix_seconds: u64,
        kind: ActivityKind,
        evidence_digest: impl Into<String>,
    ) -> Result<Self, RunActivityError> {
        let event_id = event_id.into();
        let evidence_digest = evidence_digest.into();
        if !is_identifier(&event_id) || !is_sha256_digest(&evidence_digest) {
            return Err(RunActivityError::InvalidEvent);
        }
        Ok(Self {
            event_id,
            occurred_at_unix_seconds,
            kind,
            evidence_digest,
        })
    }
    pub fn event_id(&self) -> &str {
        &self.event_id
    }
    pub fn occurred_at_unix_seconds(&self) -> u64 {
        self.occurred_at_unix_seconds
    }
    pub fn kind(&self) -> &ActivityKind {
        &self.kind
    }
    pub fn evidence_digest(&self) -> &str {
        &self.evidence_digest
    }
    fn position(&self) -> ActivityPosition {
        ActivityPosition {
            occurred_at_unix_seconds: self.occurred_at_unix_seconds,
            event_id: self.event_id.clone(),
        }
    }
}

/// Total ordered key used by an adapter index and cursor continuation.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct ActivityPosition {
    occurred_at_unix_seconds: u64,
    event_id: String,
}

#[derive(Deserialize)]
struct PersistedActivityPosition {
    occurred_at_unix_seconds: u64,
    event_id: String,
}

impl<'de> Deserialize<'de> for ActivityPosition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let persisted = PersistedActivityPosition::deserialize(deserializer)?;
        Self::new(persisted.occurred_at_unix_seconds, persisted.event_id)
            .map_err(serde::de::Error::custom)
    }
}

impl ActivityPosition {
    pub fn new(
        occurred_at_unix_seconds: u64,
        event_id: impl Into<String>,
    ) -> Result<Self, RunActivityError> {
        let event_id = event_id.into();
        if !is_identifier(&event_id) {
            return Err(RunActivityError::InvalidCursor);
        }
        Ok(Self {
            occurred_at_unix_seconds,
            event_id,
        })
    }
    pub fn occurred_at_unix_seconds(&self) -> u64 {
        self.occurred_at_unix_seconds
    }
    pub fn event_id(&self) -> &str {
        &self.event_id
    }
}

/// A bounded read result. Adapters can construct it without exposing storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityTimeline {
    revision: u64,
    purged_through_revision: u64,
    entries: Vec<ActivityEntry>,
    has_more: bool,
}

impl ActivityTimeline {
    pub fn new(
        revision: u64,
        purged_through_revision: u64,
        entries: Vec<ActivityEntry>,
        has_more: bool,
    ) -> Result<Self, RunActivityError> {
        if revision == 0
            || purged_through_revision > revision
            || entries.len() > MAX_PAGE_SIZE
            || (entries.is_empty() && has_more)
        {
            return Err(RunActivityError::InvalidReadModelResponse);
        }
        if entries
            .windows(2)
            .any(|pair| pair[0].position() >= pair[1].position())
        {
            return Err(RunActivityError::InvalidReadModelResponse);
        }
        Ok(Self {
            revision,
            purged_through_revision,
            entries,
            has_more,
        })
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn purged_through_revision(&self) -> u64 {
        self.purged_through_revision
    }
    pub fn entries(&self) -> &[ActivityEntry] {
        &self.entries
    }
    pub fn has_more(&self) -> bool {
        self.has_more
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityReadQuery {
    tenant: AuthenticatedTenant,
    job_id: String,
    after: Option<ActivityPosition>,
    limit: usize,
}

impl ActivityReadQuery {
    pub fn new(
        tenant: AuthenticatedTenant,
        job_id: impl Into<String>,
        after: Option<ActivityPosition>,
        limit: usize,
    ) -> Result<Self, RunActivityError> {
        let job_id = job_id.into();
        if !is_job_id(&job_id) {
            return Err(RunActivityError::RunNotFound);
        }
        if limit == 0 || limit > MAX_PAGE_SIZE {
            return Err(RunActivityError::InvalidPageSize);
        }
        Ok(Self {
            tenant,
            job_id,
            after,
            limit,
        })
    }
    pub fn tenant(&self) -> &AuthenticatedTenant {
        &self.tenant
    }
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    pub fn after(&self) -> Option<&ActivityPosition> {
        self.after.as_ref()
    }
    pub fn limit(&self) -> usize {
        self.limit
    }
}

/// Production adapters use `after` plus `limit` to query an indexed projection
/// rather than materializing an entire run before slicing it.
pub trait RunActivityReadModel {
    fn read_activity(
        &self,
        query: &ActivityReadQuery,
    ) -> Result<ActivityTimeline, RunActivityError>;
}

#[derive(Debug, Default)]
pub struct RunActivityProjection {
    runs: BTreeMap<(String, String), RunTimeline>,
}

#[derive(Debug, Default)]
struct RunTimeline {
    revision: u64,
    purged_through_revision: u64,
    events: BTreeMap<String, StoredEvent>,
    order: BTreeMap<ActivityPosition, String>,
}
#[derive(Debug)]
struct StoredEvent {
    event: ActivityEvent,
    projection_revision: u64,
}

impl RunActivityProjection {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn project(&mut self, event: ActivityEvent) -> Result<(), RunActivityError> {
        let key = (event.tenant_id.clone(), event.job_id.clone());
        let timeline = self.runs.entry(key).or_default();
        if let Some(existing) = timeline.events.get(&event.event_id) {
            return if existing.event == event {
                Ok(())
            } else {
                Err(RunActivityError::EventConflict)
            };
        }
        timeline.revision = timeline
            .revision
            .checked_add(1)
            .ok_or(RunActivityError::RevisionExhausted)?;
        let position = ActivityPosition {
            occurred_at_unix_seconds: event.occurred_at_unix_seconds,
            event_id: event.event_id.clone(),
        };
        timeline.order.insert(position, event.event_id.clone());
        timeline.events.insert(
            event.event_id.clone(),
            StoredEvent {
                event,
                projection_revision: timeline.revision,
            },
        );
        Ok(())
    }
    /// Deletes retained event content for one run; all cursors at/before the
    /// retention revision are explicitly invalidated on their next read.
    pub fn purge_before_revision(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        revision: u64,
    ) -> Result<(), RunActivityError> {
        let Some(timeline) = self
            .runs
            .get_mut(&(tenant_id.to_owned(), job_id.to_owned()))
        else {
            return Err(RunActivityError::RunNotFound);
        };
        if revision > timeline.revision {
            return Err(RunActivityError::InvalidPurgeRevision);
        }
        timeline.purged_through_revision = timeline.purged_through_revision.max(revision);
        let remove: Vec<_> = timeline
            .events
            .iter()
            .filter(|(_, stored)| stored.projection_revision <= timeline.purged_through_revision)
            .map(|(event_id, stored)| {
                (
                    event_id.clone(),
                    ActivityPosition {
                        occurred_at_unix_seconds: stored.event.occurred_at_unix_seconds,
                        event_id: event_id.clone(),
                    },
                )
            })
            .collect();
        for (event_id, position) in remove {
            timeline.events.remove(&event_id);
            timeline.order.remove(&position);
        }
        Ok(())
    }
    pub fn revision(&self, tenant_id: &str, job_id: &str) -> Option<u64> {
        self.runs
            .get(&(tenant_id.to_owned(), job_id.to_owned()))
            .map(|run| run.revision)
    }
}

impl RunActivityReadModel for RunActivityProjection {
    fn read_activity(
        &self,
        query: &ActivityReadQuery,
    ) -> Result<ActivityTimeline, RunActivityError> {
        let run = self
            .runs
            .get(&(query.tenant.0.clone(), query.job_id.clone()))
            .ok_or(RunActivityError::RunNotFound)?;
        let mut entries = Vec::with_capacity(query.limit);
        let mut has_more = false;
        let mut append = |event_id: &String| {
            if entries.len() == query.limit {
                has_more = true;
                return false;
            }
            let stored = run
                .events
                .get(event_id)
                .expect("order index and event store are updated atomically");
            entries.push(ActivityEntry {
                event_id: stored.event.event_id.clone(),
                occurred_at_unix_seconds: stored.event.occurred_at_unix_seconds,
                kind: stored.event.kind.clone(),
                evidence_digest: stored.event.evidence_digest.clone(),
            });
            true
        };
        match query.after() {
            Some(position) => {
                for (_, event_id) in run.order.range((Excluded(position.clone()), Unbounded)) {
                    if !append(event_id) {
                        break;
                    }
                }
            }
            None => {
                for event_id in run.order.values() {
                    if !append(event_id) {
                        break;
                    }
                }
            }
        }
        ActivityTimeline::new(run.revision, run.purged_through_revision, entries, has_more)
    }
}

/// Test/local composition seam for a projection that is updated independently
/// of a long-lived API handler. Production adapters normally provide their own
/// concurrent durable read-model implementation.
impl RunActivityReadModel for RefCell<RunActivityProjection> {
    fn read_activity(
        &self,
        query: &ActivityReadQuery,
    ) -> Result<ActivityTimeline, RunActivityError> {
        self.borrow().read_activity(query)
    }
}

/// Request DTO passed after transport authentication; raw tenant IDs are not
/// accepted by the public handler API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListRunActivityRequest {
    tenant: AuthenticatedTenant,
    job_id: String,
    cursor: Option<String>,
    page_size: usize,
}
impl ListRunActivityRequest {
    pub fn new(
        tenant: AuthenticatedTenant,
        job_id: impl Into<String>,
        cursor: Option<String>,
        page_size: usize,
    ) -> Result<Self, RunActivityError> {
        let job_id = job_id.into();
        ActivityReadQuery::new(tenant.clone(), job_id.clone(), None, page_size)?;
        Ok(Self {
            tenant,
            job_id,
            cursor,
            page_size,
        })
    }
    pub fn tenant(&self) -> &AuthenticatedTenant {
        &self.tenant
    }
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityPage {
    entries: Vec<ActivityEntry>,
    next_cursor: Option<String>,
    revision: u64,
}
impl ActivityPage {
    pub fn entries(&self) -> &[ActivityEntry] {
        &self.entries
    }
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// HMAC is used only to derive unguessable registry keys. Custom Debug avoids
/// serializing secret bytes into logs/traces.
#[derive(Clone)]
pub struct ActivityCursorSigner {
    secret: Vec<u8>,
}
impl fmt::Debug for ActivityCursorSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ActivityCursorSigner { secret: [REDACTED] }")
    }
}
impl ActivityCursorSigner {
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self, RunActivityError> {
        let secret = secret.as_ref();
        if secret.is_empty() {
            return Err(RunActivityError::InvalidCursorSigner);
        }
        Ok(Self {
            secret: secret.to_vec(),
        })
    }
    fn token(&self, nonce: u64) -> String {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts arbitrary key material");
        mac.update(b"pulso.run_activity.cursor.v1");
        mac.update(&nonce.to_be_bytes());
        format!("cursor:v1:{}", hex_encode(&mac.finalize().into_bytes()))
    }
}

/// State stored only by a cursor-registry adapter, never serialized into the
/// opaque client token.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CursorRecord {
    tenant_id: String,
    job_id: String,
    snapshot_revision: u64,
    position: ActivityPosition,
}

#[derive(Deserialize)]
struct PersistedCursorRecord {
    tenant_id: String,
    job_id: String,
    snapshot_revision: u64,
    position: ActivityPosition,
}

impl<'de> Deserialize<'de> for CursorRecord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let persisted = PersistedCursorRecord::deserialize(deserializer)?;
        Self::new(
            persisted.tenant_id,
            persisted.job_id,
            persisted.snapshot_revision,
            persisted.position,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl CursorRecord {
    pub fn new(
        tenant_id: String,
        job_id: String,
        snapshot_revision: u64,
        position: ActivityPosition,
    ) -> Result<Self, RunActivityError> {
        if !is_identifier(&tenant_id) || !is_job_id(&job_id) || snapshot_revision == 0 {
            return Err(RunActivityError::InvalidCursor);
        }
        Ok(Self {
            tenant_id,
            job_id,
            snapshot_revision,
            position,
        })
    }
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    pub fn snapshot_revision(&self) -> u64 {
        self.snapshot_revision
    }
    pub fn position(&self) -> &ActivityPosition {
        &self.position
    }
}

/// A process-independent seam. A production implementation must be shared and
/// durable across API workers; the in-memory implementation is local-only.
pub trait CursorRegistry {
    fn issue(&mut self, record: CursorRecord) -> Result<String, RunActivityError>;
    fn lookup(&mut self, token: &str) -> Result<CursorRecord, RunActivityError>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}

#[derive(Debug)]
struct StoredCursor {
    record: CursorRecord,
    last_access: u64,
    expires_at: u64,
}

#[derive(Debug)]
pub struct InMemoryCursorRegistry {
    signer: ActivityCursorSigner,
    capacity: usize,
    ttl_ticks: u64,
    tick: u64,
    nonce: u64,
    cursors: BTreeMap<String, StoredCursor>,
    expired: BTreeMap<String, ()>,
}

impl InMemoryCursorRegistry {
    pub fn new(
        signer: ActivityCursorSigner,
        capacity: usize,
        ttl_ticks: u64,
    ) -> Result<Self, RunActivityError> {
        if capacity == 0 || ttl_ticks == 0 {
            return Err(RunActivityError::InvalidCursorRegistryConfig);
        }
        Ok(Self {
            signer,
            capacity,
            ttl_ticks,
            tick: 0,
            nonce: 0,
            cursors: BTreeMap::new(),
            expired: BTreeMap::new(),
        })
    }
    fn advance(&mut self) -> Result<(), RunActivityError> {
        self.tick = self
            .tick
            .checked_add(1)
            .ok_or(RunActivityError::CursorExhausted)?;
        Ok(())
    }
    fn remember_expired(&mut self, token: String) {
        self.expired.insert(token, ());
        while self.expired.len() > self.capacity {
            let token = self
                .expired
                .keys()
                .next()
                .cloned()
                .expect("nonempty expired registry");
            self.expired.remove(&token);
        }
    }
    fn evict_one(&mut self) {
        let token = self
            .cursors
            .iter()
            .min_by_key(|(_, cursor)| cursor.last_access)
            .map(|(token, _)| token.clone());
        if let Some(token) = token {
            self.cursors.remove(&token);
            self.remember_expired(token);
        }
    }
}

impl CursorRegistry for InMemoryCursorRegistry {
    fn issue(&mut self, record: CursorRecord) -> Result<String, RunActivityError> {
        self.advance()?;
        while self.cursors.len() >= self.capacity {
            self.evict_one();
        }
        self.nonce = self
            .nonce
            .checked_add(1)
            .ok_or(RunActivityError::CursorExhausted)?;
        let token = self.signer.token(self.nonce);
        let expires_at = self
            .tick
            .checked_add(self.ttl_ticks)
            .ok_or(RunActivityError::CursorExhausted)?;
        self.cursors.insert(
            token.clone(),
            StoredCursor {
                record,
                last_access: self.tick,
                expires_at,
            },
        );
        Ok(token)
    }
    fn lookup(&mut self, token: &str) -> Result<CursorRecord, RunActivityError> {
        self.advance()?;
        let Some(cursor) = self.cursors.get_mut(token) else {
            return if self.expired.contains_key(token) {
                Err(RunActivityError::CursorExpired)
            } else {
                Err(RunActivityError::InvalidCursor)
            };
        };
        if self.tick >= cursor.expires_at {
            self.cursors.remove(token);
            self.remember_expired(token.to_owned());
            return Err(RunActivityError::CursorExpired);
        }
        cursor.last_access = self.tick;
        cursor.expires_at = self
            .tick
            .checked_add(self.ttl_ticks)
            .ok_or(RunActivityError::CursorExhausted)?;
        Ok(cursor.record.clone())
    }
    fn len(&self) -> usize {
        self.cursors.len()
    }
    fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }
}

/// Handler owns only short-lived cursor registry state. Querying never mutates
/// jobs or the projection; cursor issuance cannot expose data without a valid
/// authenticated tenant binding.
pub struct RunActivityHandler<'a, ReadModel, Registry = InMemoryCursorRegistry> {
    read_model: &'a ReadModel,
    cursor_registry: Registry,
}
impl<'a, ReadModel: RunActivityReadModel>
    RunActivityHandler<'a, ReadModel, InMemoryCursorRegistry>
{
    pub fn new(read_model: &'a ReadModel, signer: ActivityCursorSigner) -> Self {
        Self::with_cursor_registry(
            read_model,
            InMemoryCursorRegistry::new(signer, 1_024, 1_024)
                .expect("fixed registry configuration is valid"),
        )
    }
}
impl<'a, ReadModel: RunActivityReadModel, Registry: CursorRegistry>
    RunActivityHandler<'a, ReadModel, Registry>
{
    pub fn with_cursor_registry(read_model: &'a ReadModel, cursor_registry: Registry) -> Self {
        Self {
            read_model,
            cursor_registry,
        }
    }
    pub fn list(
        &mut self,
        request: ListRunActivityRequest,
    ) -> Result<ActivityPage, RunActivityError> {
        let cursor = match request.cursor.as_deref() {
            Some(value) => Some(self.cursor_registry.lookup(value)?),
            None => None,
        };
        if let Some(cursor) = &cursor {
            if cursor.tenant_id != request.tenant.0 || cursor.job_id != request.job_id {
                return Err(RunActivityError::InvalidCursor);
            }
        }
        let query = ActivityReadQuery::new(
            request.tenant.clone(),
            request.job_id.clone(),
            cursor.as_ref().map(|c| c.position.clone()),
            request.page_size,
        )?;
        let timeline = self.read_model.read_activity(&query)?;
        if timeline.entries().len() > request.page_size {
            return Err(RunActivityError::InvalidReadModelResponse);
        }
        if let Some(cursor) = &cursor {
            if timeline
                .entries()
                .first()
                .is_some_and(|entry| entry.position() <= cursor.position)
            {
                return Err(RunActivityError::InvalidReadModelResponse);
            }
            if cursor.snapshot_revision <= timeline.purged_through_revision() {
                return Err(RunActivityError::CursorPurged);
            }
            if cursor.snapshot_revision != timeline.revision() {
                return Err(RunActivityError::CursorExpired);
            }
        }
        let next_cursor = if timeline.has_more() {
            let position = timeline
                .entries()
                .last()
                .expect("nonempty bounded page has a last entry")
                .position();
            Some(self.cursor_registry.issue(CursorRecord::new(
                request.tenant.0.clone(),
                request.job_id.clone(),
                timeline.revision(),
                position,
            )?)?)
        } else {
            None
        };
        Ok(ActivityPage {
            entries: timeline.entries,
            next_cursor,
            revision: timeline.revision,
        })
    }
    pub fn cursor_count(&self) -> usize {
        self.cursor_registry.len()
    }
}

/// Concrete, transport-neutral stream adapter. A web server may serialize
/// batches as SSE, while tests and local callers can resume by opaque cursor.
pub trait RunActivityStreamPort {
    fn next_batch(
        &mut self,
        request: StreamRunActivityRequest,
    ) -> Result<ActivityStreamBatch, RunActivityError>;
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamRunActivityRequest {
    pub activity: ListRunActivityRequest,
}
impl StreamRunActivityRequest {
    pub fn new(activity: ListRunActivityRequest) -> Self {
        Self { activity }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityStreamBatch {
    pub event: &'static str,
    pub page: ActivityPage,
}
pub struct ProjectionActivityStream<'a, ReadModel> {
    handler: RunActivityHandler<'a, ReadModel>,
}
impl<'a, ReadModel: RunActivityReadModel> ProjectionActivityStream<'a, ReadModel> {
    pub fn new(handler: RunActivityHandler<'a, ReadModel>) -> Self {
        Self { handler }
    }
}
impl<'a, ReadModel: RunActivityReadModel> RunActivityStreamPort
    for ProjectionActivityStream<'a, ReadModel>
{
    fn next_batch(
        &mut self,
        request: StreamRunActivityRequest,
    ) -> Result<ActivityStreamBatch, RunActivityError> {
        Ok(ActivityStreamBatch {
            event: "run.activity",
            page: self.handler.list(request.activity)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityApiStatus {
    BadRequest,
    NotFound,
    Conflict,
    Gone,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunActivityError {
    InvalidTenantBinding,
    InvalidEvent,
    EventConflict,
    RevisionExhausted,
    RunNotFound,
    InvalidPageSize,
    InvalidReadModelResponse,
    InvalidCursorSigner,
    InvalidCursorRegistryConfig,
    InvalidCursor,
    InvalidPurgeRevision,
    CursorExpired,
    CursorPurged,
    CursorExhausted,
}
impl RunActivityError {
    pub fn status(&self) -> ActivityApiStatus {
        match self {
            Self::RunNotFound => ActivityApiStatus::NotFound,
            Self::CursorExpired | Self::CursorPurged => ActivityApiStatus::Gone,
            Self::EventConflict | Self::RevisionExhausted | Self::CursorExhausted => {
                ActivityApiStatus::Conflict
            }
            _ => ActivityApiStatus::BadRequest,
        }
    }
}
impl fmt::Display for RunActivityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl std::error::Error for RunActivityError {}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}
fn is_job_id(value: &str) -> bool {
    value.len() == 75
        && value.starts_with("job:sha256:")
        && value.as_bytes()[11..]
            .iter()
            .all(|b| matches!(*b, b'0'..=b'9' | b'a'..=b'f'))
}
fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|b| matches!(*b, b'0'..=b'9' | b'a'..=b'f'))
}
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
