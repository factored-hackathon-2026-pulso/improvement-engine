//! Deterministic, provenance-preserving metrics over U29 projections.
//!
//! U30 consumes treated `WindowProjection` values only. It never reads a
//! batch, blob, cursor, or database row directly, and it never rebuilds a
//! coverage declaration from the events it happens to see. A metric is only
//! authoritative when the deployment's immutable sensor registry binds its
//! metric/layer/population semantics to one trusted source contract.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::platform_observations::{
    EvidenceKind, InteractionEventKind, Layer, TargetSystem, WindowProjection,
};

/// V3's initial handoff-rate denominator: distinct eligible attention source
/// runs, not decision-tree goals. The source contract must attest this grain.
pub const ATTENTION_SOURCE_RUNS_POPULATION_REF: &str = "attention_source_runs";
pub const ATTENTION_RUN_HANDOFF_RATE_METRIC_ID: &str = "attention_run_handoff_rate";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LayerMetricSpec {
    metric_id: String,
    metric_version: u16,
    layer: Layer,
    /// Canonical eligible-population semantics, not merely a display label.
    population_ref: String,
}

impl LayerMetricSpec {
    /// Builds the initial V3 attention handoff rate. Its denominator grain is
    /// deliberately fixed; the trusted source contract must attest that the
    /// expected population counts distinct eligible attention source runs.
    pub fn handoff_rate(
        metric_id: impl Into<String>,
        metric_version: u16,
        layer: Layer,
        population_ref: impl Into<String>,
    ) -> Result<Self, PlatformSensorError> {
        let spec = Self {
            metric_id: metric_id.into(),
            metric_version,
            layer,
            population_ref: population_ref.into(),
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }
    pub fn metric_version(&self) -> u16 {
        self.metric_version
    }
    pub fn layer(&self) -> Layer {
        self.layer
    }
    pub fn population_ref(&self) -> &str {
        &self.population_ref
    }

    fn validate(&self) -> Result<(), PlatformSensorError> {
        if !valid_identifier(&self.metric_id)
            || self.metric_id != ATTENTION_RUN_HANDOFF_RATE_METRIC_ID
            || self.metric_version == 0
            || !valid_ref(&self.population_ref)
            || self.population_ref != ATTENTION_SOURCE_RUNS_POPULATION_REF
            || self.layer == Layer::Unknown
        {
            return Err(PlatformSensorError::InvalidSpec);
        }
        Ok(())
    }
}

/// Immutable composition-root binding for the only source contract allowed to
/// supply an authoritative rate for one metric specification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TrustedLayerMetricMapping {
    spec: LayerMetricSpec,
    source_id: String,
    contract_ref: String,
    digest: String,
}

impl TrustedLayerMetricMapping {
    fn handoff_rate(
        spec: LayerMetricSpec,
        source_id: impl Into<String>,
        contract_ref: impl Into<String>,
    ) -> Result<Self, PlatformSensorError> {
        spec.validate()?;
        let mut mapping = Self {
            spec,
            source_id: source_id.into(),
            contract_ref: contract_ref.into(),
            digest: String::new(),
        };
        if !valid_ref(&mapping.source_id) || !valid_ref(&mapping.contract_ref) {
            return Err(PlatformSensorError::InvalidMapping);
        }
        mapping.digest = mapping_digest(&mapping);
        Ok(mapping)
    }

    fn matches(&self, spec: &LayerMetricSpec) -> bool {
        self.spec == *spec
    }

