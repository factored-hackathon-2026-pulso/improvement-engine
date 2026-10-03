//! Foundational, non-wire binding of an E0 candidate to an already compiled
//! Core draft and immutable registry/evaluation references.
//!
//! This module does not compile a `ChangeSpec`, write a Core registry, invoke
//! evaluation, or prove improvement. A future bridge adapter may mint the
//! opaque registry readback only after a real, exact Agent Core read.

use crate::change_compiler::{CompiledChange, CoreEntityKind};
use crate::e0_mechanism_resolution::{E0MechanismEvidencePacket, RouteResolution};
use crate::e0_proposal_assembly::LocalProposalCandidate;
use sha2::{Digest, Sha256};

/// Declared Core artifact identity carried into a future native trial.
/// Format validation is not trusted registry readback; refs remain unverified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreArtifactRef {
    id: String,
    version: String,
    digest: String,
}

impl CoreArtifactRef {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        digest: impl Into<String>,
    ) -> Result<Self, E0DraftBindingError> {
        let value = Self {
            id: id.into(),
            version: version.into(),
            digest: digest.into(),
        };
        if !valid_identifier(&value.id)
            || !valid_version(&value.version)
            || !valid_digest(&value.digest)
        {
            return Err(E0DraftBindingError::InvalidCoreArtifactReference);
        }
        Ok(value)
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Unverified declaration used only to exercise the binding seam. It is not
/// evidence that Agent Core returned these fields; a trusted bridge verifier
/// must replace this type before any production verified status is possible.
#[derive(Clone, Eq, PartialEq)]
pub struct UnverifiedCoreRouteReadback {
    tenant_id: String,
    agent_core_sha: String,
    catalog_id: String,
    catalog_revision: u64,
    catalog_digest: String,
    metric_id: String,
    pattern_ref: String,
    flow_id: String,
    flow_version: String,
    content_digest: String,
    registry_revision: u64,
    readback_digest: String,
}

impl UnverifiedCoreRouteReadback {
    /// Crate-private test seam. Never treat this constructor as Core proof.
    #[allow(dead_code)] // The Claude-owned bridge adapter will mint this receipt.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn declared_unverified(
        tenant_id: impl Into<String>,
        agent_core_sha: impl Into<String>,
        catalog_id: impl Into<String>,
        catalog_revision: u64,
        catalog_digest: impl Into<String>,
        metric_id: impl Into<String>,
        pattern_ref: impl Into<String>,
        flow_id: impl Into<String>,
        flow_version: impl Into<String>,
        content_digest: impl Into<String>,
        registry_revision: u64,
    ) -> Result<Self, E0DraftBindingError> {
        let result = Self {
            tenant_id: tenant_id.into(),
            agent_core_sha: agent_core_sha.into(),
            catalog_id: catalog_id.into(),
            catalog_revision,
            catalog_digest: catalog_digest.into(),
            metric_id: metric_id.into(),
            pattern_ref: pattern_ref.into(),
            flow_id: flow_id.into(),
            flow_version: flow_version.into(),
            content_digest: content_digest.into(),
            registry_revision,
            readback_digest: String::new(),
        };
        if !valid_identifier(&result.tenant_id)
            || !valid_git_sha(&result.agent_core_sha)
            || !valid_identifier(&result.catalog_id)
            || result.catalog_revision == 0
            || !valid_digest(&result.catalog_digest)
            || !valid_identifier(&result.metric_id)
            || !valid_digest(&result.pattern_ref)
            || !valid_identifier(&result.flow_id)
            || !valid_version(&result.flow_version)
            || !valid_digest(&result.content_digest)
            || result.registry_revision == 0
        {
            return Err(E0DraftBindingError::InvalidRegistryReadback);
        }
        let mut result = result;
        result.readback_digest = readback_digest(&result);
        Ok(result)
    }

    fn validate_integrity(&self) -> bool {
        self.readback_digest == readback_digest(self)
    }
}

