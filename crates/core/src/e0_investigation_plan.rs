//! Read-only investigation envelope for one E0 proposal seed.
//!
//! This is not an Agent Core Proposal or executable artifact. It binds a
//! descriptive candidate to its exact evidence packet and route-catalog
//! resolution, then presents bounded investigation choices without granting
//! execution, compilation, evaluation, publication, or lift authority.

use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::e0_mechanism_resolution::{E0MechanismEvidencePacket, RouteResolution};
use crate::e0_proposal_assembly::{LocalProposalCandidate, LocalProposalSnapshotReference};

const PLAN_SCHEMA_VERSION: u16 = 1;
const UNLINKED_MAPPING_REASON: &str = "no_exact_supported_flow_mapping";
const PLAN_ARTIFACT_KIND: &str = "e0_read_only_investigation_plan_not_agent_core_proposal";

/// User-visible, non-executable next-step suggestion for investigating an E0
/// candidate. `InvestigateMappedFlow` means inspect a declared mapping only;
/// it does not run or evaluate the Flow.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum E0InvestigationOption {
    InvestigateMapping,
    InvestigateMappedFlow,
    DoNothing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum E0InvestigationReviewState {
    PendingReview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum E0InvestigationAuthority {
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum E0InvestigationExecutionState {
    NotExecutable,
}

/// The plan's bounded decision envelope. A recommendation is an investigation
/// prompt only; choosing it cannot invoke an Agent Core primitive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0InvestigationDecisionEnvelope {
    pub review_state: E0InvestigationReviewState,
    pub recommended_option: E0InvestigationOption,
    pub available_options: Vec<E0InvestigationOption>,
    pub authority: E0InvestigationAuthority,
    pub execution_state: E0InvestigationExecutionState,
}

/// Candidate-bound E0 investigation plan. Identifiers and commitments are
/// included for lineage; tenant scope and source query values are not.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct E0InvestigationProposalPlan {
    pub schema_version: u16,
    pub artifact_kind: String,
    pub plan_digest: String,
    pub proposal_ref: String,
    pub source_run_id: String,
    pub source_snapshot_ref: LocalProposalSnapshotReference,
    pub observed_cutoff_rfc3339: String,
    pub metric_id: String,
    pub signal_digest: String,
    pub summary_commitment: String,
    pub evidence_packet: E0MechanismEvidencePacket,
    pub route_resolution: RouteResolution,
    pub decision: E0InvestigationDecisionEnvelope,
    pub claim_level: String,
    pub business_lift: Option<String>,
}

impl fmt::Debug for E0InvestigationProposalPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("E0InvestigationProposalPlan")
            .field("schema_version", &self.schema_version)
            .field("artifact_kind", &self.artifact_kind)
            .field("plan_digest", &self.plan_digest)
            .field("proposal_ref", &self.proposal_ref)
            .field("source_run_id", &self.source_run_id)
            .field("source_snapshot_ref", &self.source_snapshot_ref)
            .field("observed_cutoff_rfc3339", &self.observed_cutoff_rfc3339)
            .field("metric_id", &self.metric_id)
            .field("signal_digest", &self.signal_digest)
            .field("summary_commitment", &self.summary_commitment)
            .field("evidence_packet", &"candidate-bound privacy-safe packet")
            .field("route_resolution", &self.route_resolution)
            .field("decision", &self.decision)
            .field("claim_level", &self.claim_level)
            .field("business_lift", &self.business_lift)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E0InvestigationPlanError {
    CandidateEvidenceMismatch,
    RouteEvidenceMismatch,
    UnsupportedRouteResolution,
    InvalidPlan,
    DigestMismatch,
}

/// Bind a proposal seed, its exact evidence packet, and the exact catalog
/// resolution into an investigation-only envelope.
pub fn build_e0_investigation_plan(
    candidate: &LocalProposalCandidate,
    packet: &E0MechanismEvidencePacket,
    resolution: &RouteResolution,
) -> Result<E0InvestigationProposalPlan, E0InvestigationPlanError> {
    if !candidate_matches_packet(candidate, packet) {
        return Err(E0InvestigationPlanError::CandidateEvidenceMismatch);
    }
    if !route_resolution_matches_packet(resolution, packet) {
        return Err(E0InvestigationPlanError::RouteEvidenceMismatch);
    }
    let (recommended_option, available_options) = match (resolution.status(), resolution.reason()) {
        ("unlinked", Some(UNLINKED_MAPPING_REASON)) => (
            E0InvestigationOption::InvestigateMapping,
            vec![
                E0InvestigationOption::InvestigateMapping,
                E0InvestigationOption::DoNothing,
            ],
        ),
        ("mapped", None) => (
            E0InvestigationOption::InvestigateMappedFlow,
            vec![
                E0InvestigationOption::InvestigateMappedFlow,
                E0InvestigationOption::DoNothing,
            ],
        ),
        _ => {
            return Err(E0InvestigationPlanError::UnsupportedRouteResolution);
        }
    };
    let mut plan = E0InvestigationProposalPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        artifact_kind: PLAN_ARTIFACT_KIND.to_owned(),
        plan_digest: String::new(),
        proposal_ref: candidate.proposal_ref.clone(),
        source_run_id: candidate.source_run_id.clone(),
        source_snapshot_ref: candidate.source_snapshot_ref.clone(),
        observed_cutoff_rfc3339: candidate.observed_cutoff_rfc3339.clone(),
        metric_id: candidate.metric_id.clone(),
        signal_digest: candidate.signal_digest.clone(),
        summary_commitment: candidate.summary_commitment.clone(),
        evidence_packet: packet.clone(),
        route_resolution: resolution.clone(),
        decision: E0InvestigationDecisionEnvelope {
            review_state: E0InvestigationReviewState::PendingReview,
            recommended_option,
            available_options,
            authority: E0InvestigationAuthority::None,
            execution_state: E0InvestigationExecutionState::NotExecutable,
        },
        claim_level: "descriptive_only".to_owned(),
        business_lift: None,
    };
    if !plan.bindings_are_valid() {
        return Err(E0InvestigationPlanError::InvalidPlan);
    }
    plan.plan_digest = plan.compute_digest();
    Ok(plan)
}

