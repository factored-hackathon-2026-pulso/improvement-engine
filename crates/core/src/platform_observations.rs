//! Treated, tenant-scoped evidence from the external attention platform.
//!
//! A platform audit event is eligible to support a denominator only when its
//! declared source coverage is complete. Sampled OTel evidence is diagnostic.
//! No free-text customer payload crosses this boundary.

use postgres::{Client, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const INLINE_TREATED_BLOB_MAX_BYTES: usize = 262_144;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    CoreAudit,
    PlatformAudit,
    OtelSampledSpan,
    OtelSampledLog,
    OtelMetric,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetSystem {
    Attention,
    Evolution,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportSequenceMode {
    Contiguous,
    Opaque,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceSamplingMode {
    DurableAudit,
    SampledDiagnostic,
    Mixed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceClockMode {
    OccurredAndReceivedEpochMillis,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceCorrelationMode {
    SourceRunRequiredEpisodeOptional,
}

/// Trusted adapter configuration, not a declaration supplied by an agent or
/// by the same untrusted batch whose completeness it is meant to validate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationSourceContract {
    source_id: String,
    contract_ref: String,
    target_system: TargetSystem,
    sequence_mode: TransportSequenceMode,
    sampling_mode: SourceSamplingMode,
    complete_coverage_supported: bool,
    clock_mode: SourceClockMode,
    correlation_mode: SourceCorrelationMode,
}

impl ObservationSourceContract {
    pub fn new(
        source_id: impl Into<String>,
        contract_ref: impl Into<String>,
        target_system: TargetSystem,
        sequence_mode: TransportSequenceMode,
        sampling_mode: SourceSamplingMode,
        complete_coverage_supported: bool,
    ) -> Result<Self, ObservationError> {
        let contract = Self {
            source_id: source_id.into(),
            contract_ref: contract_ref.into(),
            target_system,
            sequence_mode,
            sampling_mode,
            complete_coverage_supported,
            clock_mode: SourceClockMode::OccurredAndReceivedEpochMillis,
            correlation_mode: SourceCorrelationMode::SourceRunRequiredEpisodeOptional,
        };
        validate_ref(&contract.source_id)?;
        validate_ref(&contract.contract_ref)?;
        if sampling_mode == SourceSamplingMode::SampledDiagnostic && complete_coverage_supported {
            return Err(ObservationError::InvalidSourceContract);
        }
        Ok(contract)
    }

    pub fn clock_mode(&self) -> SourceClockMode {
        self.clock_mode
    }
    pub fn correlation_mode(&self) -> SourceCorrelationMode {
        self.correlation_mode
    }

    fn validate_batch(&self, batch: &PlatformObservationBatch) -> Result<(), ObservationError> {
        if self.source_id != batch.source_id || self.contract_ref != batch.contract_ref {
            return Err(ObservationError::UnknownSourceContract);
        }
        if batch
            .events
            .iter()
            .any(|event| event.target_system != self.target_system)
        {
            return Err(ObservationError::SourceTargetMismatch);
        }
        match (self.sequence_mode, batch.cursor.from_seq) {
            (TransportSequenceMode::Contiguous, None)
            | (TransportSequenceMode::Opaque, Some(_)) => {
                return Err(ObservationError::SourceSequenceModeMismatch);
            }
            _ => {}
        }
        if batch.coverage.state == CoverageState::Complete
            && (!self.complete_coverage_supported
                || batch
                    .events
                    .iter()
                    .any(|event| !event.evidence_kind.is_durable_audit()))
        {
            return Err(ObservationError::CoverageDeclarationMismatch);
        }
        if batch.events.iter().any(|event| match self.sampling_mode {
            SourceSamplingMode::DurableAudit => !event.evidence_kind.is_durable_audit(),
            SourceSamplingMode::SampledDiagnostic => event.evidence_kind.is_durable_audit(),
            SourceSamplingMode::Mixed => false,
        }) {
            return Err(ObservationError::SourceSamplingMismatch);
        }
        Ok(())
    }
}

/// Immutable registry assembled by the trusted deployment composition root.
/// An ingest caller receives a repository, never the registry builder.
pub struct ObservationSourceRegistry {
    contracts: BTreeMap<(String, String), ObservationSourceContract>,
}

impl ObservationSourceRegistry {
    pub fn from_trusted_configuration(
        contracts: Vec<ObservationSourceContract>,
    ) -> Result<Self, ObservationError> {
        let mut by_identity = BTreeMap::new();
        for contract in contracts {
            let key = (contract.source_id.clone(), contract.contract_ref.clone());
            if by_identity.insert(key, contract).is_some() {
                return Err(ObservationError::InvalidSourceContract);
            }
        }
        Ok(Self {
            contracts: by_identity,
        })
    }

    fn get(&self, source_id: &str, contract_ref: &str) -> Option<&ObservationSourceContract> {
        self.contracts
            .get(&(source_id.to_owned(), contract_ref.to_owned()))
    }
}

impl EvidenceKind {
    pub fn is_durable_audit(self) -> bool {
        matches!(self, Self::CoreAudit | Self::PlatformAudit)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionEventKind {
    LayerAttempt,
    Handoff,
    ToolResult,
    ModelDecision,
    HumanAction,
    ResponseRequested,
    ResponseReceived,
    DeliveryStateChanged,
    CaseStateChanged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiagnosticEvidence {
    Metric {
        name: String,
        value_milli: i64,
        unit: String,
    },
    SampledLog {
        code: String,
        severity: DiagnosticSeverity,
    },
    SampledSpan {
        trace_ref: String,
        duration_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Classifier,
    Tree,
    Ai1,
    Ai2,
    Human,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationEvent {
    source_event_id: String,
    source_run_ref: String,
    target_system: TargetSystem,
    episode_ref: Option<String>,
    evidence_kind: EvidenceKind,
    event_kind: InteractionEventKind,
    layer: Option<Layer>,
    occurred_at_ms: i64,
    received_at_ms: i64,
    core_run_sequence: Option<u64>,
    source_event_digest: Option<String>,
    trace_refs: Vec<String>,
    diagnostic: Option<DiagnosticEvidence>,
}

impl ObservationEvent {
    pub fn new(
        source_event_id: impl Into<String>,
        source_run_ref: impl Into<String>,
        target_system: TargetSystem,
        evidence_kind: EvidenceKind,
        event_kind: InteractionEventKind,
        occurred_at_ms: i64,
        received_at_ms: i64,
    ) -> Result<Self, ObservationError> {
        let event = Self {
            source_event_id: source_event_id.into(),
            source_run_ref: source_run_ref.into(),
            target_system,
            episode_ref: None,
            evidence_kind,
            event_kind,
            layer: None,
            occurred_at_ms,
            received_at_ms,
            core_run_sequence: None,
            source_event_digest: None,
            trace_refs: Vec::new(),
            diagnostic: None,
        };
        event.validate()?;
        Ok(event)
    }

    pub fn source_event_id(&self) -> &str {
        &self.source_event_id
    }
    pub fn source_run_ref(&self) -> &str {
        &self.source_run_ref
    }
    pub fn target_system(&self) -> TargetSystem {
        self.target_system
    }
    pub fn event_kind(&self) -> InteractionEventKind {
        self.event_kind
    }
    pub fn layer(&self) -> Option<Layer> {
        self.layer
    }
    pub fn occurred_at_ms(&self) -> i64 {
        self.occurred_at_ms
    }
    pub fn received_at_ms(&self) -> i64 {
        self.received_at_ms
    }
    pub fn evidence_kind(&self) -> EvidenceKind {
        self.evidence_kind
    }
    pub fn diagnostic(&self) -> Option<&DiagnosticEvidence> {
        self.diagnostic.as_ref()
    }

    pub fn with_diagnostic(
        mut self,
        diagnostic: DiagnosticEvidence,
    ) -> Result<Self, ObservationError> {
        self.diagnostic = Some(diagnostic);
        self.validate()?;
        Ok(self)
    }

    pub fn with_source_event_digest(
        mut self,
        digest: impl Into<String>,
    ) -> Result<Self, ObservationError> {
        self.source_event_digest = Some(digest.into());
        self.validate()?;
        Ok(self)
    }

    /// Associates a treated event with a declared platform layer. The value is
    /// supplied by the trusted adapter, never inferred downstream from text.
    pub fn with_layer(mut self, layer: Layer) -> Result<Self, ObservationError> {
        self.layer = Some(layer);
        self.validate()?;
        Ok(self)
    }

    pub fn with_core_run_sequence(mut self, sequence: u64) -> Self {
        self.core_run_sequence = Some(sequence);
        self
    }

    fn validate(&self) -> Result<(), ObservationError> {
        validate_ref(&self.source_event_id)?;
        validate_ref(&self.source_run_ref)?;
        if self
            .episode_ref
            .as_deref()
            .is_some_and(|value| validate_ref(value).is_err())
            || self
                .trace_refs
                .iter()
                .any(|value| validate_ref(value).is_err())
        {
            return Err(ObservationError::InvalidRef);
        }
        if self
            .source_event_digest
            .as_deref()
            .is_some_and(|value| !is_digest(value))
        {
            return Err(ObservationError::DigestMismatch);
        }
        if let Some(diagnostic) = &self.diagnostic {
            match (self.evidence_kind, diagnostic) {
                (EvidenceKind::OtelMetric, DiagnosticEvidence::Metric { name, unit, .. }) => {
                    validate_ref(name)?;
                    validate_ref(unit)?;
                }
                (EvidenceKind::OtelSampledLog, DiagnosticEvidence::SampledLog { code, .. }) => {
                    validate_ref(code)?
                }
                (
                    EvidenceKind::OtelSampledSpan,
                    DiagnosticEvidence::SampledSpan {
                        trace_ref,
                        duration_ms,
                    },
                ) => {
                    validate_ref(trace_ref)?;
                    if *duration_ms == 0 {
                        return Err(ObservationError::InvalidDiagnostic);
                    }
                }
                _ => return Err(ObservationError::InvalidDiagnostic),
            }
        }
        if self.occurred_at_ms <= 0 || self.received_at_ms <= 0 {
            return Err(ObservationError::InvalidClock);
        }
        Ok(())
    }

    fn digest(&self) -> String {
        digest_json(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    Complete,
    Partial,
    Unknown,
    Degraded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    state: CoverageState,
    population_ref: String,
    window_start_ms: i64,
    window_end_ms: i64,
    expected_population: Option<u64>,
    missing_reason: Option<String>,
}

impl Coverage {
    pub fn new(
        state: CoverageState,
        population_ref: impl Into<String>,
        window_start_ms: i64,
        window_end_ms: i64,
        expected_population: Option<u64>,
        missing_reason: Option<String>,
    ) -> Result<Self, ObservationError> {
        let coverage = Self {
            state,
            population_ref: population_ref.into(),
            window_start_ms,
            window_end_ms,
            expected_population,
            missing_reason,
        };
        coverage.validate()?;
        Ok(coverage)
    }

    pub fn state(&self) -> CoverageState {
        self.state
    }
    pub fn expected_population(&self) -> Option<u64> {
        self.expected_population
    }
    pub fn population_ref(&self) -> &str {
        &self.population_ref
    }

    fn validate(&self) -> Result<(), ObservationError> {
        validate_ref(&self.population_ref)?;
        if self.window_start_ms <= 0 || self.window_end_ms <= self.window_start_ms {
            return Err(ObservationError::InvalidWindow);
        }
        if self.state == CoverageState::Complete {
            if self.expected_population.is_none() || self.missing_reason.is_some() {
                return Err(ObservationError::InvalidCoverage);
            }
        } else if self.missing_reason.as_deref().is_none_or(str::is_empty) {
            return Err(ObservationError::InvalidCoverage);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransportCursor {
    from_seq: Option<u64>,
    to_seq: Option<u64>,
    opaque: Option<String>,
}

impl TransportCursor {
    pub fn sequence(from_seq: u64, to_seq: u64) -> Self {
        Self {
            from_seq: Some(from_seq),
            to_seq: Some(to_seq),
            opaque: None,
        }
    }
    pub fn opaque(value: impl Into<String>) -> Self {
        Self {
            from_seq: None,
            to_seq: None,
            opaque: Some(value.into()),
        }
    }
    pub fn from_seq(&self) -> Option<u64> {
        self.from_seq
    }
    pub fn to_seq(&self) -> Option<u64> {
        self.to_seq
    }

    fn key(&self) -> String {
        match (self.from_seq, self.to_seq, self.opaque.as_deref()) {
            (Some(from), Some(to), None) => format!("seq:{from}:{to}"),
            (None, None, Some(value)) => format!("opaque:{value}"),
            _ => unreachable!("validated cursor"),
        }
    }

    fn validate(&self) -> Result<(), ObservationError> {
        match (self.from_seq, self.to_seq, self.opaque.as_deref()) {
            (Some(from), Some(to), None) if from <= to => Ok(()),
            (None, None, Some(value)) => validate_ref(value),
            _ => Err(ObservationError::InvalidCursor),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationBatchContext {
    tenant_id: String,
    source_id: String,
    partition: String,
    contract_ref: String,
    cursor: TransportCursor,
}

impl ObservationBatchContext {
    pub fn new(
        tenant_id: impl Into<String>,
        source_id: impl Into<String>,
        partition: impl Into<String>,
        contract_ref: impl Into<String>,
        cursor: TransportCursor,
    ) -> Result<Self, ObservationError> {
        let context = Self {
            tenant_id: tenant_id.into(),
            source_id: source_id.into(),
            partition: partition.into(),
            contract_ref: contract_ref.into(),
            cursor,
        };
        validate_ref(&context.tenant_id)?;
        validate_ref(&context.source_id)?;
        validate_ref(&context.partition)?;
        validate_ref(&context.contract_ref)?;
        context.cursor.validate()?;
        Ok(context)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformObservationBatch {
    tenant_id: String,
    source_id: String,
    partition: String,
    contract_ref: String,
    cursor: TransportCursor,
    coverage: Coverage,
    events: Vec<ObservationEvent>,
    observed_at_ms: i64,
    retention_until_ms: i64,
    batch_digest: String,
    event_blob_ref: String,
}

impl PlatformObservationBatch {
    pub fn new(
        context: ObservationBatchContext,
        coverage: Coverage,
        events: Vec<ObservationEvent>,
        observed_at_ms: i64,
        retention_until_ms: i64,
    ) -> Result<Self, ObservationError> {
        let mut batch = Self {
            tenant_id: context.tenant_id,
            source_id: context.source_id,
            partition: context.partition,
            contract_ref: context.contract_ref,
            cursor: context.cursor,
            coverage,
            events,
            observed_at_ms,
            retention_until_ms,
            batch_digest: String::new(),
            event_blob_ref: String::new(),
        };
        batch.validate_fields()?;
        batch.event_blob_ref = digest_json(&batch.events);
        batch.batch_digest = batch.content_digest();
        Ok(batch)
    }

    pub fn batch_digest(&self) -> &str {
        &self.batch_digest
    }
    pub fn event_blob_ref(&self) -> &str {
        &self.event_blob_ref
    }
    pub fn coverage(&self) -> &Coverage {
        &self.coverage
    }
    pub fn events(&self) -> &[ObservationEvent] {
        &self.events
    }

    fn validate(&self) -> Result<(), ObservationError> {
        self.validate_fields()?;
        if self.event_blob_ref != digest_json(&self.events)
            || self.batch_digest != self.content_digest()
        {
            return Err(ObservationError::DigestMismatch);
        }
        Ok(())
    }

    fn validate_fields(&self) -> Result<(), ObservationError> {
        validate_ref(&self.tenant_id)?;
        validate_ref(&self.source_id)?;
        validate_ref(&self.partition)?;
        validate_ref(&self.contract_ref)?;
        self.cursor.validate()?;
        self.coverage.validate()?;
        for event in &self.events {
            event.validate()?;
            if matches!(
                event.evidence_kind,
                EvidenceKind::OtelMetric
                    | EvidenceKind::OtelSampledLog
                    | EvidenceKind::OtelSampledSpan
            ) && event.diagnostic.is_none()
            {
                return Err(ObservationError::InvalidDiagnostic);
            }
        }
        if self.observed_at_ms <= 0
            || self
                .events
                .iter()
                .any(|event| event.received_at_ms > self.observed_at_ms)
        {
            return Err(ObservationError::InvalidClock);
        }
        if self.retention_until_ms <= self.observed_at_ms {
            return Err(ObservationError::InvalidRetention);
        }
        Ok(())
    }

    fn content_digest(&self) -> String {
        digest_json(&(
            &self.tenant_id,
            &self.source_id,
            &self.partition,
            &self.contract_ref,
            &self.cursor,
            &self.coverage,
            &self.event_blob_ref,
            self.observed_at_ms,
            self.retention_until_ms,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestReceipt {
    pub batch_digest: String,
    pub event_count: usize,
    pub core_verification_refs: Vec<CoreVerificationReceipt>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoreVerificationReceipt {
    pub source_event_id: String,
    pub receipt_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowProjection {
    tenant_id: String,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
    events: Vec<ObservationEvent>,
    observed_events: Vec<ObservedEvent>,
    coverages: Vec<CoverageEvidence>,
    digest: String,
}

impl WindowProjection {
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn window_start_ms(&self) -> i64 {
        self.window_start_ms
    }
    pub fn window_end_ms(&self) -> i64 {
        self.window_end_ms
    }
    pub fn received_as_of_ms(&self) -> i64 {
        self.received_as_of_ms
    }
    pub fn events(&self) -> &[ObservationEvent] {
        &self.events
    }
    pub fn observed_events(&self) -> &[ObservedEvent] {
        &self.observed_events
    }
    pub fn coverages(&self) -> &[CoverageEvidence] {
        &self.coverages
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageEvidence {
    tenant_id: String,
    source_id: String,
    contract_ref: String,
    partition: String,
    batch_digest: String,
    coverage: Coverage,
    verified_complete_capability: bool,
    evidence_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedEvent {
    tenant_id: String,
    source_id: String,
    contract_ref: String,
    batch_digest: String,
    event: ObservationEvent,
}

impl ObservedEvent {
    pub fn batch_digest(&self) -> &str {
        &self.batch_digest
    }
    pub fn event(&self) -> &ObservationEvent {
        &self.event
    }
    pub fn source_id(&self) -> &str {
        &self.source_id
    }
    pub fn contract_ref(&self) -> &str {
        &self.contract_ref
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundDenominator {
    expected_population: u64,
    batch_digest: String,
    coverage_evidence_digest: String,
}

impl BoundDenominator {
    pub fn expected_population(&self) -> u64 {
        self.expected_population
    }
    pub fn batch_digest(&self) -> &str {
        &self.batch_digest
    }
    pub fn coverage_evidence_digest(&self) -> &str {
        &self.coverage_evidence_digest
    }
}

impl CoverageEvidence {
    pub fn coverage(&self) -> &Coverage {
        &self.coverage
    }
    pub fn source_id(&self) -> &str {
        &self.source_id
    }
    pub fn batch_digest(&self) -> &str {
        &self.batch_digest
    }
    pub fn denominator_for(&self, observed: &ObservedEvent) -> Option<BoundDenominator> {
        if !self.verified_complete_capability
            || self.coverage.state != CoverageState::Complete
            || self.tenant_id != observed.tenant_id
            || self.source_id != observed.source_id
            || self.contract_ref != observed.contract_ref
            || self.batch_digest != observed.batch_digest
            || observed.event.target_system != TargetSystem::Attention
            || observed.event.evidence_kind != EvidenceKind::PlatformAudit
            || self.evidence_digest != coverage_evidence_digest(self)
        {
            return None;
        }
        Some(BoundDenominator {
            expected_population: self.coverage.expected_population?,
            batch_digest: self.batch_digest.clone(),
            coverage_evidence_digest: self.evidence_digest.clone(),
        })
    }
}

fn coverage_evidence_digest(evidence: &CoverageEvidence) -> String {
    digest_json(&(
        &evidence.tenant_id,
        &evidence.source_id,
        &evidence.contract_ref,
        &evidence.partition,
        &evidence.batch_digest,
        &evidence.coverage,
        evidence.verified_complete_capability,
    ))
}

/// Verification of the original Core event belongs to a trusted adapter;
/// a treated projection does not contain enough bytes to re-check its chain.
pub trait CoreChainVerifierPort {
    fn verify(&self, event: &ObservationEvent) -> CoreChainVerification;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreChainVerification {
    Verified { receipt_digest: String },
    Unavailable,
    Invalid,
}

pub trait ObservationRepository {
    fn ingest(
        &mut self,
        batch: PlatformObservationBatch,
    ) -> Result<IngestReceipt, ObservationError>;
    fn list(&mut self, tenant_id: &str) -> Result<Vec<ObservationEvent>, ObservationError>;
    fn window_projection(
        &mut self,
        tenant_id: &str,
        window_start_ms: i64,
        window_end_ms: i64,
        received_as_of_ms: i64,
    ) -> Result<WindowProjection, ObservationError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationError {
    InvalidBatch,
    InvalidRef,
    InvalidClock,
    InvalidDiagnostic,
    InvalidWindow,
    InvalidCoverage,
    InvalidRetention,
    RetentionConflict,
    InvalidSourceContract,
    UnknownSourceContract,
    SourceSequenceModeMismatch,
    SourceSamplingMismatch,
    SourceTargetMismatch,
    CoverageDeclarationMismatch,
    InvalidCursor,
    DigestMismatch,
    ConflictingEvent { event_id: String },
    ConflictingCursor,
    SequenceGap,
    CoreChainUnavailable,
    CoreChainInvalid,
    TenantAccessDenied,
    InlineBlobTooLarge,
    Storage { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationAccess {
    tenant_id: String,
    grant_id: String,
    purpose: String,
}

impl ObservationAccess {
    pub fn new(
        tenant_id: impl Into<String>,
        grant_id: impl Into<String>,
        purpose: impl Into<String>,
    ) -> Result<Self, ObservationError> {
        let access = Self {
            tenant_id: tenant_id.into(),
            grant_id: grant_id.into(),
            purpose: purpose.into(),
        };
        validate_ref(&access.tenant_id)?;
        validate_ref(&access.grant_id)?;
        validate_ref(&access.purpose)?;
        Ok(access)
    }
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }
    pub fn purpose(&self) -> &str {
        &self.purpose
    }
}

/// Inject only an authority backed by the control plane. Constructing an
/// ObservationAccess is not proof that its caller owns that grant.
pub trait ObservationAuthorizationPort {
    fn authorize(&self, access: &ObservationAccess) -> bool;
}

pub struct InMemoryObservationRepository {
    access: ObservationAccess,
    authority: Box<dyn ObservationAuthorizationPort>,
    events: BTreeMap<(String, String, String), ObservationEvent>,
    event_bindings: BTreeMap<(String, String, String), (String, String)>,
    batches: BTreeMap<(String, String, String, String), PlatformObservationBatch>,
    blob_retentions: BTreeMap<(String, String), i64>,
    batch_receipts: BTreeMap<(String, String, String, String), IngestReceipt>,
    verified_batch_capabilities: BTreeMap<(String, String, String, String), bool>,
    core_verifier: Option<Box<dyn CoreChainVerifierPort>>,
    source_registry: ObservationSourceRegistry,
}

impl InMemoryObservationRepository {
    pub fn authorized(
        access: ObservationAccess,
        authority: Box<dyn ObservationAuthorizationPort>,
        source_registry: ObservationSourceRegistry,
    ) -> Result<Self, ObservationError> {
        if !authority.authorize(&access) {
            return Err(ObservationError::TenantAccessDenied);
        }
        Ok(Self {
            access,
            authority,
            events: BTreeMap::new(),
            event_bindings: BTreeMap::new(),
            batches: BTreeMap::new(),
            blob_retentions: BTreeMap::new(),
            batch_receipts: BTreeMap::new(),
            verified_batch_capabilities: BTreeMap::new(),
            core_verifier: None,
            source_registry,
        })
    }

    pub fn authorized_with_core_verifier(
        access: ObservationAccess,
        authority: Box<dyn ObservationAuthorizationPort>,
        source_registry: ObservationSourceRegistry,
        verifier: Box<dyn CoreChainVerifierPort>,
    ) -> Result<Self, ObservationError> {
        let mut repository = Self::authorized(access, authority, source_registry)?;
        repository.core_verifier = Some(verifier);
        Ok(repository)
    }

    fn check_access(&self, tenant_id: &str) -> Result<(), ObservationError> {
        if tenant_id != self.access.tenant_id || !self.authority.authorize(&self.access) {
            return Err(ObservationError::TenantAccessDenied);
        }
        Ok(())
    }
}

impl ObservationRepository for InMemoryObservationRepository {
    fn ingest(
        &mut self,
        batch: PlatformObservationBatch,
    ) -> Result<IngestReceipt, ObservationError> {
        batch.validate()?;
        self.check_access(&batch.tenant_id)?;
        validate_registered_source(&self.source_registry, &batch)?;
        let complete_capability = self
            .source_registry
            .get(&batch.source_id, &batch.contract_ref)
            .expect("validated source contract")
            .complete_coverage_supported;
        validate_inline_blob(&batch.events)?;
        let batch_key = (
            batch.tenant_id.clone(),
            batch.source_id.clone(),
            batch.partition.clone(),
            batch.cursor.key(),
        );
        if let Some(existing) = self.batch_receipts.get(&batch_key) {
            return if existing.batch_digest == batch.batch_digest {
                Ok(existing.clone())
            } else {
                Err(ObservationError::ConflictingCursor)
            };
        }
        let blob_key = (batch.tenant_id.clone(), batch.event_blob_ref.clone());
        if self
            .blob_retentions
            .get(&blob_key)
            .is_some_and(|retention| *retention != batch.retention_until_ms)
        {
            return Err(ObservationError::RetentionConflict);
        }
        let core_verification_refs = verify_core_events(&batch, self.core_verifier.as_deref())?;
        let mut staged = BTreeMap::new();
        for event in &batch.events {
            let key = (
                batch.tenant_id.clone(),
                batch.source_id.clone(),
                event.source_event_id.clone(),
            );
            if self
                .events
                .get(&key)
                .is_some_and(|existing| existing.digest() != event.digest())
                || staged
                    .get(&key)
                    .is_some_and(|existing: &ObservationEvent| existing.digest() != event.digest())
            {
                return Err(ObservationError::ConflictingEvent {
                    event_id: event.source_event_id.clone(),
                });
            }
            staged.insert(key, event.clone());
        }
        for key in staged.keys() {
            self.event_bindings
                .entry(key.clone())
                .or_insert_with(|| (batch.contract_ref.clone(), batch.batch_digest.clone()));
        }
        self.events.extend(staged);
        self.blob_retentions
            .insert(blob_key, batch.retention_until_ms);
        let receipt = IngestReceipt {
            batch_digest: batch.batch_digest.clone(),
            event_count: batch.events.len(),
            core_verification_refs,
        };
        self.batches.insert(batch_key.clone(), batch);
        self.verified_batch_capabilities
            .insert(batch_key.clone(), complete_capability);
        self.batch_receipts.insert(batch_key, receipt.clone());
        Ok(receipt)
    }

    fn list(&mut self, tenant_id: &str) -> Result<Vec<ObservationEvent>, ObservationError> {
        validate_ref(tenant_id)?;
        self.check_access(tenant_id)?;
        Ok(self
            .events
            .iter()
            .filter(|((tenant, _, _), _)| tenant == tenant_id)
            .map(|(_, event)| event.clone())
            .collect())
    }

    fn window_projection(
        &mut self,
        tenant_id: &str,
        window_start_ms: i64,
        window_end_ms: i64,
        received_as_of_ms: i64,
    ) -> Result<WindowProjection, ObservationError> {
        validate_projection_window(tenant_id, window_start_ms, window_end_ms, received_as_of_ms)?;
        self.check_access(tenant_id)?;
        let visible_batch_digests: BTreeSet<_> = self
            .batches
            .iter()
            .filter(|((tenant, _, _, _), batch)| {
                tenant == tenant_id && batch.observed_at_ms <= received_as_of_ms
            })
            .map(|(_, batch)| batch.batch_digest.clone())
            .collect();
        let observed_events = self
            .events
            .iter()
            .filter(|(key, event)| {
                self.event_bindings
                    .get(*key)
                    .is_some_and(|(_, digest)| visible_batch_digests.contains(digest))
                    && key.0 == tenant_id
                    && event.occurred_at_ms >= window_start_ms
                    && event.occurred_at_ms < window_end_ms
                    && event.received_at_ms <= received_as_of_ms
            })
            .map(|(key, event)| {
                let (contract_ref, batch_digest) =
                    self.event_bindings.get(key).expect("filtered key");
                ObservedEvent {
                    tenant_id: key.0.clone(),
                    source_id: key.1.clone(),
                    contract_ref: contract_ref.clone(),
                    batch_digest: batch_digest.clone(),
                    event: event.clone(),
                }
            })
            .collect();
        let coverages = self
            .batches
            .iter()
            .filter(|((tenant, _, _, _), batch)| {
                tenant == tenant_id
                    && batch.coverage.window_start_ms == window_start_ms
                    && batch.coverage.window_end_ms == window_end_ms
                    && batch.observed_at_ms <= received_as_of_ms
            })
            .map(|(key, batch)| {
                make_coverage_evidence(
                    &batch.tenant_id,
                    &batch.source_id,
                    &batch.contract_ref,
                    &batch.partition,
                    &batch.batch_digest,
                    batch.coverage.clone(),
                    *self
                        .verified_batch_capabilities
                        .get(key)
                        .expect("accepted batch"),
                )
            })
            .collect();
        Ok(make_projection(
            tenant_id,
            window_start_ms,
            window_end_ms,
            received_as_of_ms,
            observed_events,
            coverages,
        ))
    }
}

/// PostgreSQL adapter for the U29 durable boundary. The caller must provide a
/// connection limited to Pulso-owned tables; source bank tables are untouched.
pub struct PostgresObservationRepository {
    client: Client,
    access: ObservationAccess,
    authority: Box<dyn ObservationAuthorizationPort>,
    core_verifier: Option<Box<dyn CoreChainVerifierPort>>,
    source_registry: ObservationSourceRegistry,
}

impl PostgresObservationRepository {
    pub fn authorized(
        client: Client,
        access: ObservationAccess,
        authority: Box<dyn ObservationAuthorizationPort>,
        source_registry: ObservationSourceRegistry,
    ) -> Result<Self, ObservationError> {
        if !authority.authorize(&access) {
            return Err(ObservationError::TenantAccessDenied);
        }
        Ok(Self {
            client,
            access,
            authority,
            core_verifier: None,
            source_registry,
        })
    }
    pub fn authorized_with_core_verifier(
        client: Client,
        access: ObservationAccess,
        authority: Box<dyn ObservationAuthorizationPort>,
        source_registry: ObservationSourceRegistry,
        verifier: Box<dyn CoreChainVerifierPort>,
    ) -> Result<Self, ObservationError> {
        let mut repository = Self::authorized(client, access, authority, source_registry)?;
        repository.core_verifier = Some(verifier);
        Ok(repository)
    }
    fn check_access(&self, tenant_id: &str) -> Result<(), ObservationError> {
        if tenant_id != self.access.tenant_id || !self.authority.authorize(&self.access) {
            return Err(ObservationError::TenantAccessDenied);
        }
        Ok(())
    }
}

impl ObservationRepository for PostgresObservationRepository {
    fn ingest(
        &mut self,
        batch: PlatformObservationBatch,
    ) -> Result<IngestReceipt, ObservationError> {
        batch.validate()?;
        self.check_access(&batch.tenant_id)?;
        validate_registered_source(&self.source_registry, &batch)?;
        let complete_capability = self
            .source_registry
            .get(&batch.source_id, &batch.contract_ref)
            .expect("validated source contract")
            .complete_coverage_supported;
        let treated_bytes = validate_inline_blob(&batch.events)?;
        let from_seq = batch
            .cursor
            .from_seq
            .map(|n| i64::try_from(n).map_err(|_| ObservationError::InvalidCursor))
            .transpose()?;
        let to_seq = batch
            .cursor
            .to_seq
            .map(|n| i64::try_from(n).map_err(|_| ObservationError::InvalidCursor))
            .transpose()?;
        let cursor_key = batch.cursor.key();
        let coverage =
            serde_json::to_value(&batch.coverage).map_err(|error| ObservationError::Storage {
                message: error.to_string(),
            })?;
        let mut tx = self.client.transaction().map_err(pg_error)?;
        set_pg_access(&mut tx, &self.access)?;
        tx.execute(
            "INSERT INTO pulso_platform_observation_cursors (tenant_id,source_id,partition_id)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
            &[&batch.tenant_id, &batch.source_id, &batch.partition],
        )
        .map_err(pg_error)?;
        let cursor_row = tx
            .query_one(
                "SELECT last_to_seq FROM pulso_platform_observation_cursors
             WHERE tenant_id=$1 AND source_id=$2 AND partition_id=$3 FOR UPDATE",
                &[&batch.tenant_id, &batch.source_id, &batch.partition],
            )
            .map_err(pg_error)?;
        let last_to_seq: Option<i64> = cursor_row.get(0);
        let existing = tx.query_opt(
            "SELECT batch_digest, core_verification_refs FROM pulso_platform_observation_batches
             WHERE tenant_id=$1 AND source_id=$2 AND partition_id=$3 AND cursor_key=$4",
            &[&batch.tenant_id, &batch.source_id, &batch.partition, &cursor_key],
        ).map_err(pg_error)?;
        if let Some(row) = existing {
            let digest: String = row.get(0);
            if digest != batch.batch_digest {
                return Err(ObservationError::ConflictingCursor);
            }
            let refs_json: serde_json::Value = row.get(1);
            let core_verification_refs =
                serde_json::from_value(refs_json).map_err(|error| ObservationError::Storage {
                    message: error.to_string(),
                })?;
            tx.commit().map_err(pg_error)?;
            return Ok(IngestReceipt {
                batch_digest: digest,
                event_count: batch.events.len(),
                core_verification_refs,
            });
        }
        if let (Some(last), Some(from)) = (last_to_seq, from_seq) {
            if from != last.checked_add(1).ok_or(ObservationError::InvalidCursor)? {
                return Err(ObservationError::SequenceGap);
            }
        }
        let core_verification_refs = verify_core_events(&batch, self.core_verifier.as_deref())?;
        let refs_json = serde_json::to_value(&core_verification_refs).map_err(|error| {
            ObservationError::Storage {
                message: error.to_string(),
            }
        })?;
        {
            let mut blobs = ContentAddressedInlinePostgresBlobStore::new(&mut tx);
            let stored_ref =
                blobs.put_treated(&batch.tenant_id, &treated_bytes, batch.retention_until_ms)?;
            if stored_ref != batch.event_blob_ref {
                return Err(ObservationError::DigestMismatch);
            }
        }
        for event in &batch.events {
            let event_json =
                serde_json::to_value(event).map_err(|error| ObservationError::Storage {
                    message: error.to_string(),
                })?;
            let event_digest = event.digest();
            tx.execute(
                "INSERT INTO pulso_platform_observation_events
                 (tenant_id,source_id,source_event_id,event_digest,event_json,occurred_at_ms,received_at_ms,batch_digest)
                 VALUES ($1,$2,$3,$4,$5::jsonb,$6,$7,$8) ON CONFLICT DO NOTHING",
                &[&batch.tenant_id,&batch.source_id,&event.source_event_id,&event_digest,
                  &event_json,&event.occurred_at_ms,&event.received_at_ms,&batch.batch_digest],
            ).map_err(pg_error)?;
            let stored = tx
                .query_one(
                    "SELECT event_digest FROM pulso_platform_observation_events
                 WHERE tenant_id=$1 AND source_id=$2 AND source_event_id=$3",
                    &[&batch.tenant_id, &batch.source_id, &event.source_event_id],
                )
                .map_err(pg_error)?;
            let stored_digest: String = stored.get(0);
            if stored_digest != event_digest {
                return Err(ObservationError::ConflictingEvent {
                    event_id: event.source_event_id.clone(),
                });
            }
        }
        tx.execute(
            "INSERT INTO pulso_platform_observation_batches
             (tenant_id,source_id,partition_id,cursor_key,from_seq,to_seq,contract_ref,batch_digest,event_blob_ref,retention_until_ms,coverage,event_count,core_verification_refs,max_received_at_ms,verified_complete_capability)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11::jsonb,$12,$13::jsonb,$14,$15)",
            &[&batch.tenant_id,&batch.source_id,&batch.partition,&cursor_key,&from_seq,&to_seq,
              &batch.contract_ref,&batch.batch_digest,&batch.event_blob_ref,&batch.retention_until_ms,
              &coverage,&(batch.events.len() as i64),&refs_json,
              &batch.observed_at_ms,&complete_capability],
        ).map_err(pg_error)?;
        if let Some(to_seq) = to_seq {
            tx.execute(
                "UPDATE pulso_platform_observation_cursors SET last_to_seq=$4,last_batch_digest=$5
                 WHERE tenant_id=$1 AND source_id=$2 AND partition_id=$3",
                &[
                    &batch.tenant_id,
                    &batch.source_id,
                    &batch.partition,
                    &to_seq,
                    &batch.batch_digest,
                ],
            )
            .map_err(pg_error)?;
        }
        tx.commit().map_err(pg_error)?;
        Ok(IngestReceipt {
            batch_digest: batch.batch_digest,
            event_count: batch.events.len(),
            core_verification_refs,
        })
    }

    fn list(&mut self, tenant_id: &str) -> Result<Vec<ObservationEvent>, ObservationError> {
        validate_ref(tenant_id)?;
        self.check_access(tenant_id)?;
        let mut tx = self.client.transaction().map_err(pg_error)?;
        set_pg_access(&mut tx, &self.access)?;
        let result = tx
            .query(
                "SELECT event_json FROM pulso_platform_observation_events WHERE tenant_id=$1
             ORDER BY occurred_at_ms, source_id, source_event_id",
                &[&tenant_id],
            )
            .map_err(pg_error)?
            .into_iter()
            .map(|row| {
                let json: serde_json::Value = row.get(0);
                let event: ObservationEvent =
                    serde_json::from_value(json).map_err(|error| ObservationError::Storage {
                        message: error.to_string(),
                    })?;
                event.validate()?;
                Ok(event)
            })
            .collect();
        tx.commit().map_err(pg_error)?;
        result
    }

    fn window_projection(
        &mut self,
        tenant_id: &str,
        window_start_ms: i64,
        window_end_ms: i64,
        received_as_of_ms: i64,
    ) -> Result<WindowProjection, ObservationError> {
        validate_projection_window(tenant_id, window_start_ms, window_end_ms, received_as_of_ms)?;
        self.check_access(tenant_id)?;
        let mut tx = self.client.transaction().map_err(pg_error)?;
        set_pg_access(&mut tx, &self.access)?;
        let rows = tx
            .query(
                "SELECT e.event_json,e.tenant_id,e.source_id,b.contract_ref,e.batch_digest FROM pulso_platform_observation_events e
             JOIN pulso_platform_observation_batches b
               ON b.tenant_id=e.tenant_id AND b.source_id=e.source_id
              AND b.batch_digest=e.batch_digest
             WHERE e.tenant_id=$1 AND e.occurred_at_ms >= $2 AND e.occurred_at_ms < $3
               AND e.received_at_ms <= $4 AND b.max_received_at_ms <= $4
             ORDER BY e.occurred_at_ms, e.source_id, e.source_event_id",
                &[
                    &tenant_id,
                    &window_start_ms,
                    &window_end_ms,
                    &received_as_of_ms,
                ],
            )
            .map_err(pg_error)?;
        let observed_events = rows
            .into_iter()
            .map(|row| {
                let event: ObservationEvent =
                    serde_json::from_value(row.get(0)).map_err(|error| {
                        ObservationError::Storage {
                            message: error.to_string(),
                        }
                    })?;
                event.validate()?;
                Ok(ObservedEvent {
                    tenant_id: row.get(1),
                    source_id: row.get(2),
                    contract_ref: row.get(3),
                    batch_digest: row.get(4),
                    event,
                })
            })
            .collect::<Result<Vec<_>, ObservationError>>()?;
        let coverage_rows = tx.query(
            "SELECT source_id,partition_id,batch_digest,coverage,contract_ref,verified_complete_capability FROM pulso_platform_observation_batches
             WHERE tenant_id=$1 AND (coverage->>'window_start_ms')::bigint=$2
               AND (coverage->>'window_end_ms')::bigint=$3 AND max_received_at_ms <= $4
             ORDER BY source_id,partition_id,cursor_key",
            &[&tenant_id,&window_start_ms,&window_end_ms,&received_as_of_ms],
        ).map_err(pg_error)?;
        let coverages = coverage_rows
            .into_iter()
            .map(|row| {
                let value: serde_json::Value = row.get(3);
                let coverage: Coverage =
                    serde_json::from_value(value).map_err(|error| ObservationError::Storage {
                        message: error.to_string(),
                    })?;
                coverage.validate()?;
                Ok(make_coverage_evidence(
                    tenant_id,
                    &row.get::<_, String>(0),
                    &row.get::<_, String>(4),
                    &row.get::<_, String>(1),
                    &row.get::<_, String>(2),
                    coverage,
                    row.get(5),
                ))
            })
            .collect::<Result<Vec<_>, ObservationError>>()?;
        tx.commit().map_err(pg_error)?;
        Ok(make_projection(
            tenant_id,
            window_start_ms,
            window_end_ms,
            received_as_of_ms,
            observed_events,
            coverages,
        ))
    }
}

fn set_pg_access(
    tx: &mut Transaction<'_>,
    access: &ObservationAccess,
) -> Result<(), ObservationError> {
    tx.query_one(
        "SELECT set_config('pulso.observation_grant',$1,true),
                set_config('pulso.observation_purpose',$2,true)",
        &[&access.grant_id, &access.purpose],
    )
    .map_err(pg_error)?;
    Ok(())
}

fn pg_error(error: postgres::Error) -> ObservationError {
    ObservationError::Storage {
        message: error.to_string(),
    }
}

fn validate_registered_source(
    registry: &ObservationSourceRegistry,
    batch: &PlatformObservationBatch,
) -> Result<(), ObservationError> {
    let contract = registry
        .get(&batch.source_id, &batch.contract_ref)
        .ok_or(ObservationError::UnknownSourceContract)?;
    contract.validate_batch(batch)
}

fn validate_projection_window(
    tenant_id: &str,
    start: i64,
    end: i64,
    as_of: i64,
) -> Result<(), ObservationError> {
    validate_ref(tenant_id)?;
    if start <= 0 || end <= start || as_of <= 0 {
        return Err(ObservationError::InvalidWindow);
    }
    Ok(())
}

fn make_projection(
    tenant_id: &str,
    start: i64,
    end: i64,
    received_as_of_ms: i64,
    observed_events: Vec<ObservedEvent>,
    coverages: Vec<CoverageEvidence>,
) -> WindowProjection {
    let coverage_identity: Vec<_> = coverages
        .iter()
        .map(|item| {
            (
                &item.tenant_id,
                &item.source_id,
                &item.contract_ref,
                &item.partition,
                &item.batch_digest,
                &item.coverage,
                item.verified_complete_capability,
                &item.evidence_digest,
            )
        })
        .collect();
    let digest = digest_json(&(
        tenant_id,
        start,
        end,
        received_as_of_ms,
        &observed_events
            .iter()
            .map(|item| {
                (
                    &item.tenant_id,
                    &item.source_id,
                    &item.contract_ref,
                    &item.batch_digest,
                    &item.event,
                )
            })
            .collect::<Vec<_>>(),
        coverage_identity,
    ));
    let events = observed_events
        .iter()
        .map(|item| item.event.clone())
        .collect();
    WindowProjection {
        tenant_id: tenant_id.to_owned(),
        window_start_ms: start,
        window_end_ms: end,
        received_as_of_ms,
        events,
        observed_events,
        coverages,
        digest,
    }
}

fn make_coverage_evidence(
    tenant_id: &str,
    source_id: &str,
    contract_ref: &str,
    partition: &str,
    batch_digest: &str,
    coverage: Coverage,
    verified_complete_capability: bool,
) -> CoverageEvidence {
    let mut evidence = CoverageEvidence {
        tenant_id: tenant_id.into(),
        source_id: source_id.into(),
        contract_ref: contract_ref.into(),
        partition: partition.into(),
        batch_digest: batch_digest.into(),
        coverage,
        verified_complete_capability,
        evidence_digest: String::new(),
    };
    evidence.evidence_digest = coverage_evidence_digest(&evidence);
    evidence
}

pub trait ObservationBlobStore {
    /// Stores only already-treated, bounded bytes. Returns a content digest.
    fn put_treated(
        &mut self,
        tenant_id: &str,
        bytes: &[u8],
        retention_until_ms: i64,
    ) -> Result<String, ObservationError>;
}

/// Bounded content-addressed blob storage inside the same PG transaction as
/// the U29 batch/cursor. Larger batches require a future authorized blob port.
pub struct ContentAddressedInlinePostgresBlobStore<'tx, 'client> {
    tx: &'tx mut Transaction<'client>,
}

impl<'tx, 'client> ContentAddressedInlinePostgresBlobStore<'tx, 'client> {
    pub fn new(tx: &'tx mut Transaction<'client>) -> Self {
        Self { tx }
    }
}

impl ObservationBlobStore for ContentAddressedInlinePostgresBlobStore<'_, '_> {
    fn put_treated(
        &mut self,
        tenant_id: &str,
        bytes: &[u8],
        retention_until_ms: i64,
    ) -> Result<String, ObservationError> {
        validate_ref(tenant_id)?;
        if bytes.len() > INLINE_TREATED_BLOB_MAX_BYTES {
            return Err(ObservationError::InlineBlobTooLarge);
        }
        if retention_until_ms <= 0 {
            return Err(ObservationError::InvalidRetention);
        }
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        self.tx
            .execute(
                "INSERT INTO pulso_platform_observation_blobs
             (tenant_id,digest,treated_bytes,retention_until_ms) VALUES ($1,$2,$3,$4)
             ON CONFLICT DO NOTHING",
                &[&tenant_id, &digest, &bytes, &retention_until_ms],
            )
            .map_err(pg_error)?;
        let row = self.tx.query_one(
            "SELECT treated_bytes,retention_until_ms FROM pulso_platform_observation_blobs WHERE tenant_id=$1 AND digest=$2",
            &[&tenant_id,&digest],
        ).map_err(pg_error)?;
        let stored: Vec<u8> = row.get(0);
        if stored != bytes {
            return Err(ObservationError::DigestMismatch);
        }
        let stored_retention: i64 = row.get(1);
        if stored_retention != retention_until_ms {
            return Err(ObservationError::RetentionConflict);
        }
        Ok(digest)
    }
}

fn validate_inline_blob(events: &[ObservationEvent]) -> Result<Vec<u8>, ObservationError> {
    let bytes = serde_json::to_vec(events).map_err(|error| ObservationError::Storage {
        message: error.to_string(),
    })?;
    if bytes.len() > INLINE_TREATED_BLOB_MAX_BYTES {
        return Err(ObservationError::InlineBlobTooLarge);
    }
    Ok(bytes)
}

fn verify_core_events(
    batch: &PlatformObservationBatch,
    verifier: Option<&dyn CoreChainVerifierPort>,
) -> Result<Vec<CoreVerificationReceipt>, ObservationError> {
    let mut refs = Vec::new();
    for event in &batch.events {
        if event.evidence_kind != EvidenceKind::CoreAudit {
            continue;
        }
        if event.source_event_digest.is_none() {
            return Err(ObservationError::CoreChainUnavailable);
        }
        match verifier
            .map(|verifier| verifier.verify(event))
            .unwrap_or(CoreChainVerification::Unavailable)
        {
            CoreChainVerification::Verified { receipt_digest } if is_digest(&receipt_digest) => {
                refs.push(CoreVerificationReceipt {
                    source_event_id: event.source_event_id.clone(),
                    receipt_digest,
                })
            }
            CoreChainVerification::Invalid => return Err(ObservationError::CoreChainInvalid),
            _ => return Err(ObservationError::CoreChainUnavailable),
        }
    }
    Ok(refs)
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn validate_ref(value: &str) -> Result<(), ObservationError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
    {
        Err(ObservationError::InvalidRef)
    } else {
        Ok(())
    }
}

fn digest_json(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("typed observation JSON serialization");
    format!("sha256:{:x}", Sha256::digest(bytes))
}