/// Immutable, descriptive lineage binding for one already-compiled draft.
/// It is not a Core Proposal, registry write, evaluation, or promotion grant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct E0CompiledCandidateBinding {
    proposal_ref: String,
    candidate_digest: String,
    source_run_id: String,
    source_snapshot_id: String,
    source_snapshot_revision: u64,
    source_snapshot_digest: String,
    signal_digest: String,
    summary_commitment: String,
    route_catalog_id: String,
    route_catalog_revision: u64,
    route_catalog_digest: String,
    mapped_metric_id: String,
    mapped_pattern_ref: String,
    agent_core_sha: String,
    flow_id: String,
    flow_version: String,
    flow_content_digest: String,
    registry_revision: u64,
    readback_digest: String,
    draft_digest: String,
    compiled_commitment: String,
    base_release: CoreArtifactRef,
    evaluation_suite: CoreArtifactRef,
    binding_digest: String,
    status: &'static str,
}

impl E0CompiledCandidateBinding {
    #[must_use]
    pub fn proposal_ref(&self) -> &str {
        &self.proposal_ref
    }

    #[must_use]
    pub fn source_run_id(&self) -> &str {
        &self.source_run_id
    }

    #[must_use]
    pub fn signal_digest(&self) -> &str {
        &self.signal_digest
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
    pub fn base_release(&self) -> &CoreArtifactRef {
        &self.base_release
    }

    #[must_use]
    pub fn evaluation_suite(&self) -> &CoreArtifactRef {
        &self.evaluation_suite
    }

    #[must_use]
    pub fn binding_digest(&self) -> &str {
        &self.binding_digest
    }

    #[must_use]
    pub fn status(&self) -> &'static str {
        self.status
    }

