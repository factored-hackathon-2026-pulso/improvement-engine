//! U23-P temporal boundary for governed published-memory use.
//!
//! This is deliberately only the temporal protocol, not the U23 E0 runner.
//! It receives no wiki bytes, does not publish a revision, and cannot score or
//! release a candidate. U22/U33 remain the authority for a governed receipt.

use crate::ArtifactRepository;
use crate::enriched_history::VerifiedReplayAvailability;
use crate::governed_memory_use::{
    MemoryUseAdmission, MemoryUseAdmissionError, MemoryUseRequest, VerifiedMemoryUse,
};
use crate::memory_store::{AtomicMemoryUseCommitPort, MemoryScope};
use postgres::Client;
use sha2::{Digest, Sha256};

/// Derive a replay-stable U06 successor identity from the immutable source
/// event. The timestamp portion is UUIDv7-compatible; the remaining bits are
/// domain-separated SHA-256 output, so retries recover the same run id.
#[allow(dead_code)] // Consumed by the P4 service composition when that caller is wired.
pub(crate) fn successor_run_id(
    tenant_id: &str,
    source_event_ref: &str,
    occurred_at_unix_ms: u64,
) -> Result<String, TemporalProtocolError> {
    if tenant_id.trim().is_empty()
        || source_event_ref.trim().is_empty()
        || occurred_at_unix_ms >= (1_u64 << 48)
    {
        return Err(TemporalProtocolError::InvalidSuccessorEventIdentity);
    }

    let mut digest = Sha256::new();
    digest.update(b"pulso.temporal-successor.uuidv7.v1\0");
    digest.update((tenant_id.len() as u64).to_be_bytes());
    digest.update(tenant_id.as_bytes());
    digest.update((source_event_ref.len() as u64).to_be_bytes());
    digest.update(source_event_ref.as_bytes());
    digest.update(occurred_at_unix_ms.to_be_bytes());
    let hash = digest.finalize();

    let mut bytes = [0_u8; 16];
    let timestamp = occurred_at_unix_ms.to_be_bytes();
    bytes[..6].copy_from_slice(&timestamp[2..]);
    bytes[6..].copy_from_slice(&hash[..10]);
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

#[allow(dead_code)] // Validates the same identity at the trusted composition boundary.
pub(crate) fn is_uuid_v7(value: &str) -> bool {
    let bytes: Vec<_> = value.bytes().collect();
    bytes.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
        && bytes[14] == b'7'
        && matches!(bytes[19].to_ascii_lowercase(), b'8'..=b'b')
}

/// The replay cutoff already established by the U04-B availability boundary.
///
/// The caller obtains this only after U04-B has validated its sealed source
/// snapshot and availability projection. This small contract never reopens
/// source files or interprets outcome data.
/// Opaque evidence emitted by the U04-B availability boundary (or its future
/// replay-runner adapter). It binds the full governed-use context to an
/// exact U04-B snapshot/profile commitments and is deliberately neither
/// constructible nor inspectable by callers.
pub struct TemporalMemoryEvidence {
    commitment: String,
    protocol: MemoryTemporalProtocol,
    cutoff_at_unix_seconds: u64,
    memory_use_at_unix_seconds: u64,
    outcome_available_at_unix_seconds: Option<u64>,
    outcome_provenance: Option<String>,
    scope: MemoryScope,
    snapshot_ref: crate::ArtifactReference,
    run_id: String,
    grant_id: String,
    grant_revision: u64,
    purpose: String,
}

/// Opaque U04-B availability projection. Its only production factory is the
/// verified U04-B adapter; it intentionally has no public constructor or raw
/// timestamp getters.
#[allow(dead_code)] // Invoked by the future trusted service composition root.
pub(crate) struct VerifiedAvailabilityProjection {
    request: MemoryUseRequest,
    cutoff_at_unix_seconds: u64,
    outcome_available_at_unix_seconds: Option<u64>,
    outcome_provenance: Option<String>,
    source_snapshot_digest: String,
    availability_profile_digest: String,
}

impl VerifiedAvailabilityProjection {
    /// The production U04-B composition path. It accepts only the opaque
    /// projection emitted after a V2 adapter revalidates the exact parsed
    /// snapshot and availability profile. No transport caller can construct
    /// either value or choose a separate replay clock.
    #[allow(dead_code)] // Invoked by the future trusted service composition root.
    pub(crate) fn from_u04b_replay(
        replay: VerifiedReplayAvailability,
        request: MemoryUseRequest,
    ) -> Result<Self, TemporalProtocolError> {
        if replay.tenant_id() != request.scope().tenant_id
            || replay.world_ref() != request.scope().world
        {
            return Err(TemporalProtocolError::U04BReplayScopeMismatch);
        }
        Ok(Self {
            cutoff_at_unix_seconds: replay.cutoff_at_unix_seconds(),
            source_snapshot_digest: replay.source_snapshot_digest().to_owned(),
            availability_profile_digest: replay.availability_profile_digest().to_owned(),
            request,
            outcome_available_at_unix_seconds: None,
            outcome_provenance: None,
        })
    }

    #[cfg(test)]
    fn deterministic(
        request: MemoryUseRequest,
        cutoff_at_unix_seconds: u64,
        outcome_available_at_unix_seconds: Option<u64>,
        outcome_provenance: Option<String>,
    ) -> Self {
        Self {
            request,
            cutoff_at_unix_seconds,
            outcome_available_at_unix_seconds,
            outcome_provenance,
            source_snapshot_digest: "sha256:test_source_snapshot".to_owned(),
            availability_profile_digest: "sha256:test_availability_profile".to_owned(),
        }
    }
}

/// Crate-private stand-in for the U04-B/future runner issuer. The service
/// composition owns it; consumers only receive opaque evidence.
#[allow(dead_code)] // Called by the future U04-B/replay composition root.
pub(crate) struct TrustedTemporalEvidenceIssuer {
    projection: VerifiedAvailabilityProjection,
}

impl TrustedTemporalEvidenceIssuer {
    /// Composition-only factory for a U04-B replay projection. It is
    /// crate-private: transport callers cannot select a snapshot/profile,
    /// tenant/world or cutoff and therefore cannot mint temporal evidence.
    /// This source has no sealed outcome adapter yet (U20-E/U27), so it can
    /// only result in an admitted `Frozen` use; `Continuous` evidence is
    /// rejected for missing outcome evidence before U33 observes a receipt.
    #[allow(dead_code)] // Invoked by the future trusted service composition root.
    pub(crate) fn from_u04b_replay(
        replay: VerifiedReplayAvailability,
        request: MemoryUseRequest,
    ) -> Result<Self, TemporalProtocolError> {
        let projection = VerifiedAvailabilityProjection::from_u04b_replay(replay, request)?;
        Ok(Self { projection })
    }

    #[cfg(test)]
    fn deterministic(projection: VerifiedAvailabilityProjection) -> Self {
        Self { projection }
    }

    #[allow(dead_code)]
    pub(crate) fn attest(&self, protocol: MemoryTemporalProtocol) -> TemporalMemoryEvidence {
        let request = &self.projection.request;
        let commitment = temporal_commitment(&TemporalCommitmentInput {
            protocol,
            scope: request.scope(),
            access: request.access(),
            allowed_at: request.allowed_at_unix_seconds(),
            cutoff: self.projection.cutoff_at_unix_seconds,
            outcome_at: self.projection.outcome_available_at_unix_seconds,
            outcome_provenance: self.projection.outcome_provenance.as_deref(),
            source_snapshot_digest: &self.projection.source_snapshot_digest,
            availability_profile_digest: &self.projection.availability_profile_digest,
        });
        TemporalMemoryEvidence {
            commitment,
            protocol,
            cutoff_at_unix_seconds: self.projection.cutoff_at_unix_seconds,
            memory_use_at_unix_seconds: request.allowed_at_unix_seconds(),
            outcome_available_at_unix_seconds: self.projection.outcome_available_at_unix_seconds,
            outcome_provenance: self.projection.outcome_provenance.clone(),
            scope: request.scope().clone(),
            snapshot_ref: request.access().snapshot_ref.clone(),
            run_id: request.access().run_id.clone(),
            grant_id: request.access().grant_id.clone(),
            grant_revision: request.access().grant_revision,
            purpose: request.access().purpose.clone(),
        }
    }
}

/// The two allowed temporal semantics for a partitioned memory head.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryTemporalProtocol {
    Frozen,
    Continuous,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemporalProtocolError {
    ProtocolMismatch {
        expected: &'static str,
        actual: String,
    },
    MemoryAfterReplayCutoff,
    AccessTimeMismatch,
    GrantRevisionMismatch,
    OutcomeForbiddenInFrozen,
    OutcomeRequired,
    OutcomeAfterMemoryUse,
    OutcomeAfterReplayCutoff,
    U04BReplayScopeMismatch,
    InvalidSuccessorEventIdentity,
}

/// Exact authorization context the U05 authority must hold for the full
/// receipt+outbox transaction. A provider must reject absent, expired,
/// revoked, or scope-mismatched grants before invoking the callback, and its
/// lease must make revocation wait until the callback (including commit) ends.
#[allow(dead_code)] // Implemented by the service-owned U05 authority adapter.
pub(crate) struct MemoryGrantLeaseClaims<'a> {
    tenant_id: &'a str,
    grant_id: &'a str,
    grant_revision: u64,
    purpose: &'a str,
    scope: &'a MemoryScope,
    allowed_at_unix_seconds: u64,
}