    fn identity(&self) -> (&str, u16, Layer, &str) {
        (
            self.spec.metric_id(),
            self.spec.metric_version(),
            self.spec.layer(),
            self.spec.population_ref(),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformSignalStatus {
    Measured {
        numerator: u64,
        denominator: u64,
        missing: u64,
    },
    InsufficientEvidence,
    AmbiguousCoverage,
    InconsistentEvidence,
}

/// A sealed signal carries refs/digests only, never raw observation payloads
/// and never an opportunity or causal assertion. It can only be created by a
/// configured [`PlatformLayerSensor`].
///
/// ```compile_fail
/// use improvement_engine_core::platform_observations::Layer;
/// use improvement_engine_core::platform_sensor::{PlatformLayerSignal, PlatformSignalStatus};
/// let _forged = PlatformLayerSignal { tenant_id: "bank_demo".to_owned(), metric_id: "attention_run_handoff_rate".to_owned(), metric_version: 1, layer: Layer::Tree, population_ref: "attention_source_runs".to_owned(), status: PlatformSignalStatus::InsufficientEvidence, window_start_ms: 1, window_end_ms: 2, received_as_of_ms: 2, projection_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned(), source_id: None, contract_ref: None, batch_digest: None, coverage_evidence_digest: None, metric_mapping_digest: None, digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned() };
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformLayerSignal {
    tenant_id: String,
    metric_id: String,
    metric_version: u16,
    layer: Layer,
    population_ref: String,
    status: PlatformSignalStatus,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
    projection_digest: String,
    source_id: Option<String>,
    contract_ref: Option<String>,
    batch_digest: Option<String>,
    coverage_evidence_digest: Option<String>,
    metric_mapping_digest: Option<String>,
    mapping_resolution_digest: String,
    digest: String,
}

impl PlatformLayerSignal {
    /// Tenant scope bound by U29's repository-produced projection.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }
    pub fn metric_version(&self) -> u16 {
        self.metric_version
    }
    pub fn layer(&self) -> Layer {
        self.layer
    }
    pub fn population_ref(&self) -> &str {
        &self.population_ref
    }
    pub fn status(&self) -> &PlatformSignalStatus {
        &self.status
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
    pub fn projection_digest(&self) -> &str {
        &self.projection_digest
    }
    pub fn source_id(&self) -> Option<&str> {
        self.source_id.as_deref()
    }
    pub fn contract_ref(&self) -> Option<&str> {
        self.contract_ref.as_deref()
    }
    pub fn batch_digest(&self) -> Option<&str> {
        self.batch_digest.as_deref()
    }
    pub fn coverage_evidence_digest(&self) -> Option<&str> {
        self.coverage_evidence_digest.as_deref()
    }
    /// Commitment of the trusted mapping that made this metric authoritative.
    /// It remains present even if no eligible evidence was found.
    pub fn metric_mapping_digest(&self) -> Option<&str> {
        self.metric_mapping_digest.as_deref()
    }
    /// Non-optional receipt that binds every outcome to the exact metric
    /// request, projection, and trusted registry state. For an unmapped
    /// request it records the deliberate `missing` resolution, rather than
    /// leaving a provenance gap.
    pub fn mapping_resolution_digest(&self) -> &str {
        &self.mapping_resolution_digest
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformSensorError {
    InvalidSpec,
    InvalidMapping,
    DuplicateMapping,
}

/// Closed Product-platform capability profile used to avoid proposing work
/// against features that the selected Product phase cannot execute.
///
/// This is a detector input, not an authorization grant. The profile contains
/// only reviewed capability names and never accepts arbitrary source fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformCapabilityProfile {
    version: String,
    capabilities: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformCapabilityAssessment {
    profile_version: String,
    capability: String,
    supported: bool,
}

impl PlatformCapabilityProfile {
    /// Product Phase 1 has no `tool_call` executor in its declared surface.
    pub fn product_phase_one() -> Self {
        Self {
            version: "phase-1".to_owned(),
            capabilities: BTreeSet::new(),
        }
    }

    /// Acceptance fixture for the documented capability superset.
    pub fn product_superset_0_5_1() -> Self {
        Self {
            version: "0.5.1".to_owned(),
            capabilities: BTreeSet::from(["tool_call".to_owned()]),
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn assess_tool_call(&self) -> PlatformCapabilityAssessment {
        PlatformCapabilityAssessment {
            profile_version: self.version.clone(),
            capability: "tool_call".to_owned(),
            supported: self.capabilities.contains("tool_call"),
        }
    }
}

impl PlatformCapabilityAssessment {
    pub fn status(&self) -> &'static str {
        if self.supported {
            "supported"
        } else {
            "unsupported"
        }
    }

    pub fn profile_version(&self) -> &str {
        &self.profile_version
    }

    pub fn capability(&self) -> &str {
        &self.capability
    }

    pub fn is_supported(&self) -> bool {
        self.supported
    }
}

/// Minimal case-lifecycle projection needed to test the PL-07 chain rule.
/// IDs remain inside the deterministic calculation and are never copied into
/// the resulting finding.
#[derive(Clone, Eq, PartialEq)]
pub struct PlatformCaseClosure {
    case_id: String,
    previous_case_id: Option<String>,
    previous_case_field_available: bool,
    is_failure: Option<bool>,
    close_reason: Option<String>,
    closed_at_ms: Option<i64>,
    available_at_ms: Option<i64>,
    observed_as_of_ms: Option<i64>,
    source_ref: Option<String>,
    source_digest: Option<String>,
    simulator: Option<bool>,
    evidence_kind: Option<EvidenceKind>,
}

impl PlatformCaseClosure {
    pub fn new(
        case_id: impl Into<String>,
        previous_case_id: Option<&str>,
        is_failure: Option<bool>,
        close_reason: Option<&str>,
    ) -> Self {
        Self {
            case_id: case_id.into(),
            previous_case_id: previous_case_id.map(str::to_owned),
            previous_case_field_available: true,
            is_failure,
            close_reason: close_reason.map(str::to_owned),
            closed_at_ms: None,
            available_at_ms: None,
            observed_as_of_ms: None,
            source_ref: None,
            source_digest: None,
            simulator: None,
            evidence_kind: None,
        }
    }

    /// Binds the row to its immutable source snapshot, closure time, and
    /// explicit observation cutoff. Unknown simulator/evidence metadata is
    /// retained as unknown and makes the population ineligible.
    pub fn with_snapshot_provenance(
        mut self,
        source_ref: Option<&str>,
        source_digest: Option<&str>,
        closed_at_ms: Option<i64>,
        observed_as_of_ms: Option<i64>,
        simulator: Option<bool>,
        evidence_kind: Option<EvidenceKind>,
    ) -> Self {
        self.source_ref = source_ref.map(str::to_owned);
        self.source_digest = source_digest.map(str::to_owned);
        self.closed_at_ms = closed_at_ms;
        self.observed_as_of_ms = observed_as_of_ms;
        self.simulator = simulator;
        self.evidence_kind = evidence_kind;
        self
    }

    /// Records when this closure became available to the platform snapshot.
    /// The detector rejects events arriving after its as-of cutoff.
    pub fn with_available_at_ms(mut self, available_at_ms: Option<i64>) -> Self {
        self.available_at_ms = available_at_ms;
        self
    }

    /// Represents a projection whose schema omitted `previous_case_id`, which
    /// must not be confused with an explicit null (a root case).
    pub fn without_previous_case_field(mut self) -> Self {
        self.previous_case_field_available = false;
        self
    }
}

impl fmt::Debug for PlatformCaseClosure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformCaseClosure")
            .field("case_id", &"[REDACTED]")
            .field(
                "previous_case_id",
                &self.previous_case_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "previous_case_field_available",
                &self.previous_case_field_available,
            )
            .field("is_failure", &self.is_failure)
            .field(
                "close_reason",
                &self.close_reason.as_ref().map(|_| "[REDACTED]"),
            )
            .field("closed_at_ms", &self.closed_at_ms)
            .field("available_at_ms", &self.available_at_ms)
            .field("observed_as_of_ms", &self.observed_as_of_ms)
            .field(
                "source_ref",
                &self.source_ref.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "source_digest",
                &self.source_digest.as_ref().map(|_| "[REDACTED]"),
            )
            .field("simulator", &self.simulator)
            .field("evidence_kind", &self.evidence_kind)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PlatformCustomerPopulationRow {
    customer_id: String,
    simulator: Option<bool>,
}

impl fmt::Debug for PlatformCustomerPopulationRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformCustomerPopulationRow")
            .field("customer_id", &"[REDACTED]")
            .field("simulator", &self.simulator)
            .finish()
    }
}

impl PlatformCustomerPopulationRow {
    pub fn new(customer_id: impl Into<String>, simulator: Option<bool>) -> Self {
        Self {
            customer_id: customer_id.into(),
            simulator,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformPopulationStatus {
    Measured,
    InsufficientEvidence,
}

/// Non-ratio population summary. Simulator rows are excluded before counting,
/// so callers cannot mistake them for eligible customers in a denominator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformPopulationAssessment {
    status: PlatformPopulationStatus,
    eligible_customer_count: Option<u64>,
    team_generated_excluded_count: u64,
    excluded_reason_counts: BTreeMap<String, u64>,
    missing_fields: Vec<&'static str>,
}

impl PlatformPopulationAssessment {
    pub fn status(&self) -> &PlatformPopulationStatus {
        &self.status
    }

    pub fn eligible_customer_count(&self) -> Option<u64> {
        self.eligible_customer_count
    }

    pub fn team_generated_excluded_count(&self) -> u64 {
        self.team_generated_excluded_count
    }

    pub fn missing_fields(&self) -> &[&'static str] {
        &self.missing_fields
    }

    pub fn is_publishable(&self) -> bool {
        false
    }
}

/// Treated SLA row. `observed_at_ms` is the upstream projection's supplied
/// observation cutoff for the snapshot and is required to match across rows.
/// It is not necessarily the case's resolution or closure time. The source
/// reference and content digest bind the measurement to its immutable input;
/// no case ID is returned and no regulatory conclusion is implied.
#[derive(Clone, Eq, PartialEq)]
pub struct PlatformSlaCase {
    case_id: String,
    sla_due_at_ms: Option<i64>,
    observed_at_ms: Option<i64>,
    source_ref: Option<String>,
    source_digest: Option<String>,
    simulator: Option<bool>,
    evidence_kind: Option<EvidenceKind>,
}

impl PlatformSlaCase {
    pub fn new(
        case_id: impl Into<String>,
        sla_due_at_ms: Option<i64>,
        observed_at_ms: Option<i64>,
        source_ref: Option<&str>,
        source_digest: Option<&str>,
    ) -> Self {
        Self {
            case_id: case_id.into(),
            sla_due_at_ms,
            observed_at_ms,
            source_ref: source_ref.map(str::to_owned),
            source_digest: source_digest.map(str::to_owned),
            simulator: None,
            evidence_kind: None,
        }
    }

    pub fn with_evidence_metadata(
        mut self,
        simulator: Option<bool>,
        evidence_kind: Option<EvidenceKind>,
    ) -> Self {
        self.simulator = simulator;
        self.evidence_kind = evidence_kind;
        self
    }
}

impl fmt::Debug for PlatformSlaCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformSlaCase")
            .field("case_id", &"[REDACTED]")
            .field("sla_due_at_ms", &self.sla_due_at_ms)
            .field("observed_at_ms", &self.observed_at_ms)
            .field(
                "source_ref",
                &self.source_ref.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "source_digest",
                &self.source_digest.as_ref().map(|_| "[REDACTED]"),
            )
            .field("simulator", &self.simulator)
            .field("evidence_kind", &self.evidence_kind)
            .finish()
    }
}

/// Descriptive, non-publishable result of a platform sensor calculation.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct PlatformSensorFinding {
    status: PlatformSignalStatus,
    missing_fields: Vec<&'static str>,
    excluded_reason_counts: BTreeMap<String, u64>,
    source_ref: Option<String>,
    source_digest: Option<String>,
    observed_as_of_ms: Option<i64>,
    measurement_digest: Option<String>,
    statement: String,
}

impl fmt::Debug for PlatformSensorFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformSensorFinding")
            .field("status", &self.status)
            .field("missing_fields", &self.missing_fields)
            .field("excluded_reason_counts", &self.excluded_reason_counts)
            .field(
                "source_ref",
                &self.source_ref.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "source_digest",
                &self.source_digest.as_ref().map(|_| "[REDACTED]"),
            )
            .field("observed_as_of_ms", &self.observed_as_of_ms)
            .field(
                "measurement_digest",
                &self.measurement_digest.as_ref().map(|_| "[REDACTED]"),
            )
            .field("statement", &self.statement)
            .finish()
    }
}

impl PlatformSensorFinding {
    pub fn status(&self) -> &PlatformSignalStatus {
        &self.status
    }

    pub fn missing_fields(&self) -> &[&'static str] {
        &self.missing_fields
    }

    pub fn excluded_reason_count(&self, reason: &str) -> u64 {
        self.excluded_reason_counts
            .get(reason)
            .copied()
            .unwrap_or(0)
    }

    pub fn source_ref(&self) -> Option<&str> {
        self.source_ref.as_deref()
    }

    pub fn source_digest(&self) -> Option<&str> {
        self.source_digest.as_deref()
    }

    /// The common observation cutoff used by descriptive SLA measurements.
    /// Other sensor findings and insufficient SLA results have no as-of value.
    pub fn observed_as_of_ms(&self) -> Option<i64> {
        self.observed_as_of_ms
    }

    /// A deterministic commitment to the exact eligible values and cutoff
    /// used for this aggregate; it is not an identifier or a source signature.
    pub fn measurement_digest(&self) -> Option<&str> {
        self.measurement_digest.as_deref()
    }

    pub fn statement(&self) -> &str {
        &self.statement
    }

    /// Platform sensors only emit evidence; proposal compilation and publish
    /// authority are separate stages and are never implied by a finding.
    pub fn is_publishable(&self) -> bool {
        false
    }
}

/// Explicit gate for the platform-to-Core handoff. It cannot publish anything.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformDependencyCheck {
    reason: &'static str,
    dependency_state: &'static str,
}

impl PlatformDependencyCheck {
    pub fn reason(&self) -> &'static str {
        self.reason
    }