    /// Detects accidental or serialized-field drift; this is not a signature.
    pub fn validate_integrity(&self) -> bool {
        self.binding_digest
            == binding_digest_fields(
                &self.proposal_ref,
                &self.candidate_digest,
                &self.source_run_id,
                &self.source_snapshot_id,
                self.source_snapshot_revision,
                &self.source_snapshot_digest,
                &self.signal_digest,
                &self.summary_commitment,
                &self.route_catalog_id,
                self.route_catalog_revision,
                &self.route_catalog_digest,
                &self.mapped_metric_id,
                &self.mapped_pattern_ref,
                &self.agent_core_sha,
                &self.flow_id,
                &self.flow_version,
                &self.flow_content_digest,
                self.registry_revision,
                &self.readback_digest,
                &self.draft_digest,
                &self.compiled_commitment,
                &self.base_release,
                &self.evaluation_suite,
            )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E0DraftBindingError {
    CandidateEvidenceMismatch,
    RegistryReadbackRequired,
    InvalidRegistryReadback,
    RegistryReadbackMismatch,
    RouteNotMapped,
    CompiledDraftMismatch,
    InvalidCoreArtifactReference,
}

/// Bind an existing compiled draft to one E0 candidate, an exact mapped Flow
/// and immutable base/suite refs. A fixture catalog is not a registry
/// readback; an unlinked route is never promoted to a candidate binding.
pub fn bind_e0_compiled_candidate(
    candidate: &LocalProposalCandidate,
    packet: &E0MechanismEvidencePacket,
    route: &RouteResolution,
    registry_readback: Option<&UnverifiedCoreRouteReadback>,
    compiled: &CompiledChange,
    base_release: CoreArtifactRef,
    evaluation_suite: CoreArtifactRef,
) -> Result<E0CompiledCandidateBinding, E0DraftBindingError> {
    if candidate.claim_level != "descriptive_only"
        || candidate.business_lift.is_some()
        || candidate.native_agent_core_status != "not_connected"
        || candidate.route_status != "unlinked"
        || candidate.evaluation_status != "not_evaluated"
        || candidate.proposal_ref != packet.proposal_ref()
        || candidate.source_run_id != packet.source_run_id()
        || candidate.metric_id != packet.metric_id()
        || candidate.signal_digest != packet.signal_digest()
        || candidate.summary_commitment != packet.summary_commitment()
        || candidate.source_snapshot_ref.id != packet.source_snapshot_ref().id
        || candidate.source_snapshot_ref.revision != packet.source_snapshot_ref().revision
        || candidate.source_snapshot_ref.digest != packet.source_snapshot_ref().digest
        || candidate.observed_cutoff_rfc3339 != packet.observed_cutoff_rfc3339()
        || candidate_digest(candidate) != packet.candidate_digest()
        || packet.claim_level() != "descriptive_only"
        || !route.is_bound_to(packet)
    {
        return Err(E0DraftBindingError::CandidateEvidenceMismatch);
    }
    if route.status() != "mapped" || route.flow_ref().is_none() {
        return Err(E0DraftBindingError::RouteNotMapped);
    }
    let readback = registry_readback.ok_or(E0DraftBindingError::RegistryReadbackRequired)?;
    let flow = route.flow_ref().expect("mapped route has a flow reference");
    if !readback.validate_integrity()
        || packet.tenant_scope() != route.catalog_tenant_scope()
        || packet.tenant_scope() != compiled.authorization().scope().tenant_id()
        || packet.tenant_scope() != readback.tenant_id
        || readback.tenant_id != compiled.authorization().scope().tenant_id()
        || readback.agent_core_sha != flow.contract_pin_sha()
        || readback.catalog_id != route.catalog_ref().id()
        || readback.catalog_revision != route.catalog_ref().revision()
        || readback.catalog_digest != route.catalog_ref().digest()
        || readback.metric_id != packet.metric_id()
        || readback.pattern_ref != packet.pattern_ref()
        || readback.flow_id != flow.flow_id()
        || readback.flow_version != flow.flow_version()
        || readback.content_digest != flow.content_digest()
        || readback.registry_revision == 0
    {
        return Err(E0DraftBindingError::RegistryReadbackMismatch);
    }
    let authorization = compiled.authorization();
    let snapshot = authorization.source_snapshot();
    let drafts = compiled.drafts();
    if drafts.len() != 1
        || drafts[0].kind() != CoreEntityKind::Flow
        || drafts[0].id() != readback.flow_id
        || drafts[0].version() != readback.flow_version
        || authorization.kind() != CoreEntityKind::Flow
        || authorization.entity_id() != readback.flow_id
        || authorization.entity_version() != readback.flow_version
        || authorization.content_digest() != readback.content_digest
        || snapshot.id != candidate.source_snapshot_ref.id
        || snapshot.revision != candidate.source_snapshot_ref.revision
        || snapshot.digest != candidate.source_snapshot_ref.digest
        || !valid_digest(drafts[0].digest())
        || authorization.candidate_route() != route.candidate_route()
        || authorization.mechanism() != route.mechanism()
        || !valid_digest(compiled.commitment())
    {
        return Err(E0DraftBindingError::CompiledDraftMismatch);
    }
    let mut binding = E0CompiledCandidateBinding {
        proposal_ref: candidate.proposal_ref.clone(),
        candidate_digest: candidate_digest(candidate),
        source_run_id: candidate.source_run_id.clone(),
        source_snapshot_id: candidate.source_snapshot_ref.id.clone(),
        source_snapshot_revision: candidate.source_snapshot_ref.revision,
        source_snapshot_digest: candidate.source_snapshot_ref.digest.clone(),
        signal_digest: candidate.signal_digest.clone(),
        summary_commitment: packet.summary_commitment().to_owned(),
        route_catalog_id: route.catalog_ref().id().to_owned(),
        route_catalog_revision: route.catalog_ref().revision(),
        route_catalog_digest: route.catalog_ref().digest().to_owned(),
        mapped_metric_id: readback.metric_id.clone(),
        mapped_pattern_ref: readback.pattern_ref.clone(),
        agent_core_sha: readback.agent_core_sha.clone(),
        flow_id: readback.flow_id.clone(),
        flow_version: readback.flow_version.clone(),
        flow_content_digest: readback.content_digest.clone(),
        registry_revision: readback.registry_revision,
        readback_digest: readback.readback_digest.clone(),
        draft_digest: drafts[0].digest().to_owned(),
        compiled_commitment: compiled.commitment().to_owned(),
        base_release,
        evaluation_suite,
        binding_digest: String::new(),
        status: "unverified_draft_and_refs_bound_not_submitted_or_evaluated",
    };
    binding.binding_digest = binding_digest_fields(
        &binding.proposal_ref,
        &binding.candidate_digest,
        &binding.source_run_id,
        &binding.source_snapshot_id,
        binding.source_snapshot_revision,
        &binding.source_snapshot_digest,
        &binding.signal_digest,
        &binding.summary_commitment,
        &binding.route_catalog_id,
        binding.route_catalog_revision,
        &binding.route_catalog_digest,
        &binding.mapped_metric_id,
        &binding.mapped_pattern_ref,
        &binding.agent_core_sha,
        &binding.flow_id,
        &binding.flow_version,
        &binding.flow_content_digest,
        binding.registry_revision,
        &binding.readback_digest,
        &binding.draft_digest,
        &binding.compiled_commitment,
        &binding.base_release,
        &binding.evaluation_suite,
    );
    Ok(binding)
}

#[allow(clippy::too_many_arguments)]
fn binding_digest_fields(
    proposal_ref: &str,
    candidate_digest: &str,
    source_run_id: &str,
    source_snapshot_id: &str,
    source_snapshot_revision: u64,
    source_snapshot_digest: &str,
    signal_digest: &str,
    summary_commitment: &str,
    route_catalog_id: &str,
    route_catalog_revision: u64,
    route_catalog_digest: &str,
    mapped_metric_id: &str,
    mapped_pattern_ref: &str,
    agent_core_sha: &str,
    flow_id: &str,
    flow_version: &str,
    flow_content_digest: &str,
    registry_revision: u64,
    readback_digest: &str,
    draft_digest: &str,
    compiled_commitment: &str,
    base_release: &CoreArtifactRef,
    suite: &CoreArtifactRef,
) -> String {
    let mut hash = Sha256::new();
    for part in [
        proposal_ref.to_owned(),
        candidate_digest.to_owned(),
        source_run_id.to_owned(),
        source_snapshot_id.to_owned(),
        source_snapshot_revision.to_string(),
        source_snapshot_digest.to_owned(),
        signal_digest.to_owned(),
        summary_commitment.to_owned(),
        route_catalog_id.to_owned(),
        route_catalog_revision.to_string(),
        route_catalog_digest.to_owned(),
        mapped_metric_id.to_owned(),
        mapped_pattern_ref.to_owned(),
        agent_core_sha.to_owned(),
        flow_id.to_owned(),
        flow_version.to_owned(),
        flow_content_digest.to_owned(),
        registry_revision.to_string(),
        readback_digest.to_owned(),
        draft_digest.to_owned(),
        compiled_commitment.to_owned(),
        base_release.id.clone(),
        base_release.version.clone(),
        base_release.digest.clone(),
        suite.id.clone(),
        suite.version.clone(),
        suite.digest.clone(),
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("sha256:{:x}", hash.finalize())
}

fn candidate_digest(candidate: &LocalProposalCandidate) -> String {
    let bytes = serde_json::to_vec(candidate).expect("proposal candidate serializes infallibly");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn readback_digest(readback: &UnverifiedCoreRouteReadback) -> String {
    let mut hash = Sha256::new();
    for part in [
        readback.tenant_id.as_str(),
        readback.agent_core_sha.as_str(),
        readback.catalog_id.as_str(),
        &readback.catalog_revision.to_string(),
        readback.catalog_digest.as_str(),
        readback.metric_id.as_str(),
        readback.pattern_ref.as_str(),
        readback.flow_id.as_str(),
        readback.flow_version.as_str(),
        readback.content_digest.as_str(),
        &readback.registry_revision.to_string(),
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("sha256:{:x}", hash.finalize())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_git_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'/' | b'.'))
}

fn valid_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArtifactReference;
    use crate::change_compiler::compiled_for_governed_registry_test;
    use crate::e0_mechanism_resolution::{
        E0MechanismEvidencePacket, E0RouteCatalog, E0RouteMapping, SupportedCoreFlowRef,
        resolve_e0_mechanism_route,
    };
    use crate::e0_proposal_assembly::assemble_e0_proposals;
    use crate::local_simulation::{
        LocalObservedQuery, LocalRunInput, LocalRunMetadata, LocalSourceKind, run_local_simulation,
    };

    const TENANT: &str = "pulso_local";
    const CORE_SHA: &str = "86a767474042a566a0dbd6ed23588959f27ebdb3";
    const SOURCE_ID: &str = "018f0f4e-7bbd-7000-8000-000000000600";

    fn artifact_ref(id: &str) -> CoreArtifactRef {
        CoreArtifactRef::new(id, "1.0.0", format!("sha256:{}", "e".repeat(64)))
            .expect("valid Core ref")
    }

    fn source_snapshot() -> ArtifactReference {
        ArtifactReference {
            tenant_id: TENANT.into(),
            id: SOURCE_ID.into(),
            revision: 1,
            digest: format!("sha256:{}", "a".repeat(64)),
        }
    }

    fn e0_candidate() -> (
        crate::e0_proposal_assembly::LocalProposalCandidate,
        E0MechanismEvidencePacket,
    ) {
        let run = run_local_simulation(
            LocalRunInput::new(
                LocalRunMetadata::new(
                    "run_123_1775000000",
                    TENANT,
                    LocalSourceKind::E0,
                    format!("sha256:{}", "b".repeat(64)),
                    source_snapshot(),
                    1_775_001_600,
                    "2026-04-01T00:00:00Z",
                ),
                (1..=20).collect(),
                0,
                Vec::new(),
            )
            .with_queries(
                (1..=20)
                    .map(|ordinal| {
                        LocalObservedQuery::new(
                            ordinal,
                            format!("sha256_{}", "a".repeat(56)),
                            format!("2026-03-01T00:00:{:02}Z", ordinal % 60),
                        )
                    })
                    .collect(),
            ),
        )
        .expect("simulated E0 run");
        let candidate = assemble_e0_proposals(&run)
            .expect("proposal assembly")
            .candidates
            .into_iter()
            .find(|candidate| candidate.metric_id == "e0_recurring_copilot_query_cases")
            .expect("recurring-query candidate");
        let signal = run
            .signals
            .iter()
            .find(|signal| signal.metric_id == candidate.metric_id)
            .expect("source signal");
        let packet = E0MechanismEvidencePacket::from_candidate(&run, &candidate, signal)
            .expect("candidate-bound packet");
        (candidate, packet)
    }

    fn mapped_resolution(
        packet: &E0MechanismEvidencePacket,
        compiled: &crate::change_compiler::CompiledChange,
    ) -> crate::e0_mechanism_resolution::RouteResolution {
        let draft = &compiled.drafts()[0];
        let flow = SupportedCoreFlowRef::new(
            CORE_SHA,
            draft.id(),
            draft.version(),
            compiled.authorization().content_digest(),
        )
        .expect("flow ref");
        let mapping = E0RouteMapping::new(
            packet.metric_id(),
            packet.pattern_ref(),
            "flow/payment-status",
            "transaction_status_lookup",
            flow,
        )
        .expect("mapping");
        let catalog = E0RouteCatalog::new(
            ArtifactReference {
                tenant_id: TENANT.into(),
                id: "0199b21c-7eab-7000-8000-000000000203".into(),
                revision: 2,
                digest: E0RouteCatalog::content_digest(std::slice::from_ref(&mapping)),
            },
            vec![mapping],
        )
        .expect("team catalog fixture");
        resolve_e0_mechanism_route(packet, &catalog)
    }

    fn readback_for(
        packet: &E0MechanismEvidencePacket,
        route: &crate::e0_mechanism_resolution::RouteResolution,
        compiled: &crate::change_compiler::CompiledChange,
    ) -> UnverifiedCoreRouteReadback {
        let flow = route.flow_ref().expect("mapped flow");
        UnverifiedCoreRouteReadback::declared_unverified(
            TENANT,
            CORE_SHA,
            route.catalog_ref().id(),
            route.catalog_ref().revision(),
            route.catalog_ref().digest(),
            packet.metric_id(),
            packet.pattern_ref(),
            flow.flow_id(),
            flow.flow_version(),
            compiled.authorization().content_digest(),
            7,
        )
        .expect("synthetic route readback for pure boundary tests")
    }

    #[test]
    fn fixture_mapping_without_core_readback_declaration_is_blocked() {
        let compiled = compiled_for_governed_registry_test(TENANT);
        let (candidate, packet) = e0_candidate();
        let route = mapped_resolution(&packet, &compiled);

        let result = bind_e0_compiled_candidate(
            &candidate,
            &packet,
            &route,
            None,
            &compiled,
            artifact_ref("base-release-1"),
            artifact_ref("recurring-query-suite"),
        );

        assert_eq!(result, Err(E0DraftBindingError::RegistryReadbackRequired));
    }

    #[test]
    fn exact_candidate_route_draft_and_refs_produce_non_executable_binding() {
        let compiled = compiled_for_governed_registry_test(TENANT);
        let (candidate, packet) = e0_candidate();
        let route = mapped_resolution(&packet, &compiled);
        let readback = readback_for(&packet, &route, &compiled);

        let binding = bind_e0_compiled_candidate(
            &candidate,
            &packet,
            &route,
            Some(&readback),
            &compiled,
            artifact_ref("base-release-1"),
            artifact_ref("recurring-query-suite"),
        )
        .expect("exact bindings pass the pure seam");

        assert_eq!(
            binding.status,
            "unverified_draft_and_refs_bound_not_submitted_or_evaluated"
        );
        assert_eq!(binding.flow_id, "payment_status_resolution");
        assert_eq!(binding.flow_version, "1.0.0");
        assert_eq!(binding.source_run_id, candidate.source_run_id);
        assert_eq!(binding.signal_digest, candidate.signal_digest);
        assert_eq!(binding.agent_core_sha, CORE_SHA);
        assert!(binding.binding_digest.starts_with("sha256:"));
        assert!(binding.validate_integrity());
        let second = bind_e0_compiled_candidate(
            &candidate,
            &packet,
            &route,
            Some(&readback),
            &compiled,
            artifact_ref("base-release-1"),
            CoreArtifactRef::new(
                "recurring-query-suite",
                "1.0.1",
                format!("sha256:{}", "e".repeat(64)),
            )
            .expect("valid updated suite pin"),
        )
        .expect("new suite pin remains bindable");
        assert_ne!(binding.binding_digest, second.binding_digest);
        let mut drifted = binding;
        drifted.evaluation_suite.version = "2.0.0".into();
        assert!(!drifted.validate_integrity());
    }

    #[test]
    fn candidate_policy_and_hypothesis_are_bound_to_the_evidence_receipt() {
        let compiled = compiled_for_governed_registry_test(TENANT);
        let (mut candidate, packet) = e0_candidate();
        let route = mapped_resolution(&packet, &compiled);
        let readback = readback_for(&packet, &route, &compiled);
        candidate.detector_policy_version += 1;

        assert_eq!(
            bind_e0_compiled_candidate(
                &candidate,
                &packet,
                &route,
                Some(&readback),
                &compiled,
                artifact_ref("base-release-1"),
                artifact_ref("recurring-query-suite"),
            ),
            Err(E0DraftBindingError::CandidateEvidenceMismatch)
        );
    }

    #[test]
    fn compiled_tenant_must_match_packet_route_and_readback_tenants() {
        let compiled = compiled_for_governed_registry_test("other_tenant");
        let (candidate, packet) = e0_candidate();
        let route = mapped_resolution(&packet, &compiled);
        let readback = readback_for(&packet, &route, &compiled);

        assert_eq!(
            bind_e0_compiled_candidate(
                &candidate,
                &packet,
                &route,
                Some(&readback),
                &compiled,
                artifact_ref("base-release-1"),
                artifact_ref("recurring-query-suite"),
            ),
            Err(E0DraftBindingError::RegistryReadbackMismatch)
        );
    }

    #[test]
    fn cross_tenant_route_catalog_cannot_resolve_or_bind() {
        let compiled = compiled_for_governed_registry_test(TENANT);
        let (candidate, packet) = e0_candidate();
        let draft = &compiled.drafts()[0];
        let flow = SupportedCoreFlowRef::new(
            CORE_SHA,
            draft.id(),
            draft.version(),
            compiled.authorization().content_digest(),
        )
        .expect("flow ref");
        let mapping = E0RouteMapping::new(
            packet.metric_id(),
            packet.pattern_ref(),
            "flow/payment-status",
            "transaction_status_lookup",
            flow,
        )
        .expect("mapping");
        let catalog = E0RouteCatalog::new(
            ArtifactReference {
                tenant_id: "other_tenant".into(),
                id: "0199b21c-7eab-7000-8000-000000000203".into(),
                revision: 2,
                digest: E0RouteCatalog::content_digest(std::slice::from_ref(&mapping)),
            },
            vec![mapping],
        )
        .expect("foreign catalog");
        let route = resolve_e0_mechanism_route(&packet, &catalog);
        assert_eq!(route.status(), "unlinked");

        assert_eq!(
            bind_e0_compiled_candidate(
                &candidate,
                &packet,
                &route,
                None,
                &compiled,
                artifact_ref("base-release-1"),
                artifact_ref("recurring-query-suite"),
            ),
            Err(E0DraftBindingError::RouteNotMapped)
        );
    }

    #[test]
    fn mutated_readback_fields_with_stale_digest_are_rejected() {
        let compiled = compiled_for_governed_registry_test(TENANT);
        let (candidate, packet) = e0_candidate();
        let route = mapped_resolution(&packet, &compiled);
        let mut readback = readback_for(&packet, &route, &compiled);
        readback.flow_id.push_str("_changed");

        assert_eq!(
            bind_e0_compiled_candidate(
                &candidate,
                &packet,
                &route,
                Some(&readback),
                &compiled,
                artifact_ref("base-release-1"),
                artifact_ref("recurring-query-suite"),
            ),
            Err(E0DraftBindingError::RegistryReadbackMismatch)
        );
    }

    #[test]
    fn compiled_candidate_route_and_mechanism_must_match_exact_catalog_entry() {
        let compiled = compiled_for_governed_registry_test(TENANT);
        let (candidate, packet) = e0_candidate();
        let draft = &compiled.drafts()[0];
        let flow = SupportedCoreFlowRef::new(
            CORE_SHA,
            draft.id(),
            draft.version(),
            compiled.authorization().content_digest(),
        )
        .expect("flow ref");
        let mapping = E0RouteMapping::new(
            packet.metric_id(),
            packet.pattern_ref(),
            "flow/different-route",
            "different-mechanism",
            flow,
        )
        .expect("mapping");
        let catalog = E0RouteCatalog::new(
            ArtifactReference {
                tenant_id: TENANT.into(),
                id: "0199b21c-7eab-7000-8000-000000000203".into(),
                revision: 2,
                digest: E0RouteCatalog::content_digest(std::slice::from_ref(&mapping)),
            },
            vec![mapping],
        )
        .expect("catalog");
        let route = resolve_e0_mechanism_route(&packet, &catalog);
        let readback = readback_for(&packet, &route, &compiled);

        assert_eq!(
            bind_e0_compiled_candidate(
                &candidate,
                &packet,
                &route,
                Some(&readback),
                &compiled,
                artifact_ref("base-release-1"),
                artifact_ref("recurring-query-suite"),
            ),
            Err(E0DraftBindingError::CompiledDraftMismatch)
        );
    }

    #[test]
    fn core_refs_reject_invalid_version_and_digest() {
        assert_eq!(
            CoreArtifactRef::new("suite", "01.0.0", format!("sha256:{}", "a".repeat(64))),
            Err(E0DraftBindingError::InvalidCoreArtifactReference)
        );
        assert_eq!(
            CoreArtifactRef::new("suite", "1.0.0", "not-a-digest"),
            Err(E0DraftBindingError::InvalidCoreArtifactReference)
        );
    }

    #[test]
    fn unmapped_candidate_is_blocked_even_when_a_readback_is_available() {
        let compiled = compiled_for_governed_registry_test(TENANT);
        let (candidate, packet) = e0_candidate();
        let empty_catalog = E0RouteCatalog::empty(ArtifactReference {
            tenant_id: TENANT.into(),
            id: "0199b21c-7eab-7000-8000-000000000202".into(),
            revision: 1,
            digest: E0RouteCatalog::content_digest(&[]),
        })
        .expect("empty fixture catalog");
        let route = resolve_e0_mechanism_route(&packet, &empty_catalog);
        let mapped = mapped_resolution(&packet, &compiled);
        let readback = readback_for(&packet, &mapped, &compiled);

        let result = bind_e0_compiled_candidate(
            &candidate,
            &packet,
            &route,
            Some(&readback),
            &compiled,
            artifact_ref("base-release-1"),
            artifact_ref("recurring-query-suite"),
        );

        assert_eq!(result, Err(E0DraftBindingError::RouteNotMapped));
    }

    #[test]
    fn route_catalog_must_match_the_persisted_mapping_readback() {
        let compiled = compiled_for_governed_registry_test(TENANT);
        let (candidate, packet) = e0_candidate();
        let route = mapped_resolution(&packet, &compiled);
        let mut readback = readback_for(&packet, &route, &compiled);
        readback.catalog_digest = format!("sha256:{}", "d".repeat(64));

        let result = bind_e0_compiled_candidate(
            &candidate,
            &packet,
            &route,
            Some(&readback),
            &compiled,
            artifact_ref("base-release-1"),
            artifact_ref("recurring-query-suite"),
        );

        assert_eq!(result, Err(E0DraftBindingError::RegistryReadbackMismatch));
    }

    #[test]
    fn source_and_compiled_draft_must_match_before_binding() {
        let compiled = compiled_for_governed_registry_test(TENANT);
        let (mut candidate, packet) = e0_candidate();
        let route = mapped_resolution(&packet, &compiled);
        let readback = readback_for(&packet, &route, &compiled);
        candidate.source_snapshot_ref.digest = format!("sha256:{}", "d".repeat(64));

        let result = bind_e0_compiled_candidate(
            &candidate,
            &packet,
            &route,
            Some(&readback),
            &compiled,
            artifact_ref("base-release-1"),
            artifact_ref("recurring-query-suite"),
        );

        assert_eq!(result, Err(E0DraftBindingError::CandidateEvidenceMismatch));
    }
}