/// U05 integration port. The production authority is intentionally not
/// fabricated in this crate yet; the missing-provider implementation fails
/// closed. Local fixtures may implement this only in tests.
#[allow(dead_code)] // Injected by the trusted service composition root.
pub(crate) trait RevocationSafeMemoryGrantAuthority {
    fn with_valid_lease<T, F>(
        &self,
        claims: &MemoryGrantLeaseClaims<'_>,
        operation: F,
    ) -> Result<T, TemporalSuccessorError>
    where
        F: FnOnce() -> Result<T, TemporalSuccessorError>;
}

#[allow(dead_code)] // Used until the service composition supplies its U05 adapter.
pub(crate) struct MissingMemoryGrantAuthority;

#[allow(dead_code)]
impl RevocationSafeMemoryGrantAuthority for MissingMemoryGrantAuthority {
    fn with_valid_lease<T, F>(
        &self,
        _claims: &MemoryGrantLeaseClaims<'_>,
        _operation: F,
    ) -> Result<T, TemporalSuccessorError>
    where
        F: FnOnce() -> Result<T, TemporalSuccessorError>,
    {
        Err(TemporalSuccessorError::DependencyBlocked)
    }
}

#[derive(Debug)]
#[allow(dead_code)] // Returned by the not-yet-wired service composition seam.
pub(crate) enum TemporalSuccessorError {
    DependencyBlocked,
    GrantDenied,
    InvalidRequest,
    Temporal(TemporalProtocolError),
    Conflict(postgres::Error),
    ReconciliationRequired(postgres::Error),
    Storage(postgres::Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TemporalDatabaseOutcome {
    Conflict,
    ReconciliationRequired,
    Storage,
}

fn classify_temporal_successor_sqlstate(
    sqlstate: &postgres::error::SqlState,
) -> TemporalDatabaseOutcome {
    if sqlstate == &postgres::error::SqlState::UNIQUE_VIOLATION {
        TemporalDatabaseOutcome::Conflict
    } else if sqlstate == &postgres::error::SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE {
        TemporalDatabaseOutcome::ReconciliationRequired
    } else {
        TemporalDatabaseOutcome::Storage
    }
}

fn map_temporal_successor_storage_error(error: postgres::Error) -> TemporalSuccessorError {
    match error
        .code()
        .map(classify_temporal_successor_sqlstate)
        .unwrap_or(TemporalDatabaseOutcome::Storage)
    {
        TemporalDatabaseOutcome::Conflict => TemporalSuccessorError::Conflict(error),
        TemporalDatabaseOutcome::ReconciliationRequired => {
            TemporalSuccessorError::ReconciliationRequired(error)
        }
        TemporalDatabaseOutcome::Storage => TemporalSuccessorError::Storage(error),
    }
}

#[allow(dead_code)]
impl std::fmt::Display for TemporalSuccessorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DependencyBlocked => formatter.write_str("U05 grant authority is not configured"),
            Self::GrantDenied => {
                formatter.write_str("U05 grant is expired, revoked, or out of scope")
            }
            Self::InvalidRequest => formatter.write_str("temporal successor request is invalid"),
            Self::Temporal(error) => write!(formatter, "temporal admission failed: {error:?}"),
            Self::Conflict(_) => {
                formatter.write_str("temporal successor request conflicts with persisted state")
            }
            Self::ReconciliationRequired(_) => {
                formatter.write_str("temporal successor state requires reconciliation")
            }
            Self::Storage(_) => formatter.write_str("temporal successor persistence failed"),
        }
    }
}

