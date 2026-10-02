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

use crate::platform_observations::{
    EvidenceKind, InteractionEventKind, Layer, TargetSystem, WindowProjection,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LayerMetricSpec {
    metric_id: String,
    metric_version: u16,
    layer: Layer,
    /// Canonical eligible-population semantics, not merely a display label.
    population_ref: String,
}

impl LayerMetricSpec {
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
            || self.metric_version == 0
            || !valid_ref(&self.population_ref)
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
/// let _forged = PlatformLayerSignal { metric_id: "tree_handoff_rate".to_owned(), metric_version: 1, layer: Layer::Tree, population_ref: "tree_goals".to_owned(), status: PlatformSignalStatus::InsufficientEvidence, window_start_ms: 1, window_end_ms: 2, received_as_of_ms: 2, projection_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned(), source_id: None, contract_ref: None, batch_digest: None, coverage_evidence_digest: None, metric_mapping_digest: None, digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned() };
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformLayerSignal {
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
///     LayerMetricSpec::handoff_rate("arbitrary", 1, Layer::Tree, "anything").unwrap(),
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
                    coverage
                        .denominator_for(observed)
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
            LayerMetricSpec::handoff_rate("tree_handoff_rate", 1, Layer::Tree, "tree_goals")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(source_id: &str, contract_ref: &str) -> TrustedLayerMetricMapping {
        TrustedLayerMetricMapping::handoff_rate(
            LayerMetricSpec::handoff_rate("tree_handoff_rate", 1, Layer::Tree, "tree_goals")
                .unwrap(),
            source_id,
            contract_ref,
        )
        .unwrap()
    }

    #[test]
    fn missing_mapping_receipt_commits_registry_presence_and_configuration() {
        let requested = LayerMetricSpec::handoff_rate(
            "tree_handoff_rate",
            1,
            Layer::Tree,
            "tree_goals_excluding_retries",
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
        let spec = LayerMetricSpec::handoff_rate("tree_handoff_rate", 1, Layer::Tree, "tree_goals")
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
}
