//! P1 handoff from a governed U30 platform measurement into discovery.
//!
//! This is deliberately a typed discovery input, not a Scout candidate or an
//! opportunity claim. U13 still requires its own source/Core/model authority
//! receipts; until a platform-specific U13 contract exists this capability
//! only carries eligible evidence to that future composition boundary.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::core_task::CoreTaskScope;
use crate::platform_observations::Layer;
use crate::platform_sensor::{PlatformLayerSignal, PlatformSignalStatus};

/// A measured, provenance-bound platform signal prepared for hypothesis
/// generation. Fields are private and there is no deserializer or public
/// constructor, so callers cannot fabricate U30 evidence by setting digests.
///
/// ```compile_fail
/// use improvement_engine_core::platform_discovery::PlatformDiscoveryInput;
/// let _forged = PlatformDiscoveryInput {
///     metric_id: "tree_handoff_rate".into(), metric_version: 1,
///     layer: improvement_engine_core::platform_observations::Layer::Tree,
///     population_ref: "tree_goals".into(), numerator: 9, denominator: 10,
///     missing: 0, window_start_ms: 1, window_end_ms: 2, received_as_of_ms: 2,
///     projection_digest: "sha256:00".into(), source_id: "fake".into(),
///     contract_ref: "fake".into(), batch_digest: "sha256:00".into(),
///     coverage_evidence_digest: "sha256:00".into(),
///     metric_mapping_digest: "sha256:00".into(),
///     mapping_resolution_digest: "sha256:00".into(), signal_digest: "sha256:00".into(),
///     commitment: "sha256:00".into(),
/// };
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformDiscoveryInput {
    tenant_id: String,
    metric_id: String,
    metric_version: u16,
    layer: Layer,
    population_ref: String,
    numerator: u64,
    denominator: u64,
    missing: u64,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
    projection_digest: String,
    source_id: String,
    contract_ref: String,
    batch_digest: String,
    coverage_evidence_digest: String,
    metric_mapping_digest: String,
    mapping_resolution_digest: String,
    signal_digest: String,
    commitment: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformDiscoveryInputError {
    InsufficientEvidence,
    IncompleteProvenance,
    TenantScopeMismatch,
}

impl PlatformDiscoveryInput {
    /// Copies only a measured U30 result. Partial, ambiguous, inconsistent,
    /// unmapped or otherwise insufficient measurements cannot enter discovery.
    pub fn from_measured_signal(
        signal: &PlatformLayerSignal,
        scope: &CoreTaskScope,
    ) -> Result<Self, PlatformDiscoveryInputError> {
        if signal.tenant_id() != scope.tenant_id() {
            return Err(PlatformDiscoveryInputError::TenantScopeMismatch);
        }
        let PlatformSignalStatus::Measured {
            numerator,
            denominator,
            missing,
        } = signal.status()
        else {
            return Err(PlatformDiscoveryInputError::InsufficientEvidence);
        };

        let (
            Some(source_id),
            Some(contract_ref),
            Some(batch_digest),
            Some(coverage_evidence_digest),
            Some(metric_mapping_digest),
        ) = (
            signal.source_id(),
            signal.contract_ref(),
            signal.batch_digest(),
            signal.coverage_evidence_digest(),
            signal.metric_mapping_digest(),
        )
        else {
            return Err(PlatformDiscoveryInputError::IncompleteProvenance);
        };

        if signal.window_start_ms() >= signal.window_end_ms()
            || signal.received_as_of_ms() < signal.window_end_ms()
            || denominator == &0
            || numerator > denominator
            || !is_sha256(signal.projection_digest())
            || !is_sha256(signal.mapping_resolution_digest())
            || !is_sha256(signal.digest())
            || !is_sha256(batch_digest)
            || !is_sha256(coverage_evidence_digest)
            || !is_sha256(metric_mapping_digest)
        {
            return Err(PlatformDiscoveryInputError::IncompleteProvenance);
        }

        let mut input = Self {
            tenant_id: signal.tenant_id().to_owned(),
            metric_id: signal.metric_id().to_owned(),
            metric_version: signal.metric_version(),
            layer: signal.layer(),
            population_ref: signal.population_ref().to_owned(),
            numerator: *numerator,
            denominator: *denominator,
            missing: *missing,
            window_start_ms: signal.window_start_ms(),
            window_end_ms: signal.window_end_ms(),
            received_as_of_ms: signal.received_as_of_ms(),
            projection_digest: signal.projection_digest().to_owned(),
            source_id: source_id.to_owned(),
            contract_ref: contract_ref.to_owned(),
            batch_digest: batch_digest.to_owned(),
            coverage_evidence_digest: coverage_evidence_digest.to_owned(),
            metric_mapping_digest: metric_mapping_digest.to_owned(),
            mapping_resolution_digest: signal.mapping_resolution_digest().to_owned(),
            signal_digest: signal.digest().to_owned(),
            commitment: String::new(),
        };
        input.commitment = input_digest(&input);
        Ok(input)
    }

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
    pub fn numerator(&self) -> u64 {
        self.numerator
    }
    pub fn denominator(&self) -> u64 {
        self.denominator
    }
    pub fn missing(&self) -> u64 {
        self.missing
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
    pub fn source_id(&self) -> &str {
        &self.source_id
    }
    pub fn contract_ref(&self) -> &str {
        &self.contract_ref
    }
    pub fn batch_digest(&self) -> &str {
        &self.batch_digest
    }
    pub fn coverage_evidence_digest(&self) -> &str {
        &self.coverage_evidence_digest
    }
    pub fn metric_mapping_digest(&self) -> &str {
        &self.metric_mapping_digest
    }
    pub fn mapping_resolution_digest(&self) -> &str {
        &self.mapping_resolution_digest
    }
    pub fn signal_digest(&self) -> &str {
        &self.signal_digest
    }
    pub fn commitment(&self) -> &str {
        &self.commitment
    }

    /// Detects alteration after serialization or transport. This proves only
    /// commitment integrity; it does not upgrade the input into U13 authority.
    pub fn has_valid_commitment(&self) -> bool {
        self.commitment == input_digest(self)
    }
}

#[derive(Serialize)]
struct InputCommitment<'a> {
    tenant_id: &'a str,
    metric_id: &'a str,
    metric_version: u16,
    layer: Layer,
    population_ref: &'a str,
    numerator: u64,
    denominator: u64,
    missing: u64,
    window_start_ms: i64,
    window_end_ms: i64,
    received_as_of_ms: i64,
    projection_digest: &'a str,
    source_id: &'a str,
    contract_ref: &'a str,
    batch_digest: &'a str,
    coverage_evidence_digest: &'a str,
    metric_mapping_digest: &'a str,
    mapping_resolution_digest: &'a str,
    signal_digest: &'a str,
}

fn input_digest(input: &PlatformDiscoveryInput) -> String {
    let bytes = serde_json::to_vec(&InputCommitment {
        tenant_id: &input.tenant_id,
        metric_id: &input.metric_id,
        metric_version: input.metric_version,
        layer: input.layer,
        population_ref: &input.population_ref,
        numerator: input.numerator,
        denominator: input.denominator,
        missing: input.missing,
        window_start_ms: input.window_start_ms,
        window_end_ms: input.window_end_ms,
        received_as_of_ms: input.received_as_of_ms,
        projection_digest: &input.projection_digest,
        source_id: &input.source_id,
        contract_ref: &input.contract_ref,
        batch_digest: &input.batch_digest,
        coverage_evidence_digest: &input.coverage_evidence_digest,
        metric_mapping_digest: &input.metric_mapping_digest,
        mapping_resolution_digest: &input.mapping_resolution_digest,
        signal_digest: &input.signal_digest,
    })
    .expect("typed platform discovery input is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
