//! Truthful runner boundary between E0 discovery and later builder design.
//!
//! The local runner can seal candidate provenance, but it does not compose a
//! trusted U20 evaluation plan or U20-E E0 safety oracle. Therefore this
//! receipt is never a builder input, Core artifact, proposal, or authorization.

use improvement_engine_core::e0_proposal_assembly::{
    LocalProposalAssembly, LocalProposalCandidate, LocalProposalSnapshotReference,
};
use improvement_engine_core::local_simulation::{LocalRunResult, LocalSourceKind, RunEvent};
use serde::Serialize;

const SCHEMA_VERSION: &str = "e0_builder_input_preparation_v1";
const ARTIFACT_KIND: &str = "builder_input_preparation_status_not_proposal";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceBindingStatus {
    Bound,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BuilderReadinessStatus {
    DependencyBlocked,
    NotApplicable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ReadinessDependencies {
    u20_plan: &'static str,
    e0_safety_oracle: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SafeCandidateProvenance {
    metric_id: String,
    signal_digest: String,
    summary_commitment: String,
}

/// Persistable status only. The type intentionally contains no model prompt,
/// output, route, Core artifact reference, authority, or opaque readiness
/// commitment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct E0BuilderInputPreparation {
    schema_version: &'static str,
    artifact_kind: &'static str,
    status: BuilderReadinessStatus,
    evidence_binding: EvidenceBindingStatus,
    source_run_id: String,
    source_snapshot_ref: LocalProposalSnapshotReference,
    observed_cutoff_rfc3339: String,
    candidate_count: usize,
    candidates: Vec<SafeCandidateProvenance>,
    reason: Option<&'static str>,
    readiness: ReadinessDependencies,
    provider_invoked: bool,
    executable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreparationError {
    NotE0,
    RunMismatch,
    TenantMismatch,
    SnapshotMismatch,
    CutoffMismatch,
    InvalidCandidateProvenance,
}

impl E0BuilderInputPreparation {
    /// Binds the runner's validated E0 proposal assembly to its source run.
    ///
    /// `assembly` must be the result of `assemble_e0_proposals(run)`. This
    /// method rechecks source/run/snapshot/cutoff identity but intentionally
    /// relies on that constructor for signal membership and digest validity.
    pub(crate) fn from_run(
        run: &LocalRunResult,
        assembly: &LocalProposalAssembly,
        sequence: u32,
    ) -> Result<(Self, RunEvent), PreparationError> {
        if run.source_kind != LocalSourceKind::E0 || assembly.source_family != "e0" {
            return Err(PreparationError::NotE0);
        }
        if run.tenant_id != run.snapshot_ref.tenant_id {
            return Err(PreparationError::TenantMismatch);
        }
        let unsupported = assembly.status
            == improvement_engine_core::e0_proposal_assembly::LocalProposalAssemblyStatus::Unsupported;
        if !unsupported {
            if assembly.source_run_id != run.run_id {
                return Err(PreparationError::RunMismatch);
            }
            if !snapshot_matches(&assembly.source_snapshot_ref, &run.snapshot_ref) {
                return Err(PreparationError::SnapshotMismatch);
            }
            if assembly.observed_cutoff_rfc3339 != run.observed_cutoff_rfc3339 {
                return Err(PreparationError::CutoffMismatch);
            }
        }
        if assembly
            .candidates
            .iter()
            .any(|candidate| !candidate_matches(candidate, run, assembly))
        {
            return Err(PreparationError::InvalidCandidateProvenance);
        }

        let has_candidates = !assembly.candidates.is_empty();
        let reason = if has_candidates {
            None
        } else if unsupported {
            Some("proposal_assembly_unsupported")
        } else {
            Some("no_qualifying_candidate")
        };
        let (status, evidence_binding, readiness) = if has_candidates {
            (
                BuilderReadinessStatus::DependencyBlocked,
                EvidenceBindingStatus::Bound,
                ReadinessDependencies {
                    u20_plan: "unavailable_in_local_simulation",
                    e0_safety_oracle: "unavailable_in_local_simulation",
                },
            )
        } else {
            (
                BuilderReadinessStatus::NotApplicable,
                EvidenceBindingStatus::NotApplicable,
                ReadinessDependencies {
                    u20_plan: "not_requested_without_candidate",
                    e0_safety_oracle: "not_requested_without_candidate",
                },
            )
        };
        let record = Self {
            schema_version: SCHEMA_VERSION,
            artifact_kind: ARTIFACT_KIND,
            status,
            evidence_binding,
            source_run_id: run.run_id.clone(),
            source_snapshot_ref: if unsupported {
                LocalProposalSnapshotReference {
                    id: run.snapshot_ref.id.clone(),
                    revision: run.snapshot_ref.revision,
                    digest: run.snapshot_ref.digest.clone(),
                }
            } else {
                assembly.source_snapshot_ref.clone()
            },
            observed_cutoff_rfc3339: run.observed_cutoff_rfc3339.clone(),
            candidate_count: assembly.candidates.len(),
            candidates: assembly
                .candidates
                .iter()
                .map(|candidate| SafeCandidateProvenance {
                    metric_id: candidate.metric_id.clone(),
                    signal_digest: candidate.signal_digest.clone(),
                    summary_commitment: candidate.summary_commitment.clone(),
                })
                .collect(),
            reason,
            readiness,
            provider_invoked: false,
            executable: false,
        };
        let status = match (record.status, record.evidence_binding) {
            (BuilderReadinessStatus::DependencyBlocked, EvidenceBindingStatus::Bound) => {
                "dependency_blocked"
            }
            (BuilderReadinessStatus::NotApplicable, EvidenceBindingStatus::NotApplicable) => {
                "not_applicable"
            }
            _ => unreachable!("only valid typed status pairs are constructed"),
        };
        let detail = if has_candidates {
            format!(
                "candidate_count={};evidence=bound;u20_plan=unavailable_in_local_simulation;e0_safety_oracle=unavailable_in_local_simulation;provider_invoked=false;executable=false",
                record.candidate_count
            )
        } else {
            format!(
                "candidate_count=0;evidence=not_applicable;reason={};provider_invoked=false;executable=false",
                reason.expect("empty candidate set has a reason")
            )
        };
        let event = RunEvent {
            sequence,
            stage: "e0_builder_input_preparation".to_owned(),
            status: status.to_owned(),
            detail,
            observed_cutoff_rfc3339: run.observed_cutoff_rfc3339.clone(),
        };
        Ok((record, event))
    }
}

fn snapshot_matches(
    safe: &LocalProposalSnapshotReference,
    actual: &improvement_engine_core::ArtifactReference,
) -> bool {
    safe.id == actual.id && safe.revision == actual.revision && safe.digest == actual.digest
}

fn candidate_matches(
    candidate: &LocalProposalCandidate,
    run: &LocalRunResult,
    assembly: &LocalProposalAssembly,
) -> bool {
    candidate.source_run_id == run.run_id
        && candidate.source_run_id == assembly.source_run_id
        && candidate.source_snapshot_ref == assembly.source_snapshot_ref
        && candidate.observed_cutoff_rfc3339 == run.observed_cutoff_rfc3339
        && !candidate.metric_id.is_empty()
        && candidate.signal_digest.starts_with("sha256:")
        && candidate.summary_commitment.starts_with("sha256:")
}

#[cfg(test)]
mod tests {
    use super::{
        BuilderReadinessStatus, E0BuilderInputPreparation, EvidenceBindingStatus, PreparationError,
    };
    use improvement_engine_core::ArtifactReference;
    use improvement_engine_core::e0_proposal_assembly::{
        LocalProposalAssembly, LocalProposalAssemblyStatus, LocalProposalCandidate,
        LocalProposalDisposition, LocalProposalSnapshotReference,
    };
    use improvement_engine_core::local_simulation::{LocalRunResult, LocalSourceKind};

    fn run() -> LocalRunResult {
        LocalRunResult {
            run_id: "run_a".into(),
            tenant_id: "tenant_a".into(),
            source_kind: LocalSourceKind::E0,
            manifest_digest: "sha256:manifest".into(),
            snapshot_ref: ArtifactReference {
                tenant_id: "tenant_a".into(),
                id: "snapshot_a".into(),
                revision: 1,
                digest: "sha256:snapshot".into(),
            },
            observed_cutoff_rfc3339: "2025-07-01T00:00:00Z".into(),
            execution_mode: "local_simulation".into(),
            simulation_version: "test".into(),
            simulation_seed: "seed".into(),
            determinism: "deterministic".into(),
            terminal_status: "complete_simulated".into(),
            formal_route: "do_nothing".into(),
            primary_signal_policy: "local_primary_signal_v3".into(),
            recurrence_measurement_status: "observed".into(),
            discovery_case_count: 10,
            excluded_replay_case_count: 0,
            u12_e_u13_e: None,
            signal: None,
            signals: Vec::new(),
            local_simulation_portfolio: None,
            candidates: Vec::new(),
            verification_status: None,
            proposal: None,
            evaluation: None,
            contact_volume_projection: None,
            snapshot_descriptive_envelope: None,
            events: Vec::new(),
        }
    }

    fn snapshot() -> LocalProposalSnapshotReference {
        LocalProposalSnapshotReference {
            id: "snapshot_a".into(),
            revision: 1,
            digest: "sha256:snapshot".into(),
        }
    }

    fn candidate() -> LocalProposalCandidate {
        LocalProposalCandidate {
            proposal_ref: "run_a:sha256:signal".into(),
            source_run_id: "run_a".into(),
            source_snapshot_ref: snapshot(),
            observed_cutoff_rfc3339: "2025-07-01T00:00:00Z".into(),
            metric_id: "e0_recurring_copilot_query_cases".into(),
            signal_digest: "sha256:signal".into(),
            summary_commitment: "sha256:summary".into(),
            detector_policy_id: "local_primary_signal_v3".into(),
            detector_policy_version: 3,
            hypothesis: "test only".into(),
            claim_level: "descriptive_only".into(),
            route_status: "unlinked".into(),
            evaluation_status: "not_evaluated".into(),
            business_lift: None,
            native_agent_core_status: "not_connected".into(),
        }
    }

    fn assembly(candidates: Vec<LocalProposalCandidate>) -> LocalProposalAssembly {
        LocalProposalAssembly {
            status: if candidates.is_empty() {
                LocalProposalAssemblyStatus::NoQualifyingSignals
            } else {
                LocalProposalAssemblyStatus::CandidatesReady
            },
            source_family: "e0".into(),
            authority: "simulator_only".into(),
            source_run_id: "run_a".into(),
            source_snapshot_ref: snapshot(),
            observed_cutoff_rfc3339: "2025-07-01T00:00:00Z".into(),
            primary_signal_digest: None,
            dispositions: vec![LocalProposalDisposition {
                metric_id: "e0_recurring_copilot_query_cases".into(),
                signal_digest: None,
                summary_commitment: None,
                state: "not_qualified".into(),
                reason: "below_support_floor".into(),
            }],
            candidates,
            business_lift: None,
        }
    }

    #[test]
    fn candidate_evidence_is_bound_but_readiness_stays_blocked_without_real_u20() {
        let (status, event) =
            E0BuilderInputPreparation::from_run(&run(), &assembly(vec![candidate()]), 1).unwrap();
        assert_eq!(status.status, BuilderReadinessStatus::DependencyBlocked);
        assert_eq!(status.evidence_binding, EvidenceBindingStatus::Bound);
        assert_eq!(status.candidate_count, 1);
        assert_eq!(status.readiness.u20_plan, "unavailable_in_local_simulation");
        assert_eq!(
            status.readiness.e0_safety_oracle,
            "unavailable_in_local_simulation"
        );
        assert!(!status.provider_invoked);
        assert!(!status.executable);
        let encoded = serde_json::to_string(&status).unwrap();
        assert!(!encoded.contains("tenant_a"));
        assert!(!encoded.contains("test only"));
        assert_eq!(event.stage, "e0_builder_input_preparation");
        assert_eq!(event.status, "dependency_blocked");
        assert!(event.detail.contains("evidence=bound"));
        assert!(event.detail.contains("candidate_count=1"));
    }

    #[test]
    fn no_candidate_is_explicitly_not_applicable_not_dependency_failure() {
        let (status, event) =
            E0BuilderInputPreparation::from_run(&run(), &assembly(Vec::new()), 1).unwrap();
        assert_eq!(status.status, BuilderReadinessStatus::NotApplicable);
        assert_eq!(
            status.evidence_binding,
            EvidenceBindingStatus::NotApplicable
        );
        assert_eq!(status.candidate_count, 0);
        assert_eq!(event.status, "not_applicable");
        assert!(event.detail.contains("reason=no_qualifying_candidate"));
    }

    #[test]
    fn cross_run_candidate_binding_is_rejected() {
        let mut candidate = candidate();
        candidate.source_run_id = "run_other".into();
        assert_eq!(
            E0BuilderInputPreparation::from_run(&run(), &assembly(vec![candidate]), 1).unwrap_err(),
            PreparationError::InvalidCandidateProvenance
        );
    }

    #[test]
    fn cross_tenant_snapshot_binding_is_rejected() {
        let mut run = run();
        run.snapshot_ref.tenant_id = "tenant_other".into();
        assert_eq!(
            E0BuilderInputPreparation::from_run(&run, &assembly(vec![candidate()]), 1).unwrap_err(),
            PreparationError::TenantMismatch
        );
    }

    #[test]
    fn changed_snapshot_and_cutoff_are_rejected() {
        let mut snapshot_assembly = assembly(vec![candidate()]);
        snapshot_assembly.source_snapshot_ref.digest = "sha256:other".into();
        assert_eq!(
            E0BuilderInputPreparation::from_run(&run(), &snapshot_assembly, 1).unwrap_err(),
            PreparationError::SnapshotMismatch
        );
        let mut cutoff_assembly = assembly(vec![candidate()]);
        cutoff_assembly.observed_cutoff_rfc3339 = "2025-07-02T00:00:00Z".into();
        assert_eq!(
            E0BuilderInputPreparation::from_run(&run(), &cutoff_assembly, 1).unwrap_err(),
            PreparationError::CutoffMismatch
        );
    }
}
