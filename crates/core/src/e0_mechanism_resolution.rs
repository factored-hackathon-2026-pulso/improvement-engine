//! Candidate-bound, descriptive E0 mechanism evidence and exact route lookup.
//!
//! This module does not compile, evaluate, or authorize a Core artifact. A
//! catalog match is evidence of a declared mapping only; later admission,
//! policy, and evaluation gates remain independent requirements.

use std::collections::BTreeSet;
use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ArtifactReference;
use crate::e0_proposal_assembly::{
    LocalProposalCandidate, LocalProposalSnapshotReference, assemble_e0_proposals,
};
use crate::local_simulation::{
    LocalRunResult, LocalSourceKind, SignalSummary, signal_summary_commitment,
};

const RECURRING_QUERY_METRIC: &str = "e0_recurring_copilot_query_cases";
const SUPPORTED_AGENT_CORE_CONTRACT_PIN: &str = "86a767474042a566a0dbd6ed23588959f27ebdb3";

/// Privacy-safe evidence bound to one assembled E0 proposal candidate.
///
/// It contains commitments and aggregate counts only: never query text or the
/// underlying query signature. The pattern reference is a recurrence key, not
/// an intent, route, or customer identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0MechanismEvidencePacket {
    // Trusted source scope for resolver isolation; never serialized into the
    // user-facing evidence packet or route-resolution receipt.
    #[serde(skip)]
    tenant_scope: String,
    proposal_ref: String,
    source_run_id: String,
    source_snapshot_ref: LocalProposalSnapshotReference,
    observed_cutoff_rfc3339: String,
    metric_id: String,
    signal_digest: String,
    summary_commitment: String,
    candidate_digest: String,
    pattern_ref: String,
    numerator: u64,
    denominator: u64,
    missing: u64,
    claim_level: String,
}

impl E0MechanismEvidencePacket {
    /// Bind a candidate and signal to their source run. Assembly and membership
    /// checks prevent a caller from substituting another candidate, signal, or
    /// snapshot and then recomputing a superficial digest.
    pub fn from_candidate(
        result: &LocalRunResult,
        candidate: &LocalProposalCandidate,
        signal: &SignalSummary,
    ) -> Result<Self, E0MechanismEvidenceError> {
        if result.source_kind != LocalSourceKind::E0
            || !result.signals.contains(signal)
            || signal_summary_commitment(signal) != signal.summary_commitment
            || signal.metric_id != RECURRING_QUERY_METRIC
            || candidate.metric_id != RECURRING_QUERY_METRIC
            || candidate.signal_digest != signal.digest
            || candidate.summary_commitment != signal.summary_commitment
            || candidate.detector_policy_id != signal.detector_policy_id
            || candidate.detector_policy_version != signal.detector_policy_version
            || candidate.claim_level != "descriptive_only"
            || candidate.route_status != "unlinked"
            || candidate.evaluation_status != "not_evaluated"
            || candidate.business_lift.is_some()
            || signal.numerator < signal.minimum_support
        {
            return Err(E0MechanismEvidenceError::CandidateSignalMismatch);
        }
        let pattern_ref = signal
            .pattern_ref
            .as_deref()
            .filter(|value| is_sha256(value))
            .ok_or(E0MechanismEvidenceError::MissingOrInvalidPatternRef)?;

        let assembly = assemble_e0_proposals(result)
            .map_err(|_| E0MechanismEvidenceError::InvalidSourceRun)?;
        if !assembly.candidates.iter().any(|item| item == candidate)
            || assembly.source_run_id != result.run_id
            || assembly.source_snapshot_ref.id != candidate.source_snapshot_ref.id
            || assembly.source_snapshot_ref.revision != candidate.source_snapshot_ref.revision
            || assembly.source_snapshot_ref.digest != candidate.source_snapshot_ref.digest
            || assembly.observed_cutoff_rfc3339 != candidate.observed_cutoff_rfc3339
        {
            return Err(E0MechanismEvidenceError::CandidateNotInSourceRun);
        }

        Ok(Self {
            tenant_scope: result.tenant_id.clone(),
            proposal_ref: candidate.proposal_ref.clone(),
            source_run_id: result.run_id.clone(),
            source_snapshot_ref: candidate.source_snapshot_ref.clone(),
            observed_cutoff_rfc3339: candidate.observed_cutoff_rfc3339.clone(),
            metric_id: candidate.metric_id.clone(),
            signal_digest: signal.digest.clone(),
            summary_commitment: signal.summary_commitment.clone(),
            candidate_digest: candidate_digest(candidate),
            pattern_ref: pattern_ref.to_owned(),
            numerator: signal.numerator,
            denominator: signal.denominator,
            missing: signal.missing,
            claim_level: "descriptive_only".to_owned(),
        })
    }