impl E0InvestigationProposalPlan {
    /// Check the content commitment and all duplicated lineage bindings before
    /// persisting or rendering this typed plan; it grants no further authority.
    pub fn validate_integrity(&self) -> Result<(), E0InvestigationPlanError> {
        if self.plan_digest != self.compute_digest() {
            return Err(E0InvestigationPlanError::DigestMismatch);
        }
        if !self.bindings_are_valid() {
            return Err(E0InvestigationPlanError::InvalidPlan);
        }
        Ok(())
    }

    fn bindings_are_valid(&self) -> bool {
        if self.schema_version != PLAN_SCHEMA_VERSION
            || self.artifact_kind != PLAN_ARTIFACT_KIND
            || self.proposal_ref != self.evidence_packet.proposal_ref()
            || self.source_run_id != self.evidence_packet.source_run_id()
            || self.source_snapshot_ref != *self.evidence_packet.source_snapshot_ref()
            || self.observed_cutoff_rfc3339 != self.evidence_packet.observed_cutoff_rfc3339()
            || self.metric_id != self.evidence_packet.metric_id()
            || self.signal_digest != self.evidence_packet.signal_digest()
            || self.summary_commitment != self.evidence_packet.summary_commitment()
            || !route_resolution_matches_packet(&self.route_resolution, &self.evidence_packet)
            || self.claim_level != "descriptive_only"
            || self.business_lift.is_some()
            || self.decision.review_state != E0InvestigationReviewState::PendingReview
            || self.decision.authority != E0InvestigationAuthority::None
            || self.decision.execution_state != E0InvestigationExecutionState::NotExecutable
        {
            return false;
        }
        match (
            self.route_resolution.status(),
            self.route_resolution.reason(),
        ) {
            ("unlinked", Some(UNLINKED_MAPPING_REASON)) => {
                self.decision.recommended_option == E0InvestigationOption::InvestigateMapping
                    && self.decision.available_options
                        == [
                            E0InvestigationOption::InvestigateMapping,
                            E0InvestigationOption::DoNothing,
                        ]
            }
            ("mapped", None) => {
                self.decision.recommended_option == E0InvestigationOption::InvestigateMappedFlow
                    && self.decision.available_options
                        == [
                            E0InvestigationOption::InvestigateMappedFlow,
                            E0InvestigationOption::DoNothing,
                        ]
            }
            _ => false,
        }
    }

    fn compute_digest(&self) -> String {
        let content = PlanDigestContent {
            schema_version: self.schema_version,
            artifact_kind: &self.artifact_kind,
            proposal_ref: &self.proposal_ref,
            source_run_id: &self.source_run_id,
            source_snapshot_ref: &self.source_snapshot_ref,
            observed_cutoff_rfc3339: &self.observed_cutoff_rfc3339,
            metric_id: &self.metric_id,
            signal_digest: &self.signal_digest,
            summary_commitment: &self.summary_commitment,
            evidence_packet: &self.evidence_packet,
            route_resolution: &self.route_resolution,
            decision: &self.decision,
            claim_level: &self.claim_level,
            business_lift: &self.business_lift,
        };
        let bytes = serde_json::to_vec(&content).expect("plan content is serializable");
        format!("sha256:{:x}", Sha256::digest(bytes))
    }
}

fn candidate_matches_packet(
    candidate: &LocalProposalCandidate,
    packet: &E0MechanismEvidencePacket,
) -> bool {
    candidate.proposal_ref == packet.proposal_ref()
        && candidate.source_run_id == packet.source_run_id()
        && candidate.source_snapshot_ref == *packet.source_snapshot_ref()
        && candidate.observed_cutoff_rfc3339 == packet.observed_cutoff_rfc3339()
        && candidate.metric_id == packet.metric_id()
        && candidate.signal_digest == packet.signal_digest()
        && candidate.summary_commitment == packet.summary_commitment()
        && candidate.claim_level == packet.claim_level()
        && candidate.business_lift.is_none()
}

fn route_resolution_matches_packet(
    resolution: &RouteResolution,
    packet: &E0MechanismEvidencePacket,
) -> bool {
    resolution.is_bound_to(packet)
}

#[derive(Serialize)]
struct PlanDigestContent<'a> {
    schema_version: u16,
    artifact_kind: &'a str,
    proposal_ref: &'a str,
    source_run_id: &'a str,
    source_snapshot_ref: &'a LocalProposalSnapshotReference,
    observed_cutoff_rfc3339: &'a str,
    metric_id: &'a str,
    signal_digest: &'a str,
    summary_commitment: &'a str,
    evidence_packet: &'a E0MechanismEvidencePacket,
    route_resolution: &'a RouteResolution,
    decision: &'a E0InvestigationDecisionEnvelope,
    claim_level: &'a str,
    business_lift: &'a Option<String>,
}