    pub fn dependency_state(&self) -> &'static str {
        self.dependency_state
    }

    pub fn is_publishable(&self) -> bool {
        false
    }
}

impl PlatformLayerSensor {
    /// Measures each unbranched case chain by its terminal closure only. A
    /// terminal customer-unresponsive close is an exclusion, not a default
    /// failure. Missing fields, dangling/cyclic links, or branched chains fail
    /// closed as insufficient evidence.
    pub fn assess_case_closures(cases: &[PlatformCaseClosure]) -> PlatformSensorFinding {
        let mut missing_fields = BTreeSet::new();
        let mut all_ids = BTreeSet::new();
        let mut by_id = BTreeMap::new();
        let mut eligible = Vec::new();
        let mut source_ref: Option<&str> = None;
        let mut source_digest: Option<&str> = None;
        let mut observed_as_of_ms: Option<i64> = None;
        let mut excluded_reason_counts = BTreeMap::<String, u64>::new();
        for case in cases {
            if case.case_id.is_empty() || !all_ids.insert(case.case_id.as_str()) {
                missing_fields.insert("unique_case_id");
            }
            match case.source_ref.as_deref().filter(|value| valid_ref(value)) {
                Some(value) if source_ref.is_none() || source_ref == Some(value) => {
                    source_ref = Some(value);
                }
                _ => {
                    missing_fields.insert("source_ref");
                }
            }
            match case
                .source_digest
                .as_deref()
                .filter(|value| valid_sha256_ref(value))
            {
                Some(value) if source_digest.is_none() || source_digest == Some(value) => {
                    source_digest = Some(value);
                }
                _ => {
                    missing_fields.insert("source_digest");
                }
            }
            match case.observed_as_of_ms {
                Some(value) if observed_as_of_ms.is_none() || observed_as_of_ms == Some(value) => {
                    observed_as_of_ms = Some(value);
                }
                Some(_) => {
                    missing_fields.insert("consistent_observed_as_of");
                }
                None => {
                    missing_fields.insert("observed_as_of");
                }
            }
            match case.closed_at_ms {
                Some(closed_at) => match case.observed_as_of_ms {
                    Some(cutoff) if closed_at <= cutoff => {}
                    Some(_) => {
                        missing_fields.insert("closed_by_observed_as_of");
                    }
                    None => {
                        missing_fields.insert("observed_as_of");
                    }
                },
                None => {
                    missing_fields.insert("closed_at");
                }
            }
            match (case.available_at_ms, case.observed_as_of_ms) {
                (Some(available_at), Some(cutoff)) if available_at <= cutoff => {}
                (Some(_), Some(_)) => {
                    missing_fields.insert("available_by_observed_as_of");
                }
                (None, _) => {
                    missing_fields.insert("available_at");
                }
                (_, None) => {
                    missing_fields.insert("observed_as_of");
                }
            }
            match (case.simulator, case.evidence_kind) {
                (Some(_), Some(_)) => {}
                (None, _) => {
                    missing_fields.insert("cases.simulator");
                }
                (_, None) => {
                    missing_fields.insert("evidence_kind");
                }
            }
            if case.simulator == Some(true) {
                *excluded_reason_counts
                    .entry("team_generated".to_owned())
                    .or_default() += 1;
            }
            if case
                .evidence_kind
                .is_some_and(|kind| kind != EvidenceKind::PlatformAudit)
            {
                *excluded_reason_counts
                    .entry("non_platform_audit_evidence".to_owned())
                    .or_default() += 1;
            }
            if case.simulator != Some(false)
                || case.evidence_kind != Some(EvidenceKind::PlatformAudit)
            {
                continue;
            }
            if !case.previous_case_field_available {
                missing_fields.insert("previous_case_id");
            }
            if case.is_failure.is_none() {
                missing_fields.insert("is_failure");
            }
            if case.close_reason.as_deref().is_none_or(str::is_empty) {
                missing_fields.insert("close_reason");
            }
            by_id.insert(case.case_id.as_str(), case);
            eligible.push(case);
        }
        if cases.is_empty() {
            missing_fields.insert("case_closures");
        }
        if !missing_fields.is_empty() {
            return insufficient_finding_with_exclusions(
                missing_fields.into_iter().collect(),
                excluded_reason_counts,
            );
        }
        if eligible.is_empty() {
            return insufficient_finding_with_exclusions(
                vec!["eligible_case_closures"],
                excluded_reason_counts,
            );
        }

        let mut child_counts = BTreeMap::<&str, usize>::new();
        for case in eligible.iter().copied() {
            if let Some(previous_id) = case.previous_case_id.as_deref() {
                *child_counts.entry(previous_id).or_default() += 1;
            }
        }
        if child_counts.values().any(|count| *count > 1) {
            return insufficient_finding_with_exclusions(
                vec!["unbranched_previous_case_id"],
                excluded_reason_counts,
            );
        }

        let mut root_by_id = BTreeMap::<&str, &str>::new();
        for case in eligible.iter().copied() {
            if root_by_id.contains_key(case.case_id.as_str()) {
                continue;
            }
            let mut cursor = case;
            let mut path = Vec::new();
            let mut visited = BTreeSet::new();
            let root = loop {
                let current_id = cursor.case_id.as_str();
                if let Some(root) = root_by_id.get(current_id).copied() {
                    break Some(root);
                }
                if !visited.insert(current_id) {
                    missing_fields.insert("acyclic_previous_case_id");
                    break None;
                }
                path.push(cursor);
                match cursor.previous_case_id.as_deref() {
                    None => break Some(current_id),
                    Some(previous_id) => match by_id.get(previous_id).copied() {
                        Some(previous) => cursor = previous,
                        None => {
                            missing_fields.insert("resolved_previous_case_id");
                            break None;
                        }
                    },
                }
            };
            if let Some(root) = root {
                for path_case in path {
                    root_by_id.insert(path_case.case_id.as_str(), root);
                }
            } else {
                break;
            }
        }
        if !missing_fields.is_empty() {
            return insufficient_finding_with_exclusions(
                missing_fields.into_iter().collect(),
                excluded_reason_counts,
            );
        }

        let mut terminal_by_root = BTreeMap::<&str, &PlatformCaseClosure>::new();
        for case in eligible.iter().copied() {
            if child_counts
                .get(case.case_id.as_str())
                .copied()
                .unwrap_or(0)
                == 0
            {
                let root = root_by_id[case.case_id.as_str()];
                if terminal_by_root.insert(root, case).is_some() {
                    missing_fields.insert("single_terminal_case_per_chain");
                }
            }
        }
        if !missing_fields.is_empty() || terminal_by_root.is_empty() {
            if terminal_by_root.is_empty() {
                missing_fields.insert("terminal_case");
            }
            return insufficient_finding_with_exclusions(
                missing_fields.into_iter().collect(),
                excluded_reason_counts,
            );
        }

        let numerator = terminal_by_root
            .values()
            .filter(|case| {
                case.is_failure == Some(true)
                    && case.close_reason.as_deref() != Some("customer_unresponsive")
            })
            .count() as u64;
        let denominator = terminal_by_root.len() as u64;
        let unresponsive_excluded = terminal_by_root
            .values()
            .filter(|case| case.close_reason.as_deref() == Some("customer_unresponsive"))
            .count() as u64;
        if unresponsive_excluded > 0 {
            excluded_reason_counts
                .insert("customer_unresponsive".to_owned(), unresponsive_excluded);
        }
        let mut result = finding(
            PlatformSignalStatus::Measured {
                numerator,
                denominator,
                missing: 0,
            },
            Vec::new(),
            excluded_reason_counts,
            source_ref.map(str::to_owned),
            source_digest.map(str::to_owned),
            format!(
                "As of the supplied observation cutoff, {numerator} failed case chains were observed among {denominator} eligible chains."
            ),
        );
        result.observed_as_of_ms = observed_as_of_ms;
        let mut committed_rows = eligible
            .iter()
            .map(|case| {
                (
                    case.case_id.as_str(),
                    case.previous_case_id.as_deref(),
                    case.is_failure,
                    case.close_reason.as_deref(),
                    case.closed_at_ms,
                    case.available_at_ms,
                )
            })
            .collect::<Vec<_>>();
        committed_rows.sort_by_key(|row| row.0);
        result.measurement_digest = Some(measurement_digest(&(
            "platform_case_closure_v1",
            result.source_ref.as_deref(),
            result.source_digest.as_deref(),
            observed_as_of_ms,
            committed_rows,
            numerator,
            denominator,
            &result.excluded_reason_counts,
        )));
        result
    }