    #[must_use]
    pub fn proposal_ref(&self) -> &str {
        &self.proposal_ref
    }

    #[must_use]
    pub fn source_run_id(&self) -> &str {
        &self.source_run_id
    }

    #[must_use]
    pub fn source_snapshot_ref(&self) -> &LocalProposalSnapshotReference {
        &self.source_snapshot_ref
    }

    #[must_use]
    pub fn observed_cutoff_rfc3339(&self) -> &str {
        &self.observed_cutoff_rfc3339
    }

    #[must_use]
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }

    #[must_use]
    pub fn signal_digest(&self) -> &str {
        &self.signal_digest
    }

    #[must_use]
    pub fn summary_commitment(&self) -> &str {
        &self.summary_commitment
    }

    #[must_use]
    pub fn pattern_ref(&self) -> &str {
        &self.pattern_ref
    }

    #[must_use]
    pub fn numerator(&self) -> u64 {
        self.numerator
    }

    #[must_use]
    pub fn denominator(&self) -> u64 {
        self.denominator
    }

    #[must_use]
    pub fn missing(&self) -> u64 {
        self.missing
    }

    #[must_use]
    pub fn claim_level(&self) -> &str {
        &self.claim_level
    }

    pub(crate) fn tenant_scope(&self) -> &str {
        &self.tenant_scope
    }

    pub(crate) fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }
}

fn candidate_digest(candidate: &LocalProposalCandidate) -> String {
    let bytes = serde_json::to_vec(candidate).expect("proposal candidate serializes");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E0MechanismEvidenceError {
    CandidateSignalMismatch,
    CandidateNotInSourceRun,
    MissingOrInvalidPatternRef,
    InvalidSourceRun,
}

/// Descriptive Flow metadata checked for syntax and the pinned Agent Core
/// contract identifier. This type does not prove the Flow exists in a registry
/// or that `content_digest` matches a retrieved artifact.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SupportedCoreFlowRef {
    contract_pin_sha: String,
    flow_id: String,
    flow_version: String,
    content_digest: String,
}

impl SupportedCoreFlowRef {
    pub fn new(
        contract_pin_sha: impl Into<String>,
        flow_id: impl Into<String>,
        flow_version: impl Into<String>,
        content_digest: impl Into<String>,
    ) -> Result<Self, E0RouteCatalogError> {
        let flow = Self {
            contract_pin_sha: contract_pin_sha.into(),
            flow_id: flow_id.into(),
            flow_version: flow_version.into(),
            content_digest: content_digest.into(),
        };
        if flow.contract_pin_sha != SUPPORTED_AGENT_CORE_CONTRACT_PIN
            || !is_core_entity_id(&flow.flow_id)
            || !is_semver(&flow.flow_version)
            || !is_sha256(&flow.content_digest)
        {
            return Err(E0RouteCatalogError::InvalidFlowReference);
        }
        Ok(flow)
    }

    #[must_use]
    pub fn contract_pin_sha(&self) -> &str {
        &self.contract_pin_sha
    }

    #[must_use]
    pub fn flow_id(&self) -> &str {
        &self.flow_id
    }

    #[must_use]
    pub fn flow_version(&self) -> &str {
        &self.flow_version
    }

    #[must_use]
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }
}

/// One explicit, exact recurrence-pattern-to-Flow mapping. It is not inferred
/// from a metric name or human-readable hypothesis.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0RouteMapping {
    metric_id: String,
    pattern_ref: String,
    candidate_route: String,
    mechanism: String,
    flow_ref: SupportedCoreFlowRef,
}

impl E0RouteMapping {
    pub fn new(
        metric_id: impl Into<String>,
        pattern_ref: impl Into<String>,
        candidate_route: impl Into<String>,
        mechanism: impl Into<String>,
        flow_ref: SupportedCoreFlowRef,
    ) -> Result<Self, E0RouteCatalogError> {
        let mapping = Self {
            metric_id: metric_id.into(),
            pattern_ref: pattern_ref.into(),
            candidate_route: candidate_route.into(),
            mechanism: mechanism.into(),
            flow_ref,
        };
        if mapping.metric_id != RECURRING_QUERY_METRIC
            || !is_sha256(&mapping.pattern_ref)
            || mapping.candidate_route.is_empty()
            || mapping.mechanism.is_empty()
        {
            return Err(E0RouteCatalogError::InvalidMapping);
        }
        Ok(mapping)
    }
}