#[allow(dead_code)]
impl std::error::Error for TemporalSuccessorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Conflict(error) | Self::ReconciliationRequired(error) | Self::Storage(error) => {
                Some(error)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Returned to the future U06 enqueue adapter as requested-only state.
pub(crate) struct TemporalSuccessorRequestReceipt {
    pub(crate) memory_receipt_digest: String,
    pub(crate) successor_run_id: String,
    /// `requested` records durable outbox creation, not U06 admission or run.
    pub(crate) status: &'static str,
}

/// Immutable trigger identity and optimistic head precondition for one
/// successor request. Grouping these fields keeps the trusted writer API
/// aligned with the event/outbox contract.
#[allow(dead_code)] // Constructed by the trusted P4 event composition caller.
pub(crate) struct TemporalSuccessorTrigger {
    pub(crate) expected_head_version: u64,
    pub(crate) source_event_ref: String,
    pub(crate) event_occurred_at_unix_ms: u64,
}

/// Atomically composes U05 lease, U23 temporal evidence, U33 use receipt, and
/// one durable U06 successor request. U06 later decides whether it admits or
/// executes the queued request. The authority callback borrows the revocation
/// fence across the database commit; this function never converts a missing
/// provider or denied grant into an allowed receipt.
#[allow(dead_code)] // The executable service composition caller is a separate slice.
pub(crate) fn record_temporal_successor<A: RevocationSafeMemoryGrantAuthority>(
    client: &mut Client,
    authority: &A,
    protocol: MemoryTemporalProtocol,
    evidence: TemporalMemoryEvidence,
    request: MemoryUseRequest,
    trigger: TemporalSuccessorTrigger,
) -> Result<TemporalSuccessorRequestReceipt, TemporalSuccessorError> {
    validate_successor_inputs(protocol, &evidence, &request, &trigger)?;

    let access = request.access();
    let claims = MemoryGrantLeaseClaims {
        tenant_id: &request.scope().tenant_id,
        grant_id: &access.grant_id,
        grant_revision: access.grant_revision,
        purpose: &request.scope().purpose,
        scope: request.scope(),
        allowed_at_unix_seconds: access.allowed_at_unix_seconds,
    };
    authority.with_valid_lease(&claims, || {
        let temporal_commitment = evidence.commitment.clone();
        let receipt_digest = crate::memory_store::receipt_id(
            request.scope(),
            access,
            &access.snapshot_ref,
            trigger.expected_head_version,
            Some(&temporal_commitment),
        );
        let request_digest = successor_request_digest(
            &request,
            &evidence,
            &receipt_digest,
            &trigger.source_event_ref,
            trigger.event_occurred_at_unix_ms,
            trigger.expected_head_version,
        );
        let successor_id = successor_run_id(
            &request.scope().tenant_id,
            &trigger.source_event_ref,
            trigger.event_occurred_at_unix_ms,
        )
        .map_err(TemporalSuccessorError::Temporal)?;
        if !is_uuid_v7(&successor_id) {
            return Err(TemporalSuccessorError::InvalidRequest);
        }

        let mut transaction = client
            .transaction()
            .map_err(map_temporal_successor_storage_error)?;
        transaction
            .query_one(
                "SELECT * FROM pulso_record_temporal_memory_successor($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::uuid,$11,$12,$13,$14,$15,$16,$17,$18::text::uuid,$19,$20)",
                &[
                    &request.scope().tenant_id,
                    &receipt_digest,
                    &temporal_commitment,
                    &trigger.source_event_ref,
                    &request.scope().purpose,
                    &request.scope().world,
                    &request.scope().campaign,
                    &request.scope().protocol,
                    &request.scope().partition,
                    &access.snapshot_ref.id,
                    &(access.snapshot_ref.revision as i64),
                    &access.snapshot_ref.digest,
                    &(trigger.expected_head_version as i64),
                    &access.run_id,
                    &access.grant_id,
                    &(access.allowed_at_unix_seconds as i64),
                    &(evidence.cutoff_at_unix_seconds as i64),
                    &successor_id,
                    &request_digest,
                    &(trigger.event_occurred_at_unix_ms as i64),
                ],
            )
            .map_err(map_temporal_successor_storage_error)?;
        transaction
            .commit()
            .map_err(map_temporal_successor_storage_error)?;
        Ok(TemporalSuccessorRequestReceipt {
            memory_receipt_digest: receipt_digest,
            successor_run_id: successor_id,
            status: "requested",
        })
    })
}

#[allow(dead_code)] // Used only by the crate-private P4 successor composition.
fn validate_successor_inputs(
    protocol: MemoryTemporalProtocol,
    evidence: &TemporalMemoryEvidence,
    request: &MemoryUseRequest,
    trigger: &TemporalSuccessorTrigger,
) -> Result<(), TemporalSuccessorError> {
    let access = request.access();
    if !is_opaque_source_event_ref(&trigger.source_event_ref)
        || trigger.expected_head_version == 0
        || trigger.expected_head_version > i64::MAX as u64
        || access.snapshot_ref.revision > i64::MAX as u64
        || access.allowed_at_unix_seconds > i64::MAX as u64
        || evidence.cutoff_at_unix_seconds > i64::MAX as u64
        || trigger.event_occurred_at_unix_ms >= (1_u64 << 48)
        || access.tenant_id != request.scope().tenant_id
        || access.memory_scope.world != request.scope().world
        || access.memory_scope.campaign != request.scope().campaign
        || access.memory_scope.protocol != request.scope().protocol
        || access.memory_scope.partition != request.scope().partition
        || evidence.memory_use_at_unix_seconds != access.allowed_at_unix_seconds
        || evidence.scope != *request.scope()
        || evidence.snapshot_ref != access.snapshot_ref
        || evidence.run_id != access.run_id
        || evidence.grant_id != access.grant_id
        || evidence.grant_revision != access.grant_revision
        || evidence.purpose != access.purpose
    {
        return Err(TemporalSuccessorError::InvalidRequest);
    }
    if evidence.protocol != protocol {
        return Err(TemporalSuccessorError::Temporal(
            TemporalProtocolError::ProtocolMismatch {
                expected: protocol.as_str(),
                actual: evidence.protocol.as_str().to_owned(),
            },
        ));
    }
    protocol
        .validate_evidence(request.scope(), evidence)
        .map_err(TemporalSuccessorError::Temporal)
}

/// Source-event references cross a persistence boundary and therefore accept
/// only the engine's opaque SHA-256 token shape, never source IDs or free text.
fn is_opaque_source_event_ref(value: &str) -> bool {
    let Some(digest) = value.strip_prefix("sha256_") else {
        return false;
    };
    digest.len() == 56
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[allow(dead_code)] // Used only by the crate-private P4 successor composition.
fn successor_request_digest(
    request: &MemoryUseRequest,
    evidence: &TemporalMemoryEvidence,
    receipt_digest: &str,
    source_event_ref: &str,
    event_occurred_at_unix_ms: u64,
    expected_head_version: u64,
) -> String {
    let access = request.access();
    let bytes = serde_json::to_vec(&(
        "pulso.temporal_successor.request.v1",
        (
            receipt_digest,
            request.scope(),
            &access.tenant_id,
            &access.run_id,
            &access.purpose,
            &access.grant_id,
            access.grant_revision,
            &access.snapshot_ref,
            access.allowed_at_unix_seconds,
            &access.memory_scope,
            expected_head_version,
            source_event_ref,
            event_occurred_at_unix_ms,
        ),
        (
            &evidence.commitment,
            evidence.protocol.as_str(),
            evidence.cutoff_at_unix_seconds,
            evidence.memory_use_at_unix_seconds,
            evidence.outcome_available_at_unix_seconds,
            &evidence.outcome_provenance,
        ),
    ))
    .expect("temporal successor identity inputs serialize deterministically");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemporalMemoryAdmissionError {
    Temporal(TemporalProtocolError),
    Governed(MemoryUseAdmissionError),
}

/// Trusted composition entry point for the U23-P temporal gate.
///
/// It deliberately runs the pure temporal check before U22 records a receipt,
/// so a not-yet-observable outcome cannot create an audit/use side effect. The
/// underlying U22 call remains crate-private: an external caller cannot supply
/// a permissive publisher or receive a use capability from this boundary.
///
/// ```compile_fail
/// use improvement_engine_core::memory_temporal_protocol::MemoryTemporalAdmission;
/// let _ = MemoryTemporalAdmission::admit;
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::memory_temporal_protocol::MemoryTemporalAdmission;
/// let _ = MemoryTemporalAdmission { _private: true };
/// ```
pub struct MemoryTemporalAdmission {
    _private: bool,
}

impl MemoryTemporalAdmission {
    #[allow(dead_code)] // Invoked by the future trusted service composition root.
    pub(crate) fn admit<R: ArtifactRepository, P: AtomicMemoryUseCommitPort>(
        protocol: MemoryTemporalProtocol,
        evidence: TemporalMemoryEvidence,
        publisher: &mut P,
        artifacts: &mut R,
        request: MemoryUseRequest,
    ) -> Result<VerifiedMemoryUse, TemporalMemoryAdmissionError> {
        if evidence.memory_use_at_unix_seconds != request.allowed_at_unix_seconds() {
            return Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch,
            ));
        }
        if evidence.scope != *request.scope()
            || evidence.snapshot_ref != request.access().snapshot_ref
            || evidence.run_id != request.access().run_id
            || evidence.grant_id != request.access().grant_id
            || evidence.purpose != request.access().purpose
        {
            return Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch,
            ));
        }
        if evidence.grant_revision != request.access().grant_revision {
            return Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::GrantRevisionMismatch,
            ));
        }
        if evidence.protocol != protocol {
            return Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::ProtocolMismatch {
                    expected: protocol.as_str(),
                    actual: evidence.protocol.as_str().to_owned(),
                },
            ));
        }
        protocol
            .validate_evidence(request.scope(), &evidence)
            .map_err(TemporalMemoryAdmissionError::Temporal)?;
        let request = request.with_temporal_commitment(evidence.commitment.clone());
        let admitted = MemoryUseAdmission::admit(publisher, artifacts, request)
            .map_err(TemporalMemoryAdmissionError::Governed)?;
        // The atomic U33 commit recomputes canonical receipt identity,
        // liveness, authorization and this exact commitment before exposing a
        // receipt; this comparison is only a defensive output check.
        if admitted.receipt().temporal_commitment.as_deref() != Some(evidence.commitment.as_str()) {
            return Err(TemporalMemoryAdmissionError::Governed(
                MemoryUseAdmissionError::ReceiptMismatch,
            ));
        }
        Ok(admitted)
    }
}