    /// Removes team-generated simulator customers before the eligible
    /// population is counted. Unknown simulator flags make that count unsafe.
    pub fn assess_customer_population(
        customers: &[PlatformCustomerPopulationRow],
    ) -> PlatformPopulationAssessment {
        let mut missing_fields = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut denominator = 0_u64;
        let mut excluded = 0_u64;
        for customer in customers {
            if customer.customer_id.is_empty() || !ids.insert(customer.customer_id.as_str()) {
                missing_fields.insert("unique_customer_id");
            }
            match customer.simulator {
                Some(true) => excluded += 1,
                Some(false) => denominator += 1,
                None => {
                    missing_fields.insert("customers.simulator");
                }
            }
        }
        if customers.is_empty() {
            missing_fields.insert("customers");
        }
        if !missing_fields.is_empty() {
            let excluded_reason_counts = team_generated_exclusion_counts(excluded);
            return PlatformPopulationAssessment {
                status: PlatformPopulationStatus::InsufficientEvidence,
                eligible_customer_count: None,
                team_generated_excluded_count: excluded,
                excluded_reason_counts,
                missing_fields: missing_fields.into_iter().collect(),
            };
        }
        let excluded_reason_counts = team_generated_exclusion_counts(excluded);
        PlatformPopulationAssessment {
            status: PlatformPopulationStatus::Measured,
            eligible_customer_count: Some(denominator),
            team_generated_excluded_count: excluded,
            excluded_reason_counts,
            missing_fields: Vec::new(),
        }
    }