/// Immutable mapping snapshot. Its ArtifactReference (id, revision, digest)
/// is mandatory, including when the catalog is empty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct E0RouteCatalog {
    reference: ArtifactReference,
    mappings: Vec<E0RouteMapping>,
}

/// Tenant-safe view of a catalog's immutable identity in run output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0RouteCatalogReference {
    id: String,
    revision: u64,
    digest: String,
}

impl E0RouteCatalogReference {
    fn from_artifact(reference: &ArtifactReference) -> Self {
        Self {
            id: reference.id.clone(),
            revision: reference.revision,
            digest: reference.digest.clone(),
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

impl E0RouteCatalog {
    /// Canonical content commitment used by the immutable catalog reference.
    #[must_use]
    pub fn content_digest(mappings: &[E0RouteMapping]) -> String {
        let bytes = serde_json::to_vec(&RouteCatalogContent {
            schema_version: 1,
            mappings,
        })
        .expect("route catalog content is infallibly serializable");
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    pub fn new(
        reference: ArtifactReference,
        mappings: Vec<E0RouteMapping>,
    ) -> Result<Self, E0RouteCatalogError> {
        if reference.tenant_id.trim().is_empty()
            || reference.tenant_id.len() > 128
            || !is_uuid_v7(&reference.id)
            || reference.revision == 0
            || !is_sha256(&reference.digest)
            || reference.digest != Self::content_digest(&mappings)
        {
            return Err(E0RouteCatalogError::InvalidCatalogReference);
        }
        let keys = mappings
            .iter()
            .map(|mapping| (mapping.metric_id.as_str(), mapping.pattern_ref.as_str()))
            .collect::<BTreeSet<_>>();
        if keys.len() != mappings.len() {
            return Err(E0RouteCatalogError::DuplicateMapping);
        }
        Ok(Self {
            reference,
            mappings,
        })
    }

    pub fn empty(reference: ArtifactReference) -> Result<Self, E0RouteCatalogError> {
        Self::new(reference, Vec::new())
    }

    #[must_use]
    pub fn reference(&self) -> &ArtifactReference {
        &self.reference
    }

    fn resolve(&self, packet: &E0MechanismEvidencePacket) -> Option<&E0RouteMapping> {
        self.mappings.iter().find(|mapping| {
            mapping.metric_id == packet.metric_id && mapping.pattern_ref == packet.pattern_ref
        })
    }
}

#[derive(Serialize)]
struct RouteCatalogContent<'a> {
    schema_version: u16,
    mappings: &'a [E0RouteMapping],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E0RouteCatalogError {
    InvalidCatalogReference,
    InvalidFlowReference,
    InvalidMapping,
    DuplicateMapping,
}

/// A resolver-minted, read-only receipt for one exact mechanism packet.
///
/// The serialized form contains route status, lookup key, catalog reference
/// and (when mapped) exact Flow reference. Its private packet binding is kept
/// in memory for downstream validation, but is not serialized. Callers cannot
/// construct or deserialize this type; use [`resolve_e0_mechanism_route`].
///
/// ```compile_fail
/// use improvement_engine_core::e0_mechanism_resolution::RouteResolution;
/// let forged = RouteResolution::Mapped {
///     catalog_ref: todo!(),
///     metric_id: "e0_recurring_copilot_query_cases".into(),
///     pattern_ref: format!("sha256:{}", "a".repeat(64)),
///     flow_ref: todo!(),
/// };
/// ```
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct RouteResolution {
    #[serde(flatten)]
    kind: RouteResolutionKind,
    #[serde(skip)]
    packet_binding: E0MechanismEvidencePacket,
    #[serde(skip)]
    catalog_tenant_scope: String,
}

impl fmt::Debug for RouteResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RouteResolution")
            .field("status", &self.status())
            .field("metric_id", &self.metric_id())
            .field("pattern_ref", &self.pattern_ref())
            .field("catalog_ref", self.catalog_ref())
            .field("reason", &self.reason())
            .field("flow_ref", &self.flow_ref())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RouteResolutionKind {
    Unlinked {
        catalog_ref: E0RouteCatalogReference,
        metric_id: String,
        pattern_ref: String,
        reason: &'static str,
    },
    Mapped {
        catalog_ref: E0RouteCatalogReference,
        metric_id: String,
        pattern_ref: String,
        candidate_route: String,
        mechanism: String,
        flow_ref: SupportedCoreFlowRef,
    },
}

impl RouteResolution {
    #[must_use]
    pub fn status(&self) -> &'static str {
        match &self.kind {
            RouteResolutionKind::Unlinked { .. } => "unlinked",
            RouteResolutionKind::Mapped { .. } => "mapped",
        }
    }

    #[must_use]
    pub fn metric_id(&self) -> &str {
        match &self.kind {
            RouteResolutionKind::Unlinked { metric_id, .. }
            | RouteResolutionKind::Mapped { metric_id, .. } => metric_id,
        }
    }

    #[must_use]
    pub fn pattern_ref(&self) -> &str {
        match &self.kind {
            RouteResolutionKind::Unlinked { pattern_ref, .. }
            | RouteResolutionKind::Mapped { pattern_ref, .. } => pattern_ref,
        }
    }

    #[must_use]
    pub fn reason(&self) -> Option<&'static str> {
        match &self.kind {
            RouteResolutionKind::Unlinked { reason, .. } => Some(*reason),
            RouteResolutionKind::Mapped { .. } => None,
        }
    }

    #[must_use]
    pub fn catalog_ref(&self) -> &E0RouteCatalogReference {
        match &self.kind {
            RouteResolutionKind::Unlinked { catalog_ref, .. }
            | RouteResolutionKind::Mapped { catalog_ref, .. } => catalog_ref,
        }
    }

    #[must_use]
    pub fn flow_ref(&self) -> Option<&SupportedCoreFlowRef> {
        match &self.kind {
            RouteResolutionKind::Mapped { flow_ref, .. } => Some(flow_ref),
            RouteResolutionKind::Unlinked { .. } => None,
        }
    }

    pub(crate) fn candidate_route(&self) -> &str {
        match &self.kind {
            RouteResolutionKind::Mapped {
                candidate_route, ..
            } => candidate_route,
            RouteResolutionKind::Unlinked { .. } => "",
        }
    }

    pub(crate) fn mechanism(&self) -> &str {
        match &self.kind {
            RouteResolutionKind::Mapped { mechanism, .. } => mechanism,
            RouteResolutionKind::Unlinked { .. } => "",
        }
    }

    pub(crate) fn is_bound_to(&self, packet: &E0MechanismEvidencePacket) -> bool {
        &self.packet_binding == packet
            && self.metric_id() == packet.metric_id()
            && self.pattern_ref() == packet.pattern_ref()
    }

    pub(crate) fn catalog_tenant_scope(&self) -> &str {
        &self.catalog_tenant_scope
    }

    /// A route mapping alone is never compile authority.
    #[must_use]
    pub fn may_compile(&self) -> bool {
        false
    }

    /// A route mapping alone is never sandbox-trial authority.
    #[must_use]
    pub fn may_start_sandbox_trial(&self) -> bool {
        false
    }
}

pub fn resolve_e0_mechanism_route(
    packet: &E0MechanismEvidencePacket,
    catalog: &E0RouteCatalog,
) -> RouteResolution {
    if packet.tenant_scope != catalog.reference.tenant_id {
        return RouteResolution {
            kind: RouteResolutionKind::Unlinked {
                catalog_ref: E0RouteCatalogReference::from_artifact(&catalog.reference),
                metric_id: packet.metric_id.clone(),
                pattern_ref: packet.pattern_ref.clone(),
                reason: "catalog_tenant_mismatch",
            },
            packet_binding: packet.clone(),
            catalog_tenant_scope: catalog.reference.tenant_id.clone(),
        };
    }
    let kind = match catalog.resolve(packet) {
        Some(mapping) => RouteResolutionKind::Mapped {
            catalog_ref: E0RouteCatalogReference::from_artifact(&catalog.reference),
            metric_id: packet.metric_id.clone(),
            pattern_ref: packet.pattern_ref.clone(),
            candidate_route: mapping.candidate_route.clone(),
            mechanism: mapping.mechanism.clone(),
            flow_ref: mapping.flow_ref.clone(),
        },
        None => RouteResolutionKind::Unlinked {
            catalog_ref: E0RouteCatalogReference::from_artifact(&catalog.reference),
            metric_id: packet.metric_id.clone(),
            pattern_ref: packet.pattern_ref.clone(),
            reason: "no_exact_supported_flow_mapping",
        },
    };
    RouteResolution {
        kind,
        packet_binding: packet.clone(),
        catalog_tenant_scope: catalog.reference.tenant_id.clone(),
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

/// Matches the pinned Agent Core entity identifier grammar:
/// `[a-z0-9][a-z0-9_/-]*`.
fn is_core_entity_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'/' | b'-')
        })
}

/// Route references pin exact Core versions; accept strict numeric X.Y.Z only.
fn is_semver(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        })
}

fn is_uuid_v7(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
        && bytes[14] == b'7'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && value.bytes().all(|byte| !byte.is_ascii_uppercase())
}