impl MemoryTemporalProtocol {
    /// Validates only temporal eligibility. Receipt/authentication/liveness are
    /// deliberately revalidated by the U22/U33 composition boundary.
    fn as_str(self) -> &'static str {
        match self {
            Self::Frozen => "frozen",
            Self::Continuous => "continuous",
        }
    }

    fn validate_evidence(
        self,
        scope: &MemoryScope,
        evidence: &TemporalMemoryEvidence,
    ) -> Result<(), TemporalProtocolError> {
        let expected = match self {
            Self::Frozen => "frozen",
            Self::Continuous => "continuous",
        };
        if scope.protocol != expected {
            return Err(TemporalProtocolError::ProtocolMismatch {
                expected,
                actual: scope.protocol.clone(),
            });
        }
        if evidence.memory_use_at_unix_seconds > evidence.cutoff_at_unix_seconds {
            return Err(TemporalProtocolError::MemoryAfterReplayCutoff);
        }
        match self {
            Self::Frozen
                if evidence.outcome_available_at_unix_seconds.is_some()
                    || evidence.outcome_provenance.is_some() =>
            {
                Err(TemporalProtocolError::OutcomeForbiddenInFrozen)
            }
            Self::Frozen => Ok(()),
            Self::Continuous => {
                let outcome = evidence
                    .outcome_available_at_unix_seconds
                    .ok_or(TemporalProtocolError::OutcomeRequired)?;
                if evidence
                    .outcome_provenance
                    .as_deref()
                    .is_none_or(str::is_empty)
                {
                    return Err(TemporalProtocolError::OutcomeRequired);
                }
                if outcome > evidence.memory_use_at_unix_seconds {
                    return Err(TemporalProtocolError::OutcomeAfterMemoryUse);
                }
                if outcome > evidence.cutoff_at_unix_seconds {
                    return Err(TemporalProtocolError::OutcomeAfterReplayCutoff);
                }
                Ok(())
            }
        }
    }
}

/// U23-E uses the same U04-B trusted clock as U23-P, but carries the result to
/// the E0-specific U33-E sidecar instead of the generic U22 memory ledger.
/// This helper emits only the temporal commitment after the pure Frozen gate;
/// U33-E still rechecks exact publication, head, liveness and grant at commit.
#[allow(dead_code)] // Used by the trusted U23-E runtime composition once wired.
pub(crate) fn attest_frozen_e0_reuse(
    replay: &VerifiedReplayAvailability,
    scope: &MemoryScope,
    access: &crate::wiki_scratch::WikiAccess,
) -> Result<String, TemporalProtocolError> {
    if replay.tenant_id() != scope.tenant_id
        || replay.world_ref() != scope.world
        || access.tenant_id != scope.tenant_id
        || access.purpose != scope.purpose
        || access.memory_scope.world != scope.world
        || access.memory_scope.campaign != scope.campaign
        || access.memory_scope.protocol != scope.protocol
        || access.memory_scope.partition != scope.partition
    {
        return Err(TemporalProtocolError::U04BReplayScopeMismatch);
    }
    let request = MemoryUseRequest::new(scope.clone(), access.clone());
    let projection = VerifiedAvailabilityProjection {
        cutoff_at_unix_seconds: replay.cutoff_at_unix_seconds(),
        source_snapshot_digest: replay.source_snapshot_digest().to_owned(),
        availability_profile_digest: replay.availability_profile_digest().to_owned(),
        request,
        outcome_available_at_unix_seconds: None,
        outcome_provenance: None,
    };
    let evidence =
        TrustedTemporalEvidenceIssuer { projection }.attest(MemoryTemporalProtocol::Frozen);
    MemoryTemporalProtocol::Frozen.validate_evidence(scope, &evidence)?;
    Ok(evidence.commitment)
}