    /// Counts cases past their stated SLA deadline at one common supplied
    /// observation cutoff, against one immutable source reference and digest.
    /// This is a descriptive as-of measurement, not proof of closure time or
    /// regulatory breach. Equality with the deadline is on time; only
    /// `observed_at > due` counts as past due.
    pub fn measure_sla_breaches(cases: &[PlatformSlaCase]) -> PlatformSensorFinding {
        let mut missing_fields = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut denominator = 0_u64;
        let mut numerator = 0_u64;
        let mut eligible_rows = Vec::new();
        let mut excluded_reason_counts = BTreeMap::<String, u64>::new();
        let mut source_ref: Option<&str> = None;
        let mut source_digest: Option<&str> = None;
        let mut observed_as_of_ms: Option<i64> = None;
        for case in cases {
            if case.case_id.is_empty() || !ids.insert(case.case_id.as_str()) {
                missing_fields.insert("unique_case_id");
            }
            match case.sla_due_at_ms {
                Some(_) => {}
                None => {
                    missing_fields.insert("sla_due_at");
                }
            }
            match case.observed_at_ms {
                Some(value) if observed_as_of_ms.is_none() || observed_as_of_ms == Some(value) => {
                    observed_as_of_ms = Some(value);
                }
                Some(_) => {
                    missing_fields.insert("consistent_observed_as_of");
                }
                None => {
                    missing_fields.insert("observed_at");
                }
            }
            match case.source_ref.as_deref().filter(|value| valid_ref(value)) {
                Some(value) if source_ref.is_none() || source_ref == Some(value) => {
                    source_ref = Some(value);
                }
                _ => {
                    missing_fields.insert("source_ref");
                }
            }
            match case
                .source_digest
                .as_deref()
                .filter(|value| valid_sha256_ref(value))
            {
                Some(value) if source_digest.is_none() || source_digest == Some(value) => {
                    source_digest = Some(value);
                }
                _ => {
                    missing_fields.insert("source_digest");
                }
            }
            match (case.simulator, case.evidence_kind) {
                (Some(_), Some(_)) => {}
                (None, _) => {
                    missing_fields.insert("cases.simulator");
                }
                (_, None) => {
                    missing_fields.insert("evidence_kind");
                }
            }
            if case.simulator == Some(true) {
                *excluded_reason_counts
                    .entry("team_generated".to_owned())
                    .or_default() += 1;
            }
            if case
                .evidence_kind
                .is_some_and(|kind| kind != EvidenceKind::PlatformAudit)
            {
                *excluded_reason_counts
                    .entry("non_platform_audit_evidence".to_owned())
                    .or_default() += 1;
            }
            if case.simulator != Some(false)
                || case.evidence_kind != Some(EvidenceKind::PlatformAudit)
            {
                continue;
            }
            if let (Some(due), Some(observed)) = (case.sla_due_at_ms, case.observed_at_ms) {
                denominator += 1;
                if observed > due {
                    numerator += 1;
                }
                eligible_rows.push((case.case_id.as_str(), due, observed));
            }
        }
        if cases.is_empty() {
            missing_fields.insert("sla_cases");
        }
        if !missing_fields.is_empty() {
            return insufficient_finding_with_exclusions(
                missing_fields.into_iter().collect(),
                excluded_reason_counts,
            );
        }
        if denominator == 0 {
            return insufficient_finding_with_exclusions(
                vec!["eligible_sla_cases"],
                excluded_reason_counts,
            );
        }
        eligible_rows.sort_by_key(|row| row.0);
        let mut result = finding(
            PlatformSignalStatus::Measured {
                numerator,
                denominator,
                missing: 0,
            },
            Vec::new(),
            excluded_reason_counts,
            source_ref.map(str::to_owned),
            source_digest.map(str::to_owned),
            format!(
                "As of the supplied observation cutoff, {numerator} of {denominator} eligible cases were past their stated SLA due time."
            ),
        );
        result.observed_as_of_ms = observed_as_of_ms;
        result.measurement_digest = Some(measurement_digest(&(
            "platform_sla_measurement_v1",
            result.source_ref.as_deref(),
            result.source_digest.as_deref(),
            observed_as_of_ms,
            eligible_rows,
            numerator,
            denominator,
            &result.excluded_reason_counts,
        )));
        result
    }

