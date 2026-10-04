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

/// SmuãÏm¢G§²ÚîÆ­yÐ¢66W72À¢“° ¢76W'B†ÖF6†W2€¢G'W7FVEFV×÷&ÄWf–FVæ6T—77VW#£¦g&öÕ÷SF%÷&WÆ’‡SF%÷&WÆ•÷&ö¦V7F–öâ‚’Â&WVW7B’À¢W'"…FV×÷&Å&÷Fö6öÄW'&÷#£¥SD%&WÆ•66÷TÖ—6ÖF6‚¢’“°¢76W'B‡V&Æ—6†W"ç&V6V—G2‚’æ—5öV×G’‚’“°¢Ð ¢5·FW7EÐ¢fâ&öGV7F–öåö—77VW%÷&V¦V7G5÷SF%÷&ö¦V7F–öåöf÷%öæ÷F†W%÷FVæçEö&Vf÷&Uö÷&V6V—B‚’°¢ÆWB…ö'F–f7G2ÂV&Æ—6†W"Â66W72’Ò6VVFVB‚&g&÷¦Vâ"“°¢ÆWB&WVW7BÒÖVÖ÷'•W6U&WVW7C£¦æWr‡66÷R‚&g&÷¦Vâ"’Â66W72“° ¢76W'B†ÖF6†W2€¢G'W7FVEFV×÷&ÄWf–FVæ6T—77VW#£¦g&öÕ÷SF%÷&WÆ’€¢SF%÷&WÆ•÷&ö¦V7F–öåöf÷"‚'FVæçBÖ""Â""’À¢&WVW7BÀ¢’À¢W'"…FV×÷&Å&÷Fö6öÄW'&÷#£¥SD%&WÆ•66÷TÖ—6ÖF6‚¢’“°¢76W'B‡V&Æ—6†W"ç&V6V—G2‚’æ—5öV×G’‚’“°¢Ð ¢5·FW7EÐ¢fâ6÷W&6U÷6æ6†÷EöæE÷&öf–ÆUöF–vW7Eö6†ævU÷F†U÷FV×÷&Åö6öÖÖ—FÖVçB‚’°¢ÆWB…ö'F–f7G2Â÷V&Æ—6†W"Â66W72’Ò6VVFVB‚&g&÷¦Vâ"“°¢ÆWB&WVW7BÒÖVÖ÷'•W6U&WVW7C£¦æWr‡66÷R‚&g&÷¦Vâ"’Â66W72“°¢ÆWBf—'7BÒG'W7FVEFV×÷&ÄWf–FVæ6T—77VW#£¦g&öÕ÷SF%÷&WÆ’€¢SF%÷&WÆ•÷&ö¦V7F–öåöf÷"…DTäåBÂ""’À¢&WVW7Bæ6ÆöæR‚’À¢¢æW‡V7B‚&f—'7BW†7BSBÔ"6æ6†÷B"“°¢ÆWB6ÖU÷6VÖçF–75öæWu÷6æ6†÷BÒG'W7FVEFV×÷&ÄWf–FVæ6T—77VW#£¦g&öÕ÷SF%÷&WÆ’€¢SF%÷&WÆ•÷&ö¦V7F–öåöf÷"…DTäåBÂ%Æâ"’À¢&WVW7BÀ¢¢æW‡V7B‚'6V6öæBW†7BSBÔ"6æ6†÷B"“° ¢76W'EöæR€¢f—'7BæGFW7B„ÖVÖ÷'•FV×÷&Å&÷Fö6öÃ£¤g&÷¦Vâ’æ6öÖÖ—FÖVçBÀ¢6ÖU÷6VÖçF–75öæWu÷6æ6†÷@¢æGFW7B„ÖVÖ÷'•FV×÷&Å&÷Fö6öÃ£¤g&÷¦Vâ¢æ6öÖÖ—FÖVçBÀ¢'&r6æ6†÷BæB&öf–ÆR6VÇ2&R'BöbFV×÷&Â–FVçF—G’ ¢“°¢Ð ¢5·FW7EÐ¢fâ&V—77VVEöw&çE÷&Wf—6–öåö6ææ÷E÷&WW6U÷&–÷%÷FV×÷&ÅöWf–FVæ6Uö÷%öÆVfUö÷&V6V—B‚’°¢ÆWB†×WB'F–f7G2Â×WBV&Æ—6†W"Â66W72’Ò6VVFVB‚&g&÷¦Vâ"“°¢ÆWB÷&–v–æÅ÷&WVW7BÒÖVÖ÷'•W6U&WVW7C£¦æWr‡66÷R‚&g&÷¦Vâ"’Â66W72æ6ÆöæR‚’“°¢ÆWBWf–FVæ6RÒG'W7FVEFV×÷&ÄWf–FVæ6T—77VW#£¦FWFW&Ö–æ—7F–2€¢fW&–f–VDf–Æ&–Æ—G•&ö¦V7F–öã£¦FWFW&Ö–æ—7F–2†÷&–v–æÅ÷&WVW7BÂÂæöæRÂæöæR’À¢¢æGFW7B„ÖVÖ÷'•FV×÷&Å&÷Fö6öÃ£¤g&÷¦Vâ“° ¢ÆWB&–æF–ærÒÖVÖ÷'•66÷T&–æF–æs£¦æWr‚'v÷&ÆBÖ"Â&6×–vâÖ"Â&g&÷¦Vâ"Â'G&–â"“°¢V&Æ—6†W"æWF†÷&—G’‚’æ—77VR€¢v–¶”w&çC£¦æWu÷66÷VB€¢&w&çBÓ""À¢''VâÓ""À¢DTäåBÀ¢&–çfW7F–vF–öâ"À¢66W72ç6æ6†÷E÷&Vbæ6ÆöæR‚’À¢&–æF–ærÀ¢¢çv—F…÷&Wf—6–öâƒ"’À¢“°¢ÆWB&V—77VVE÷&WVW7BÐ¢ÖVÖ÷'•W6U&WVW7C£¦æWr‡66÷R‚&g&÷¦Vâ"’Â66W72çv—F…öw&çE÷&Wf—6–öâƒ"’“° ¢76W'B†ÖF6†W2€¢ÖVÖ÷'•FV×÷&ÄFÖ—76–öã£¦FÖ—B€¢ÖVÖ÷'•FV×÷&Å&÷Fö6öÃ£¤g&÷¦VâÀ¢Wf–FVæ6RÀ¢f×WBV&Æ—6†W"À¢f×WB'F–f7G2À¢&V—77VVE÷&WVW7BÀ¢’À¢W'"…FV×÷&ÄÖVÖ÷'”FÖ—76–öäW'&÷#£¥FV×÷&Â€¢FV×÷&Å&÷Fö6öÄW'&÷#£¤w&çE&Wf—6–öäÖ—6ÖF6€¢’¢’“°¢76W'B‡V&Æ—6†W"ç&V6V—G2‚’æ—5öV×G’‚’“°¢Ð ¢5·FW7EÐ¢fâWf–FVæ6U÷&V¦V7G5ö6ÆÆW%öVÆWfF–öåööeöÆÆ÷vVEöEö&Vf÷&U÷S35÷6–FUöVffV7G2‚’°¢ÆWB†×WB'F–f7G2Â×WBV&Æ—6†W"Â66W72’Ò6VVFVB‚&g&÷¦Vâ"“°¢ÆWB&WVW7BÒÖVÖ÷'•W6U&WVW7C£¦æWr‡66÷R‚&g&÷¦Vâ"’Â66W72“°¢ÆWBWf–FVæ6RÒG'W7FVEFV×÷&ÄWf–FVæ6T—77VW#£¦FWFW&Ö–æ—7F–2€¢fW&–f–VDf–Æ&–Æ—G•&ö¦V7F–öã£¦FWFW&Ö–æ—7F–2‡&WVW7Bæ6ÆöæR‚’ÂÂæöæRÂæöæR’À¢¢æGFW7B„ÖVÖ÷'•FV×÷&Å&÷Fö6öÃ£¤g&÷¦Vâ“°¢ÆWB÷&–v–æÅö66W72Ò&WVW7Bæ66W72‚’æ6ÆöæR‚“°¢ÆWBVÆWfFVBÒÖVÖ÷'•W6U&WVW7C£¦æWr€¢66÷R‚&g&÷¦Vâ"’À¢v–¶”66W73£¦æWu÷66÷VB€¢÷&–v–æÅö66W72ç'Våö–BÀ¢÷&–v–æÅö66W72çFVæçEö–BÀ¢÷&–v–æÅö66W72çW'÷6RÀ¢÷&–v–æÅö66W72æw&çEö–BÀ¢÷&–v–æÅö66W72ç6æ6†÷E÷&VbÀ¢À¢÷&–v–æÅö66W72æÖVÖ÷'•÷66÷RÀ¢’À¢“° ¢76W'B†ÖF6†W2€¢ÖVÖ÷'•FV×÷&ÄFÖ—76–öã£¦FÖ—B€¢ÖVÖ÷'•FV×÷&Å&÷Fö6öÃ£¤g&÷¦VâÀ¢Wf–FVæ6RÀ¢f×WBV&Æ—6†W"À¢f×WB'F–f7G2À¢VÆWfFVBÀ¢’À¢W'"…FV×÷&ÄÖVÖ÷'”FÖ—76–öäW'&÷#£¥FV×÷&Â€¢FV×÷&Å&÷Fö6öÄW'&÷#£¤66W75F–ÖTÖ—6ÖF6€¢’¢’“°¢76W'B‡V&Æ—6†W"ç&V6V—G2‚’æ—5öV×G’‚’“°¢Ð ¢5·FW7EÐ¢fâWf–FVæ6Uö6ææ÷Eö7&÷75ööÖVÖ÷'•÷66÷Uö&Vf÷&U÷S35÷6–FUöVffV7G2‚’°¢ÆWB†×WB'F–f7G2Â×WBV&Æ—6†W"Â66W72’Ò6VVFVB‚&g&÷¦Vâ"“°¢ÆWB&WVW7BÒÖVÖ÷'•W6U&WVW7C£¦æWr‡66÷R‚&g&÷¦Vâ"’Â66W72æ6ÆöæR‚’“°¢ÆWBWf–FVæ6RÒG'W7FVEFV×÷&ÄWf–FVæ6T—77VW#£¦FWFW&Ö–æ—7F–2€¢fW&–f–VDf–Æ&–Æ—G•&ö¦V7F–öã£¦FWFW&Ö–æ—7F–2‡&WVW7Bæ6ÆöæR‚’ÂÂæöæRÂæöæR’À¢¢æGFW7B„ÖVÖ÷'•FV×÷&Å&÷Fö6öÃ£¤g&÷¦Vâ“°¢ÆWB7&÷76VBÒÖVÖ÷'•W6U&WVW7C£¦æWr€¢ÖVÖ÷'•66÷S£¦æWr€¢DTäåBÀ¢&–çfW7F–vF–öâ"À¢'v÷&ÆBÖ""À¢&6×–vâÖ"À¢&g&÷¦Vâ"À¢'G&–â"À¢’À¢66W72À¢“°¢76W'B†ÖF6†W2€¢ÖVÖ÷'•FV×÷&ÄFÖ—76–öã£¦FÖ—B€¢ÖVÖ÷'•FV×÷&Å&÷Fö6öÃ£¤g&÷¦VâÀ¢Wf–FVæ6RÀ¢f×WBV&Æ—6†W"À¢f×WB'F–f7G2À¢7&÷76VBÀ¢’À¢W'"…FV×÷&ÄÖVÖ÷'”FÖ—76–öäW'&÷#£¥FV×÷&Â€¢FV×÷&Å&÷Fö6öÄW'&÷#£¤66W75F–ÖTÖ—6ÖF6€¢’¢’“°¢76W'B‡V&Æ—6†W"ç&V6V—G2‚’æ—5öV×G’‚’“°¢Ð ¢5·FW7EÐ¢fâ7V66W76÷%÷'Våö–FVçF—G•ö—5÷7F&ÆUöf÷%÷F†U÷6ÖUöWfVçEöæE÷fÆ–E÷WV–E÷cr‚’°¢ÆWBf—'7BÒ7V66W76÷%÷'Våö–B‚'FVæçBÖ"Â'ÆFf÷&ÒöWfVçBÓr"ÂósS•óC#óƒó¢æW‡V7B‚'fÆ–BWfVçB–FVçF—G’"“°¢ÆWB&WG'’Ò7V66W76÷%÷'Våö–B‚'FVæçBÖ"Â'ÆFf÷&ÒöWfVçBÓr"ÂósS•óC#óƒó¢æW‡V7B‚'6ÖRWfVçB–FVçF—G’"“°¢ÆWB÷F†W%öWfVçBÒ7V66W76÷%÷'Våö–B‚'FVæçBÖ"Â'ÆFf÷&ÒöWfVçBÓ‚"ÂósS•óC#óƒó¢æW‡V7B‚&F–ffW&VçBWfVçB–FVçF—G’"“° ¢76W'EöW†f—'7BÂ&WG'’Â'&WÆ’×W7B&V6÷fW"F†R6ÖR7V66W76÷"'Vâ"“°¢76W'EöæR†f—'7BÂ÷F†W%öWfVçBÂ&F–ffW&VçBWfVçG2×W7Bæ÷B6öÆÆ–FR"“°¢76W'B†—5÷WV–E÷cr‚ff—'7B’Â'7V66W76÷"&ö÷B×W7B6F—6g’SbUT”Gcr"“°¢Ð ¢5·FW7EÐ¢fâ6÷W&6UöWfVçE÷&VfW&Væ6Uö×W7Eö&Uöö6æöæ–6Åö÷VU÷Fö¶Vâ‚’°¢ÆWBfÆ–BÒf÷&ÖB‚'6†#Se÷·Ò"Â&"ç&WVBƒSb’“°¢76W'B‡7WW#£¦—5ö÷VU÷6÷W&6UöWfVçE÷&Vb‚gfÆ–B’“°¢f÷"&uö÷%öÖÆf÷&ÖVB–â°¢&7W7FöÖW$W†×ÆRæ6öÒ"À¢'ÆFf÷&ÒÖWfVçBÓ"À¢'6†#Seô$4DTb"À¢'6†#Seö"À¢Ò°¢76W'B€¢7WW#£¦—5ö÷VU÷6÷W&6UöWfVçE÷&Vb‡&uö÷%öÖÆf÷&ÖVB’À¢'&VfW&Væ6R×W7Bæ÷BFÖ—B&r÷"ÖÆf÷&ÖVB–çWC¢·&uö÷%öÖÆf÷&ÖVGÒ ¢“°¢Ð¢Ð ¢5·FW7EÐ¢fâ7V66W76÷%÷7Ç7FFUöW‡÷6W5ö6öæfÆ–7G5öæE÷&V6öæ6–Æ–F–öåö5öFöÖ–åö÷WF6öÖW2‚’°¢76W'EöW€¢7WW#£¦6Æ76–g•÷FV×÷&Å÷7V66W76÷%÷7Ç7FFR€¢g÷7Fw&W3£¦W'&÷#£¥7Å7FFS£¥Tä•TUõd”ôÄD”ôà¢’À¢7WW#£¥FV×÷&ÄFF&6T÷WF6öÖS£¤6öæfÆ–7@¢“°¢76W'EöW€¢7WW#£¦6Æ76–g•÷FV×÷&Å÷7V66W76÷%÷7Ç7FFR€¢g÷7Fw&W3£¦W'&÷#£¥7Å7FFS£¤ô$¤T5EôäõEô”åõ$U$UT•4•DUõ5DDP¢’À¢7WW#£¥FV×÷&ÄFF&6T÷WF6öÖS£¥&V6öæ6–Æ–F–öå&WV—&V@¢“°¢76W'EöW€¢7WW#£¦6Æ76–g•÷FV×÷&Å÷7V66W76÷%÷7Ç7FFR€¢g÷7Fw&W3£¦W'&÷#£¥7Å7FFS£¤4ôääT5D”ôåôd”ÅU$P¢’À¢7WW#£¥FV×÷&ÄFF&6T÷WF6öÖS£¥7F÷&vP¢“°¢Ð§Ð