#[allow(dead_code)]
struct TemporalCommitmentInput<'a> {
    protocol: MemoryTemporalProtocol,
    scope: &'a MemoryScope,
    access: &'a crate::wiki_scratch::WikiAccess,
    allowed_at: u64,
    cutoff: u64,
    outcome_at: Option<u64>,
    outcome_provenance: Option<&'a str>,
    source_snapshot_digest: &'a str,
    availability_profile_digest: &'a str,
}

#[allow(dead_code)]
fn temporal_commitment(input: &TemporalCommitmentInput<'_>) -> String {
    let bytes = serde_json::to_vec(&(
        input.protocol.as_str(),
        input.scope,
        &input.access.snapshot_ref,
        &input.access.run_id,
        &input.access.grant_id,
        input.access.grant_revision,
        &input.access.purpose,
        input.allowed_at,
        input.cutoff,
        input.outcome_at,
        input.outcome_provenance,
        input.source_snapshot_digest,
        input.availability_profile_digest,
    ))
    .expect("temporal evidence inputs serialize deterministically");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Barrier, Mutex};

    use super::{
        MemoryGrantLeaseClaims, MemoryTemporalAdmission, MemoryTemporalProtocol,
        MissingMemoryGrantAuthority, RevocationSafeMemoryGrantAuthority,
        TemporalMemoryAdmissionError, TemporalProtocolError, TemporalSuccessorError,
        TemporalSuccessorTrigger, TrustedTemporalEvidenceIssuer, VerifiedAvailabilityProjection,
        is_uuid_v7, record_temporal_successor, successor_run_id,
    };
    use crate::enriched_history::{
        AvailabilityClockMode, AvailabilityProfile, EnrichedHistoryAdapter,
        EnrichedHistoryManifest, PackageFile, ProvenanceDigests,
    };
    use crate::governed_memory_use::MemoryUseRequest;
    use crate::memory_store::{
        InMemoryGovernedMemoryCommitPort, InMemoryMemoryRegistry, MemoryPublisher, MemoryScope,
    };
    use crate::source_validation::SourceSnapshot;
    use crate::wiki_scratch::{
        InMemoryWikiGrantAuthority, MemoryScopeBinding, WikiAccess, WikiGrant,
    };
    use crate::{ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository};
    use serde_json::json;

    const TENANT: &str = "tenant-a";
    const WIKI_ID: &str = "018f50a1-7f00-7000-8000-000000000023";

    #[derive(Clone, Copy)]
    enum GrantFixtureState {
        Valid,
        Revoked,
        Expired,
        ScopeMismatch,
    }

    struct LocalGrantLeaseFixture {
        state: Mutex<GrantFixtureState>,
        lease_guard: Mutex<()>,
        hold: Option<(Arc<Barrier>, Arc<Barrier>)>,
    }

    impl LocalGrantLeaseFixture {
        fn new(state: GrantFixtureState) -> Self {
            Self {
                state: Mutex::new(state),
                lease_guard: Mutex::new(()),
                hold: None,
            }
        }

        fn held_valid_lease(entered: Arc<Barrier>, release: Arc<Barrier>) -> Self {
            Self {
                state: Mutex::new(GrantFixtureState::Valid),
                lease_guard: Mutex::new(()),
                hold: Some((entered, release)),
            }
        }

        fn try_begin_revoke(&self) -> bool {
            match self.lease_guard.try_lock() {
                Ok(guard) => {
                    *self.state.lock().expect("fixture grant state") = GrantFixtureState::Revoked;
                    drop(guard);
                    false
                }
                Err(std::sync::TryLockError::WouldBlock) => true,
                Err(std::sync::TryLockError::Poisoned(error)) => {
                    drop(error.into_inner());
                    panic!("fixture lease lock poisoned");
                }
            }
        }

        fn revoke_after_lease(&self) {
            let _guard = self.lease_guard.lock().expect("fixture lease lock");
            *self.state.lock().expect("fixture grant state") = GrantFixtureState::Revoked;
        }
    }

    impl RevocationSafeMemoryGrantAuthority for LocalGrantLeaseFixture {
        fn with_valid_lease<T, F>(
            &self,
            claims: &MemoryGrantLeaseClaims<'_>,
            operation: F,
        ) -> Result<T, TemporalSuccessorError>
        where
            F: FnOnce() -> Result<T, TemporalSuccessorError>,
        {
            // Holding this guard across the callback models the U05 contract:
            // a concurrent revoker cannot invalidate the admission before DB
            // commit. This fixture is test-only, not a production authority.
            let _guard = self.lease_guard.lock().expect("fixture lease lock");
            let state = *self.state.lock().expect("fixture grant state");
            let expected_tenant = TENANT;
            let expected_scope = scope("frozen");
            if !matches!(state, GrantFixtureState::Valid)
                || claims.tenant_id != expected_tenant
                || claims.grant_id != "grant-p4"
                || claims.grant_revision != 1
                || claims.purpose != "investigation"
                || claims.scope != &expected_scope
                || claims.allowed_at_unix_seconds != 150
            {
                return Err(TemporalSuccessorError::GrantDenied);
            }
            if let Some((entered, release)) = &self.hold {
                entered.wait();
                release.wait();
            }
            operation()
        }
    }

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn opaque_event_ref(byte: char) -> String {
        format!("sha256_{}", byte.to_string().repeat(56))
    }

    fn u04b_replay_projection() -> crate::enriched_history::VerifiedReplayAvailability {
        u04b_replay_projection_for(TENANT, "")
    }

    fn u04b_replay_projection_for(
        tenant_id: &str,
        raw_prefix: &str,
    ) -> crate::enriched_history::VerifiedReplayAvailability {
        let snapshot = SourceSnapshot::from_json(&format!(
            "{raw_prefix}{}",
            json!({
                "contract_version": {"major": 1, "minor": 0},
                "tenant_id": tenant_id,
                "source_namespace": "platform_history",
                "world_ref": "world-a",
                "observed_cutoff": "1970-01-01T00:01:40Z",
                "sources": [{
                    "table": "case",
                    "uri": "file://fixture.csv",
                    "file_digest": digest('a'),
                    "header_digest": digest('b'),
                    "row_count": 1,
                    "source_contract_ref": {"id": "case", "version": "v1", "digest": digest('c')}
                }]
            })
        ))
        .expect("fixed U04-B source snapshot");
        let profile = AvailabilityProfile::new(
            "e0_replay",
            1,
            AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed"),
            tenant_id,
            snapshot.binding_digest(),
        );
        let manifest = EnrichedHistoryManifest::new_replay(
            "platform_history",
            "world-a",
            "1970-01-01T00:01:40Z",
            profile,
            vec![
                PackageFile::new(
                    "case",
                    ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                    "1970-01-01T00:01:40Z",
                )
                .with_field_availability(BTreeMap::from([(
                    "event_time".to_owned(),
                    "1970-01-01T00:01:40Z".to_owned(),
                )]))
                .with_replay_projection_digest(digest('e'))
                .with_source_file_seal(
                    snapshot
                        .source_file_seal("case")
                        .expect("fixed snapshot source seal"),
                ),
            ],
        );
        EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot)
            .expect("fixed replay package bound to its snapshot")
            .verified_replay_availability(&snapshot)
            .expect("U04-B replay projection")
    }

    fn scope(protocol: &str) -> MemoryScope {
        MemoryScope::new(
            TENANT,
            "investigation",
            "world-a",
            "campaign-a",
            protocol,
            "train",
        )
    }

    #[test]
    #[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
    fn temporal_successor_requires_u05_lease_and_commits_receipt_job_and_event_together() {
        assert_eq!(
            std::env::var("PULSO_ALLOW_DESTRUCTIVE_TEST_DB").as_deref(),
            Ok("1")
        );
        let database_url = std::env::var("PULSO_TEST_POSTGRES_URL")
            .expect("isolated temporal-successor test database");
        let mut client = postgres::Client::connect(&database_url, postgres::NoTls)
            .expect("connect isolated test database");
        for migration in [
            include_str!("../../../migrations/0001_pulso_artifact_revisions.sql"),
            include_str!("../../../migrations/0002_pulso_memory_control.sql"),
            include_str!("../../../migrations/0003_pulso_run_events.sql"),
            include_str!("../../../migrations/0004_pulso_memory_temporal_receipts.sql"),
            include_str!("../../../migrations/0005_p4_temporal_successor_outbox.sql"),
        ] {
            client.batch_execute(migration).unwrap();
        }
        client.batch_execute("TRUNCATE pulso_memory_use_receipts, pulso_memory_tombstones, pulso_memory_lineage, pulso_memory_heads, pulso_jobs, pulso_artifact_heads, pulso_artifact_revisions CASCADE").unwrap();

        let mut artifacts = crate::PostgresArtifactRepository::new(client);
        let artifact = artifacts
            .append(
                None,
                ArtifactDraft::new(
                    TENANT,
                    WIKI_ID,
                    1,
                    ArtifactKind::MemoryWiki,
                    json!({
                        "available_at_unix_seconds": 100,
                        "purpose": "investigation",
                        "pages": {"index.md": "p4 lease fixture"}
                    }),
                    None,
                ),
            )
            .unwrap();
        let snapshot_ref = artifact.reference();
        let mut client = artifacts.into_inner();
        client
            .query_one(
                "SELECT pulso_seed_memory_head($1,$2,$3,$4,$5,$6,$7::text::uuid,$8,$9)",
                &[
                    &TENANT,
                    &"investigation",
                    &"world-a",
                    &"campaign-a",
                    &"frozen",
                    &"train",
                    &snapshot_ref.id,
                    &1_i64,
                    &snapshot_ref.digest,
                ],
            )
            .unwrap();

        let request = MemoryUseRequest::new(
            scope("frozen"),
            WikiAccess::new_scoped(
                "source-run-p4",
                TENANT,
                "investigation",
                "grant-p4",
                snapshot_ref,
                150,
                MemoryScopeBinding::new("world-a", "campaign-a", "frozen", "train"),
            ),
        );
        let evidence = || {
            TrustedTemporalEvidenceIssuer::deterministic(
                VerifiedAvailabilityProjection::deterministic(request.clone(), 200, None, None),
            )
            .attest(MemoryTemporalProtocol::Frozen)
        };
        let event_ref = opaque_event_ref('a');
        let event_time_ms = 1_759_420_800_000;
        let expected_successor_id = successor_run_id(TENANT, &event_ref, event_time_ms).unwrap();
        let counts = |client: &mut postgres::Client| -> (i64, i64, i64) {
            let receipts: i64 = client
                .query_one(
                    "SELECT count(*) FROM pulso_memory_use_receipts WHERE tenant_id=$1 AND event_ref=$2",
                    &[&TENANT, &event_ref],
                )
                .unwrap()
                .get(0);
            let jobs: i64 = client
                .query_one(
                    "SELECT count(*) FROM pulso_jobs WHERE tenant_id=$1 AND source_event_ref=$2",
                    &[&TENANT, &event_ref],
                )
                .unwrap()
                .get(0);
            let events: i64 = client
                .query_one(
                    "SELECT count(*) FROM pulso_run_events WHERE tenant_id=$1 AND run_ref=$2::text::uuid AND event_code='job_queued'",
                    &[&TENANT, &expected_successor_id],
                )
                .unwrap()
                .get(0);
            (receipts, jobs, events)
        };

        let missing = record_temporal_successor(
            &mut client,
            &MissingMemoryGrantAuthority,
            MemoryTemporalProtocol::Frozen,
            evidence(),
            request.clone(),
            TemporalSuccessorTrigger {
                expected_head_version: 1,
                source_event_ref: event_ref.to_owned(),
                event_occurred_at_unix_ms: event_time_ms,
            },
        );
        assert!(matches!(
            missing,
            Err(TemporalSuccessorError::DependencyBlocked)
        ));
        assert_eq!(counts(&mut client), (0, 0, 0));

        for state in [
            GrantFixtureState::Revoked,
            GrantFixtureState::Expired,
            GrantFixtureState::ScopeMismatch,
        ] {
            let denied = record_temporal_successor(
                &mut client,
                &LocalGrantLeaseFixture::new(state),
                MemoryTemporalProtocol::Frozen,
                evidence(),
                request.clone(),
                TemporalSuccessorTrigger {
                    expected_head_version: 1,
                    source_event_ref: event_ref.to_owned(),
                    event_occurred_at_unix_ms: event_time_ms,
                },
            );
            assert!(matches!(denied, Err(TemporalSuccessorError::GrantDenied)));
            assert_eq!(counts(&mut client), (0, 0, 0));
        }

        let admitted = record_temporal_successor(
            &mut client,
            &LocalGrantLeaseFixture::new(GrantFixtureState::Valid),
            MemoryTemporalProtocol::Frozen,
            evidence(),
            request.clone(),
            TemporalSuccessorTrigger {
                expected_head_version: 1,
                source_event_ref: event_ref.to_owned(),
                event_occurred_at_unix_ms: event_time_ms,
            },
        )
        .expect("test-only current lease allows durable request");
        assert_eq!(admitted.status, "requested");
        assert!(is_uuid_v7(&admitted.successor_run_id));
        assert_eq!(counts(&mut client), (1, 1, 1));

        let replay = record_temporal_successor(
            &mut client,
            &LocalGrantLeaseFixture::new(GrantFixtureState::Valid),
            MemoryTemporalProtocol::Frozen,
            evidence(),
            request.clone(),
            TemporalSuccessorTrigger {
                expected_head_version: 1,
                source_event_ref: event_ref.to_owned(),
                event_occurred_at_unix_ms: event_time_ms,
            },
        )
        .expect("ambiguous-commit replay returns the same durable request");
        assert_eq!(replay, admitted);
        assert_eq!(counts(&mut client), (1, 1, 1));

        // Race a revocation attempt against the actual durable callback. The
        // fixture reports that it cannot acquire the lease fence while the
        // request transaction is held; revocation proceeds only after commit.
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let authority = Arc::new(LocalGrantLeaseFixture::held_valid_lease(
            Arc::clone(&entered),
            Arc::clone(&release),
        ));
        let database_url = database_url.clone();
        let race_request = MemoryUseRequest::new(
            scope("frozen"),
            WikiAccess::new_scoped(
                "source-run-p4-race",
                TENANT,
                "investigation",
                "grant-p4",
                request.access().snapshot_ref.clone(),
                150,
                MemoryScopeBinding::new("world-a", "campaign-a", "frozen", "train"),
            ),
        );
        let race_evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(race_request.clone(), 200, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);
        let writer_authority = Arc::clone(&authority);
        let race_event_ref = opaque_event_ref('b');
        let writer_event_ref = race_event_ref.clone();
        let writer = std::thread::spawn(move || {
            let mut client = postgres::Client::connect(&database_url, postgres::NoTls)
                .expect("connect isolated race database");
            record_temporal_successor(
                &mut client,
                writer_authority.as_ref(),
                MemoryTemporalProtocol::Frozen,
                race_evidence,
                race_request,
                TemporalSuccessorTrigger {
                    expected_head_version: 1,
                    source_event_ref: writer_event_ref,
                    event_occurred_at_unix_ms: event_time_ms,
                },
            )
        });
        entered.wait();
        let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
        let revoker_authority = Arc::clone(&authority);
        let revoker = std::thread::spawn(move || {
            let blocked = revoker_authority.try_begin_revoke();
            attempted_tx
                .send(blocked)
                .expect("report revoke lock result");
            if blocked {
                revoker_authority.revoke_after_lease();
            }
        });
        assert_eq!(
            attempted_rx.recv_timeout(std::time::Duration::from_secs(2)),
            Ok(true),
            "revocation must be fenced until receipt+outbox transaction commits"
        );
        release.wait();
        let raced = writer
            .join()
            .expect("successor writer thread completes")
            .expect("held lease permits durable successor request");
        assert_eq!(raced.status, "requested");
        revoker.join().expect("revocation completes after commit");
        let race_successor_id = successor_run_id(TENANT, &race_event_ref, event_time_ms).unwrap();
        let post_race_counts_row = client
            .query_one(
                "SELECT (SELECT count(*) FROM pulso_memory_use_receipts WHERE tenant_id=$1 AND event_ref=$2), (SELECT count(*) FROM pulso_jobs WHERE tenant_id=$1 AND source_event_ref=$2), (SELECT count(*) FROM pulso_run_events WHERE tenant_id=$1 AND run_ref=$3::text::uuid AND event_code='job_queued')",
                &[&TENANT, &race_event_ref, &race_successor_id],
            )
            .unwrap();
        let post_race_counts = (
            post_race_counts_row.get::<_, i64>(0),
            post_race_counts_row.get::<_, i64>(1),
            post_race_counts_row.get::<_, i64>(2),
        );
        assert_eq!(post_race_counts, (1, 1, 1));
    }

    fn seeded(
        protocol: &str,
    ) -> (
        InMemoryArtifactRepository,
        InMemoryGovernedMemoryCommitPort,
        WikiAccess,
    ) {
        let mut artifacts = InMemoryArtifactRepository::default();
        let snapshot = artifacts
            .append(
                None,
                ArtifactDraft::new(
                    TENANT,
                    WIKI_ID,
                    1,
                    ArtifactKind::MemoryWiki,
                    json!({
                        "available_at_unix_seconds": 100,
                        "purpose": "investigation",
                        "pages": {"index.md": "published"}
                    }),
                    None,
                ),
            )
            .expect("fixed memory wiki")
            .reference();
        let binding = MemoryScopeBinding::new("world-a", "campaign-a", protocol, "train");
        let access = WikiAccess::new_scoped(
            "run-2",
            TENANT,
            "investigation",
            "grant-2",
            snapshot.clone(),
            100,
            binding.clone(),
        );
        let authority = InMemoryWikiGrantAuthority::default();
        authority.issue(WikiGrant::new_scoped(
            "grant-2",
            "run-2",
            TENANT,
            "investigation",
            snapshot.clone(),
            binding,
        ));
        let mut registry = InMemoryMemoryRegistry::default();
        registry
            .seed_head(&mut artifacts, scope(protocol), snapshot)
            .expect("fixed memory head");
        (
            artifacts,
            InMemoryGovernedMemoryCommitPort::new(registry, authority),
            access,
        )
    }

    #[test]
    fn continuous_admission_rejects_an_outcome_not_yet_available_without_recording_a_receipt() {
        let (mut artifacts, mut publisher, access) = seeded("continuous");

        let request = MemoryUseRequest::new(scope("continuous"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(
                request.clone(),
                100,
                Some(101),
                Some("outcome:1".into()),
            ),
        )
        .attest(MemoryTemporalProtocol::Continuous);
        let result = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Continuous,
            evidence,
            &mut publisher,
            &mut artifacts,
            request,
        );

        match result {
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::OutcomeAfterMemoryUse,
            )) => {}
            Err(other) => panic!("expected temporal outcome denial, got {other:?}"),
            Ok(_) => panic!("future outcome must not mint a capability"),
        }
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn admission_binds_its_temporal_claim_to_the_u33_receipt_clock_before_recording() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");

        let request = MemoryUseRequest::new(scope("frozen"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(request.clone(), 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);
        let result = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            evidence,
            &mut publisher,
            &mut artifacts,
            request,
        );

        let admitted = result.expect("issuer binds the exact access time");
        assert_eq!(
            admitted.receipt().temporal_commitment,
            publisher.receipts()[0].temporal_commitment
        );
        assert!(admitted.receipt().temporal_commitment.is_some());
    }

    #[test]
    fn frozen_admission_mints_only_the_existing_u22_receipt_provenance() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");

        let request = MemoryUseRequest::new(scope("frozen"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(request.clone(), 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);
        let admitted = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            evidence,
            &mut publisher,
            &mut artifacts,
            request,
        )
        .expect("timely U22/U33 frozen admission");

        assert_eq!(admitted.head_version(), 1);
        assert_eq!(admitted.run_id(), "run-2");
        assert_eq!(publisher.receipts().len(), 1);
    }

    #[test]
    fn production_issuer_only_accepts_a_revalidated_u04b_replay_projection() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access);
        let issuer = TrustedTemporalEvidenceIssuer::from_u04b_replay(
            u04b_replay_projection(),
            request.clone(),
        )
        .expect("tenant and world were bound by the verified U04-B projection");
        let admitted = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            issuer.attest(MemoryTemporalProtocol::Frozen),
            &mut publisher,
            &mut artifacts,
            request,
        )
        .expect("bound replay projection can produce only the governed receipt");

        assert_eq!(admitted.scope().tenant_id, TENANT);
        assert_eq!(publisher.receipts().len(), 1);
    }

    #[test]
    fn production_issuer_rejects_u04b_projection_for_another_world_before_a_receipt() {
        let (_artifacts, publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(
            MemoryScope::new(
                TENANT,
                "investigation",
                "world-b",
                "campaign-a",
                "frozen",
                "train",
            ),
            access,
        );

        assert!(matches!(
            TrustedTemporalEvidenceIssuer::from_u04b_replay(u04b_replay_projection(), request),
            Err(TemporalProtocolError::U04BReplayScopeMismatch)
        ));
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn production_issuer_rejects_u04b_projection_for_another_tenant_before_a_receipt() {
        let (_artifacts, publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access);

        assert!(matches!(
            TrustedTemporalEvidenceIssuer::from_u04b_replay(
                u04b_replay_projection_for("tenant-b", ""),
                request,
            ),
            Err(TemporalProtocolError::U04BReplayScopeMismatch)
        ));
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn source_snapshot_and_profile_digest_change_the_temporal_commitment() {
        let (_artifacts, _publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access);
        let first = TrustedTemporalEvidenceIssuer::from_u04b_replay(
            u04b_replay_projection_for(TENANT, ""),
            request.clone(),
        )
        .expect("first exact U04-B snapshot");
        let same_semantics_new_snapshot = TrustedTemporalEvidenceIssuer::from_u04b_replay(
            u04b_replay_projection_for(TENANT, "\n"),
            request,
        )
        .expect("second exact U04-B snapshot");

        assert_ne!(
            first.attest(MemoryTemporalProtocol::Frozen).commitment,
            same_semantics_new_snapshot
                .attest(MemoryTemporalProtocol::Frozen)
                .commitment,
            "raw snapshot and profile seals are part of temporal identity"
        );
    }

    #[test]
    fn reissued_grant_revision_cannot_reuse_prior_temporal_evidence_or_leave_a_receipt() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");
        let original_request = MemoryUseRequest::new(scope("frozen"), access.clone());
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(original_request, 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);

        let binding = MemoryScopeBinding::new("world-a", "campaign-a", "frozen", "train");
        publisher.authority().issue(
            WikiGrant::new_scoped(
                "grant-2",
                "run-2",
                TENANT,
                "investigation",
                access.snapshot_ref.clone(),
                binding,
            )
            .with_revision(2),
        );
        let reissued_request =
            MemoryUseRequest::new(scope("frozen"), access.with_grant_revision(2));

        assert!(matches!(
            MemoryTemporalAdmission::admit(
                MemoryTemporalProtocol::Frozen,
                evidence,
                &mut publisher,
                &mut artifacts,
                reissued_request,
            ),
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::GrantRevisionMismatch
            ))
        ));
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn evidence_rejects_caller_elevation_of_allowed_at_before_u33_side_effects() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access);
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(request.clone(), 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);
        let original_access = request.access().clone();
        let elevated = MemoryUseRequest::new(
            scope("frozen"),
            WikiAccess::new_scoped(
                original_access.run_id,
                original_access.tenant_id,
                original_access.purpose,
                original_access.grant_id,
                original_access.snapshot_ref,
                101,
                original_access.memory_scope,
            ),
        );

        assert!(matches!(
            MemoryTemporalAdmission::admit(
                MemoryTemporalProtocol::Frozen,
                evidence,
                &mut publisher,
                &mut artifacts,
                elevated,
            ),
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch
            ))
        ));
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn evidence_cannot_cross_a_memory_scope_before_u33_side_effects() {
        let (mut artifacts, mut publisher, access) = seeded("frozen");
        let request = MemoryUseRequest::new(scope("frozen"), access.clone());
        let evidence = TrustedTemporalEvidenceIssuer::deterministic(
            VerifiedAvailabilityProjection::deterministic(request.clone(), 100, None, None),
        )
        .attest(MemoryTemporalProtocol::Frozen);
        let crossed = MemoryUseRequest::new(
            MemoryScope::new(
                TENANT,
                "investigation",
                "world-b",
                "campaign-a",
                "frozen",
                "train",
            ),
            access,
        );
        assert!(matches!(
            MemoryTemporalAdmission::admit(
                MemoryTemporalProtocol::Frozen,
                evidence,
                &mut publisher,
                &mut artifacts,
                crossed,
            ),
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch
            ))
        ));
        assert!(publisher.receipts().is_empty());
    }

    #[test]
    fn successor_run_identity_is_stable_for_the_same_event_and_valid_uuid_v7() {
        let first = successor_run_id("tenant-a", "platform/event-7", 1_759_420_800_000)
            .expect("valid event identity");
        let retry = successor_run_id("tenant-a", "platform/event-7", 1_759_420_800_000)
            .expect("same event identity");
        let other_event = successor_run_id("tenant-a", "platform/event-8", 1_759_420_800_000)
            .expect("different event identity");

        assert_eq!(first, retry, "replay must recover the same successor run");
        assert_ne!(first, other_event, "different events must not collide");
        assert!(is_uuid_v7(&first), "successor root must satisfy U06 UUIDv7");
    }

    #[test]
    fn source_event_reference_must_be_a_canonical_opaque_token() {
        let valid = format!("sha256_{}", "a".repeat(56));
        assert!(super::is_opaque_source_event_ref(&valid));
        for raw_or_malformed in [
            "customer@example.com",
            "platform-event-0001",
            "sha256_ABCDEF",
            "sha256_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!",
        ] {
            assert!(
                !super::is_opaque_source_event_ref(raw_or_malformed),
                "reference must not admit raw or malformed input: {raw_or_malformed}"
            );
        }
    }

    #[test]
    fn successor_sqlstate_exposes_conflicts_and_reconciliation_as_domain_outcomes() {
        assert_eq!(
            super::classify_temporal_successor_sqlstate(
                &postgres::error::SqlState::UNIQUE_VIOLATION
            ),
            super::TemporalDatabaseOutcome::Conflict
        );
        assert_eq!(
            super::classify_temporal_successor_sqlstate(
                &postgres::error::SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE
            ),
            super::TemporalDatabaseOutcome::ReconciliationRequired
        );
        assert_eq!(
            super::classify_temporal_successor_sqlstate(
                &postgres::error::SqlState::CONNECTION_FAILURE
            ),
            super::TemporalDatabaseOutcome::Storage
        );
    }
}