    /// Product Phase 1 lacks a Core target for this finding. Keep the result
    /// explicitly waiting; never manufacture a publish target.
    pub fn check_core_target(target: Option<&str>) -> PlatformDependencyCheck {
        match target.filter(|value| valid_ref(value)) {
            Some(_) => PlatformDependencyCheck {
                reason: "core_target_declared",
                dependency_state: "declared_unverified",
            },
            None => PlatformDependencyCheck {
                reason: "missing_core_target",
                dependency_state: "waiting_dependency",
            },
        }
    }
}

fn insufficient_finding_with_exclusions(
    missing_fields: Vec<&'static str>,
    excluded_reason_counts: BTreeMap<String, u64>,
) -> PlatformSensorFinding {
    finding(
        PlatformSignalStatus::InsufficientEvidence,
        missing_fields,
        excluded_reason_counts,
        None,
        None,
        "Insufficient platform evidence; no opportunity was emitted.".to_owned(),
    )
}

fn finding(
    status: PlatformSignalStatus,
    missing_fields: Vec<&'static str>,
    excluded_reason_counts: BTreeMap<String, u64>,
    source_ref: Option<String>,
    source_digest: Option<String>,
    statement: String,
) -> PlatformSensorFinding {
    PlatformSensorFinding {
        status,
        missing_fields,
        excluded_reason_counts,
        source_ref,
        source_digest,
        observed_as_of_ms: None,
        measurement_digest: None,
        statement,
    }
}

fn measurement_digest<T: Serialize>(material: &T) -> String {
    let bytes = serde_json::to_vec(material)
        .expect("platform measurement commitment material is JSON serializable");
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

fn team_generated_exclusion_counts(count: u64) -> BTreeMap<String, u64> {
    if count == 0 {
        BTreeMap::new()
    } else {
        BTreeMap::from([("team_generated".to_owned(), count)])
    }
}

/// The sensor's trusted configuration boundary. Production composition builds
/// this from reviewed/versioned deployment configuration; neither observation
/// batches nor downstream agents can register mappings.
///
/// ```compile_fail
/// use improvement_engine_core::platform_observations::Layer;
/// use improvement_engine_core::platform_sensor::{
///     LayerMetricSpec, PlatformLayerSensor, TrustedLayerMetricMapping,
/// };
///
/// let mapping = TrustedLayerMetricMapping::handoff_rate(
///     LayerMetricSpec::handoff_rate("attention_run_handoff_rate", 1, Layer::Tree, "attention_source_runs").unwrap(),
///     "unreviewed-source",
///     "contract:unreviewed",
/// );
/// let _sensor = PlatformLayerSensor::from_trusted_configuration(vec![mapping.unwrap()]);
/// ```
pub struct PlatformLayerSensor {
    mappings: Vec<TrustedLayerMetricMapping>,
    registry_digest: String,
}

impl PlatformLayerSensor {
    /// Returns the platform-audit sensor compiled from Pulso's reviewed
    /// deployment registry. This is intentionally the only public composition
    /// entrypoint: a downstream caller can request a metric, but cannot mint a
    /// source/contract/population authority for it.
    pub fn for_platform_audit() -> Self {
        Self::from_trusted_configuration(platform_audit_mappings())
            .expect("reviewed platform-audit registry is internally valid")
    }

    fn from_trusted_configuration(
        mappings: Vec<TrustedLayerMetricMapping>,
    ) -> Result<Self, PlatformSensorError> {
        let mut identities = BTreeSet::new();
        for mapping in &mappings {
            let identity = (
                mapping.spec.metric_id.clone(),
                mapping.spec.metric_version,
                mapping.spec.layer as u8,
                mapping.spec.population_ref.clone(),
            );
            if !identities.insert(identity) {
                return Err(PlatformSensorError::DuplicateMapping);
            }
        }
        Ok(Self {
            registry_digest: registry_digest(&mappings),
            mappings,
        })
    }

    /// Measures a handoff rate for exactly one complete, mapping-authorized U29
    /// batch. Multiple eligible complete batches remain ambiguous: U30 must not
    /// sum them until a separate reconciliation contract proves disjointness.
    pub fn measure(
        &self,
        spec: &LayerMetricSpec,
        projection: &WindowProjection,
    ) -> Result<PlatformLayerSignal, PlatformSensorError> {
        spec.validate()?;
        let Some(mapping) = self.mappings.iter().find(|mapping| mapping.matches(spec)) else {
            return Ok(Self::signal(
                spec,
                projection,
                PlatformSignalStatus::InsufficientEvidence,
                None,
                None,
                &self.registry_digest,
            ));
        };
        let mut batches: BTreeMap<String, Provenance> = BTreeMap::new();
        let mut handoff_runs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut unknown_layer_batches = BTreeSet::new();
        for observed in projection.observed_events() {
            let event = observed.event();
            if observed.source_id() != mapping.source_id
                || observed.contract_ref() != mapping.contract_ref
                || event.target_system() != TargetSystem::Attention
                || event.evidence_kind() != EvidenceKind::PlatformAudit
            {
                continue;
            }
            let Some((coverage, denominator)) =
                projection.coverages().iter().find_map(|coverage| {
                    (coverage.coverage().population_ref() == spec.population_ref)
                        .then(|| coverage.denominator_for(observed))
                        .flatten()
                        .map(|denominator| (coverage, denominator))
                })
            else {
                continue;
            };
            if coverage.coverage().population_ref() != spec.population_ref {
                continue;
            }
            let batch_digest = denominator.batch_digest().to_owned();
            if matches!(event.layer(), None | Some(Layer::Unknown)) {
                unknown_layer_batches.insert(batch_digest);
                continue;
            }
            if event.layer() != Some(spec.layer) {
                continue;
            }
            batches.insert(
                batch_digest.clone(),
                Provenance {
                    source_id: observed.source_id().to_owned(),
                    contract_ref: observed.contract_ref().to_owned(),
                    batch_digest: batch_digest.clone(),
                    coverage_evidence_digest: denominator.coverage_evidence_digest().to_owned(),
                    denominator: denominator.expected_population(),
                },
            );
            if event.event_kind() == InteractionEventKind::Handoff {
                handoff_runs
                    .entry(batch_digest)
                    .or_default()
                    .insert(event.source_run_ref().to_owned());
            }
        }
        let (status, provenance) = if batches
            .keys()
            .any(|batch_digest| unknown_layer_batches.contains(batch_digest))
        {
            (PlatformSignalStatus::InsufficientEvidence, None)
        } else {
            match batches.len() {
                0 => (PlatformSignalStatus::InsufficientEvidence, None),
                1 => {
                    let (_, provenance) = batches.into_iter().next().expect("one batch");
                    let numerator = handoff_runs
                        .remove(&provenance.batch_digest)
                        .map(|runs| runs.len() as u64)
                        .unwrap_or(0);
                    let status = if provenance.denominator == 0 {
                        PlatformSignalStatus::InsufficientEvidence
                    } else if numerator > provenance.denominator {
                        PlatformSignalStatus::InconsistentEvidence
                    } else {
                        PlatformSignalStatus::Measured {
                            numerator,
                            denominator: provenance.denominator,
                            missing: 0,
                        }
                    };
                    (status, Some(provenance))
                }
                _ => (PlatformSignalStatus::AmbiguousCoverage, None),
            }
        };
        Ok(Self::signal(
            spec,
            projection,
            status,
            provenance,
            Some(mapping),
            &self.registry_digest,
        ))
    }

    fn signal(
        spec: &LayerMetricSpec,
        projection: &WindowProjection,
        status: PlatformSignalStatus,
        provenance: Option<Provenance>,
        mapping: Option<&TrustedLayerMetricMapping>,
        registry_digest: &str,
    ) -> PlatformLayerSignal {
        let mut signal = PlatformLayerSignal {
            tenant_id: projection.tenant_id().to_owned(),
            metric_id: spec.metric_id.clone(),
            metric_version: spec.metric_version,
            layer: spec.layer,
            population_ref: spec.population_ref.clone(),
            status,
            window_start_ms: projection.window_start_ms(),
            window_end_ms: projection.window_end_ms(),
            received_as_of_ms: projection.received_as_of_ms(),
            projection_digest: projection.digest().to_owned(),
            source_id: provenance.as_ref().map(|value| value.source_id.clone()),
            contract_ref: provenance.as_ref().map(|value| value.contract_ref.clone()),
            batch_digest: provenance.as_ref().map(|value| value.batch_digest.clone()),
            coverage_evidence_digest: provenance
                .as_ref()
                .map(|value| value.coverage_evidence_digest.clone()),
            metric_mapping_digest: mapping.map(|value| value.digest.clone()),
            mapping_resolution_digest: mapping_resolution_digest(
                spec,
                projection.digest(),
                registry_digest,
                mapping.map(|value| value.digest.as_str()),
            ),
            digest: String::new(),
        };
        signal.digest = signal_digest(&signal);
        signal
    }
}

struct Provenance {
    source_id: String,
    contract_ref: String,
    batch_digest: String,
    coverage_evidence_digest: String,
    denominator: u64,
}

#[derive(Serialize)]
struct SignalCommitment<'a> {
    tenant_id: &'a str,
    metric_id: &'a str,
    metric_version: u16,
    layer: Layer,
    population_ref: &'a str,
    status: &'a PlatformSignalStatus,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
    projection_digest: &'a str,
    source_id: Option<&'a str>,
    contract_ref: Option<&'a str>,
    batch_digest: Option<&'a str>,
    coverage_evidence_digest: Option<&'a str>,
    metric_mapping_digest: Option<&'a str>,
    mapping_resolution_digest: &'a str,
}

fn signal_digest(signal: &PlatformLayerSignal) -> String {
    let commitment = SignalCommitment {
        tenant_id: &signal.tenant_id,
        metric_id: &signal.metric_id,
        metric_version: signal.metric_version,
        layer: signal.layer,
        population_ref: &signal.population_ref,
        status: &signal.status,
        window_start_ms: signal.window_start_ms,
        window_end_ms: signal.window_end_ms,
        received_as_of_ms: signal.received_as_of_ms,
        projection_digest: &signal.projection_digest,
        source_id: signal.source_id.as_deref(),
        contract_ref: signal.contract_ref.as_deref(),
        batch_digest: signal.batch_digest.as_deref(),
        coverage_evidence_digest: signal.coverage_evidence_digest.as_deref(),
        metric_mapping_digest: signal.metric_mapping_digest.as_deref(),
        mapping_resolution_digest: &signal.mapping_resolution_digest,
    };
    let bytes = serde_json::to_vec(&commitment).expect("typed platform signal is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Serialize)]
struct MappingCommitment<'a> {
    spec: &'a LayerMetricSpec,
    source_id: &'a str,
    contract_ref: &'a str,
}

fn mapping_digest(mapping: &TrustedLayerMetricMapping) -> String {
    let bytes = serde_json::to_vec(&MappingCommitment {
        spec: &mapping.spec,
        source_id: &mapping.source_id,
        contract_ref: &mapping.contract_ref,
    })
    .expect("typed sensor mapping is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn platform_audit_mappings() -> Vec<TrustedLayerMetricMapping> {
    vec![
        TrustedLayerMetricMapping::handoff_rate(
            LayerMetricSpec::handoff_rate(
                ATTENTION_RUN_HANDOFF_RATE_METRIC_ID,
                1,
                Layer::Tree,
                ATTENTION_SOURCE_RUNS_POPULATION_REF,
            )
            .expect("reviewed platform-audit metric spec is valid"),
            "attention-platform",
            "contract:attention-v1",
        )
        .expect("reviewed platform-audit mapping is valid"),
    ]
}

#[derive(Serialize)]
struct RegistryCommitment<'a> {
    mappings: Vec<RegistryMappingCommitment<'a>>,
}

#[derive(Serialize)]
struct RegistryMappingCommitment<'a> {
    metric_id: &'a str,
    metric_version: u16,
    layer: Layer,
    population_ref: &'a str,
    mapping_digest: &'a str,
}

fn registry_digest(mappings: &[TrustedLayerMetricMapping]) -> String {
    let mut ordered: Vec<_> = mappings
        .iter()
        .map(|mapping| {
            let (metric_id, metric_version, layer, population_ref) = mapping.identity();
            RegistryMappingCommitment {
                metric_id,
                metric_version,
                layer,
                population_ref,
                mapping_digest: &mapping.digest,
            }
        })
        .collect();
    ordered.sort_by(|left, right| {
        (
            left.metric_id,
            left.metric_version,
            left.layer as u8,
            left.population_ref,
        )
            .cmp(&(
                right.metric_id,
                right.metric_version,
                right.layer as u8,
                right.population_ref,
            ))
    });
    let bytes = serde_json::to_vec(&RegistryCommitment { mappings: ordered })
        .expect("typed sensor registry is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Serialize)]
struct MappingResolutionCommitment<'a> {
    spec: &'a LayerMetricSpec,
    projection_digest: &'a str,
    registry_digest: &'a str,
    resolution: &'static str,
    mapping_digest: Option<&'a str>,
}

fn mapping_resolution_digest(
    spec: &LayerMetricSpec,
    projection_digest: &str,
    registry_digest: &str,
    mapping_digest: Option<&str>,
) -> String {
    let bytes = serde_json::to_vec(&MappingResolutionCommitment {
        spec,
        projection_digest,
        registry_digest,
        resolution: if mapping_digest.is_some() {
            "mapped"
        } else {
            "missing"
        },
        mapping_digest,
    })
    .expect("typed mapping resolution is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}
fn valid_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
}
fn valid_sha256_ref(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(source_id: &str, contract_ref: &str) -> TrustedLayerMetricMapping {
        TrustedLayerMetricMapping::handoff_rate(
            LayerMetricSpec::handoff_rate(
                ATTENTION_RUN_HANDOFF_RATE_METRIC_ID,
                1,
                Layer::Tree,
                ATTENTION_SOURCE_RUNS_POPULATION_REF,
            )
            .unwrap(),
            source_id,
            contract_ref,
        )
        .unwrap()
    }

    #[test]
    fn missing_mapping_receipt_commits_registry_presence_and_configuration() {
        let requested = LayerMetricSpec::handoff_rate(
            ATTENTION_RUN_HANDOFF_RATE_METRIC_ID,
            1,
            Layer::Tree,
            ATTENTION_SOURCE_RUNS_POPULATION_REF,
        )
        .unwrap();
        let first = mapping("attention-platform", "contract:attention-v1");
        let changed = mapping("attention-platform", "contract:attention-v2");
        let projection_digest =
            "sha256:0000000000000000000000000000000000000000000000000000000000000000";

        let configured_missing = mapping_resolution_digest(
            &requested,
            projection_digest,
            &registry_digest(&[first]),
            None,
        );
        let empty_missing =
            mapping_resolution_digest(&requested, projection_digest, &registry_digest(&[]), None);
        let changed_missing = mapping_resolution_digest(
            &requested,
            projection_digest,
            &registry_digest(&[changed]),
            None,
        );

        assert!(configured_missing.starts_with("sha256:"));
        assert_ne!(configured_missing, empty_missing);
        assert_ne!(configured_missing, changed_missing);
    }

    #[test]
    fn mapped_and_missing_resolution_receipts_cannot_collide() {
        let spec = LayerMetricSpec::handoff_rate(
            ATTENTION_RUN_HANDOFF_RATE_METRIC_ID,
            1,
            Layer::Tree,
            ATTENTION_SOURCE_RUNS_POPULATION_REF,
        )
        .unwrap();
        let mapping = mapping("attention-platform", "contract:attention-v1");
        let registry = registry_digest(std::slice::from_ref(&mapping));
        let projection_digest =
            "sha256:0000000000000000000000000000000000000000000000000000000000000000";

        assert_ne!(
            mapping_resolution_digest(&spec, projection_digest, &registry, Some(&mapping.digest)),
            mapping_resolution_digest(&spec, projection_digest, &registry, None),
        );
    }

    #[test]
    fn trusted_mapping_digest_commits_the_fixed_run_population_grain() {
        let mapping = mapping("attention-platform", "contract:attention-v1");
        let commitment = serde_json::to_string(&MappingCommitment {
            spec: &mapping.spec,
            source_id: &mapping.source_id,
            contract_ref: &mapping.contract_ref,
        })
        .unwrap();
        assert!(commitment.contains(ATTENTION_SOURCE_RUNS_POPULATION_REF));
        assert_eq!(mapping.digest, mapping_digest(&mapping));
    }
}
