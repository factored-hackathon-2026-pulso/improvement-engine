//! U35 is a pure, fail-closed builder-readiness gate.
//!
//! It deliberately creates no candidate, registry entry, execution, release,
//! or same-outcome assertion. Those belong to later independently governed
//! boundaries.

use crate::evaluation_plan::EvaluationPlan;
use crate::independent_verifier::{VerificationReport, VerificationStatus};
use crate::workflow_bridge::{LinkGrade, WorkflowBridgeContract};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalEligibility {
    bridge_commitment: String,
    plan_commitment: String,
}

impl FinalEligibility {
    #[must_use]
    pub fn bridge_commitment(&self) -> &str {
        &self.bridge_commitment
    }
    #[must_use]
    pub fn plan_commitment(&self) -> &str {
        &self.plan_commitment
    }
    #[must_use]
    pub fn allows_same_outcome_claim(&self) -> bool {
        false
    }
    #[must_use]
    pub fn authorizes_release(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalEligibilityReason {
    VerificationNotSupported,
    VerificationBridgeMismatch,
    ScopeMismatch,
    SourceSnapshotMismatch,
    BridgeNotMechanismProxy,
    CandidateRouteUnavailable,
    PlanBridgeMismatch,
}

impl FinalEligibilityReason {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::VerificationNotSupported => "verification_not_supported",
            Self::VerificationBridgeMismatch => "verification_bridge_mismatch",
            Self::ScopeMismatch => "scope_mismatch",
            Self::SourceSnapshotMismatch => "source_snapshot_mismatch",
            Self::BridgeNotMechanismProxy => "bridge_not_mechanism_proxy",
            Self::CandidateRouteUnavailable => "candidate_route_unavailable",
            Self::PlanBridgeMismatch => "plan_bridge_mismatch",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalEligibilityDecision {
    Eligible(FinalEligibility),
    Ineligible(Vec<FinalEligibilityReason>),
}

impl FinalEligibilityDecision {
    #[must_use]
    pub fn eligible_for_proposal(&self) -> bool {
        matches!(self, Self::Eligible(_))
    }
    #[must_use]
    pub fn reasons(&self) -> &[FinalEligibilityReason] {
        match self {
            Self::Eligible(_) => &[],
            Self::Ineligible(reasons) => reasons,
        }
    }
}

pub struct FinalEligibilityGate;

impl FinalEligibilityGate {
    #[must_use]
    pub fn decide(
        report: &VerificationReport,
        bridge: &WorkflowBridgeContract,
        plan: &EvaluationPlan,
    ) -> FinalEligibilityDecision {
        let mut reasons = Vec::new();
        if report.status() != VerificationStatus::Supported {
            reasons.push(FinalEligibilityReason::VerificationNotSupported);
        }
        if report.scope() != bridge.scope() {
            reasons.push(FinalEligibilityReason::ScopeMismatch);
        }
        if report.source_snapshot_ref() != bridge.source_snapshot_ref() {
            reasons.push(FinalEligibilityReason::SourceSnapshotMismatch);
        }
        if report.candidate_digest() != bridge.candidate_digest()
            || report.provenance_commitment() != bridge.provenance_commitment()
            || report.input_commitment() != bridge.verification_input_commitment()
            || report.receipt().digest() != bridge.verification_receipt_digest()
        {
            reasons.push(FinalEligibilityReason::VerificationBridgeMismatch);
        }
        if bridge.link_grade() != LinkGrade::MechanismProxy {
            reasons.push(FinalEligibilityReason::BridgeNotMechanismProxy);
        }
        if !bridge.alternatives().includes_candidate_route() {
            reasons.push(FinalEligibilityReason::CandidateRouteUnavailable);
        }
        if !plan.is_bound_to_bridge(bridge) {
            reasons.push(FinalEligibilityReason::PlanBridgeMismatch);
        }
        if reasons.is_empty() {
            FinalEligibilityDecision::Eligible(FinalEligibility {
                bridge_commitment: bridge.commitment().to_owned(),
                plan_commitment: plan.commitment().to_owned(),
            })
        } else {
            FinalEligibilityDecision::Ineligible(reasons)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FinalEligibilityDecision, FinalEligibilityGate, FinalEligibilityReason};
    use crate::core_task::CoreTaskScope;
    use crate::evaluation_plan::{
        EvaluationArtifactGrant, EvaluationArtifactRef, EvaluationInputs,
        InMemoryEvaluationArtifactAuthority, TrustedEvaluationComposer,
    };
    use crate::independent_verifier::{
        VerificationStatus, report_for_workflow_bridge_with_snapshot_and_scope_test,
    };
    use crate::workflow_bridge::report_and_bridge_for_final_eligibility_test;
    use crate::{
        ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository,
        InMemoryArtifactRepository,
    };
    use serde_json::json;

    const OUTCOME: &str = "reduce_repeat_payment_contacts";
    const UNIT: &str = "customer_episode";
    const MEASURE: &str = "scenario_oracle/payment_status_resolution_v1";

    fn snapshot(repo: &mut InMemoryArtifactRepository, id: &str) -> ArtifactReference {
        repo.append(
            None,
            ArtifactDraft::new(
                "tenant_a",
                id,
                1,
                ArtifactKind::SourceSnapshot,
                json!({}),
                None,
            ),
        )
        .expect("fixed source snapshot appends")
        .reference()
    }

    fn scenario(
        repo: &mut InMemoryArtifactRepository,
        id: &str,
        artifact_type: &str,
        partition: &str,
        source_snapshot_ref: ArtifactReference,
    ) -> ArtifactReference {
        repo.append(
            None,
            ArtifactDraft::new(
                "tenant_a",
                id,
                1,
                ArtifactKind::ScenarioSet,
                json!({"evaluation_contract": {
                    "artifact_type": artifact_type,
                    "partition": partition,
                    "target_outcome": OUTCOME,
                    "unit_of_analysis": UNIT,
                    "oracle_measure": MEASURE,
                    "scope": {
                        "tenant_id": "tenant_a",
                        "job_id": "job_a",
                        "grant_id": "grant_a",
                        "authority_ref": "authority_a"
                    }
                }}),
                Some(source_snapshot_ref),
            ),
        )
        .expect("fixed scenario set appends")
        .reference()
    }

    fn seal_plan(
        repo: &mut InMemoryArtifactRepository,
        bridge: &crate::workflow_bridge::WorkflowBridgeContract,
        source_snapshot_ref: ArtifactReference,
        id_suffix: &str,
    ) -> crate::evaluation_plan::EvaluationPlan {
        let baseline = scenario(
            repo,
            &format!("018f0f4e-7bbd-7000-8000-000000000{}1", id_suffix),
            "baseline",
            "shared",
            source_snapshot_ref.clone(),
        );
        let oracle = scenario(
            repo,
            &format!("018f0f4e-7bbd-7000-8000-000000000{}2", id_suffix),
            "oracle",
            "shared",
            source_snapshot_ref.clone(),
        );
        let development_suite = scenario(
            repo,
            &format!("018f0f4e-7bbd-7000-8000-000000000{}3", id_suffix),
            "development_suite",
            "development",
            source_snapshot_ref.clone(),
        );
        let final_suite = scenario(
            repo,
            &format!("018f0f4e-7bbd-7000-8000-000000000{}4", id_suffix),
            "final_suite",
            "final",
            source_snapshot_ref,
        );
        let inputs = EvaluationInputs::new(
            EvaluationArtifactRef::baseline(baseline.clone()),
            EvaluationArtifactRef::oracle(oracle.clone()),
            EvaluationArtifactRef::development_suite(development_suite.clone()),
            EvaluationArtifactRef::final_suite(final_suite.clone()),
        );
        let mut authority = InMemoryEvaluationArtifactAuthority::default();
        for reference in [&baseline, &oracle, &development_suite, &final_suite] {
            authority.issue(EvaluationArtifactGrant::for_scope(
                bridge.scope().clone(),
                reference.clone(),
            ));
        }
        TrustedEvaluationComposer::from_policy(authority)
            .seal_from_bridge(bridge, inputs, repo)
            .expect("attested semantically matching U20 inputs seal")
    }

    fn supported_triplet(
        repo: &mut InMemoryArtifactRepository,
    ) -> (
        crate::independent_verifier::VerificationReport,
        crate::workflow_bridge::WorkflowBridgeContract,
        crate::evaluation_plan::EvaluationPlan,
        ArtifactReference,
    ) {
        let source_snapshot_ref = snapshot(repo, "018f0f4e-7bbd-7000-8000-000000000500");
        let (report, bridge) = report_and_bridge_for_final_eligibility_test(
            source_snapshot_ref.clone(),
            VerificationStatus::Supported,
        );
        let plan = seal_plan(repo, &bridge, source_snapshot_ref.clone(), "50");
        (report, bridge, plan, source_snapshot_ref)
    }

    #[test]
    fn sealed_u14_u16_u20_triplet_is_builder_ready_but_never_authorizes_outcome_or_release() {
        let mut repo = InMemoryArtifactRepository::default();
        let (report, bridge, plan, _) = supported_triplet(&mut repo);

        let decision = FinalEligibilityGate::decide(&report, &bridge, &plan);
        assert_eq!(
            decision,
            FinalEligibilityGate::decide(&report, &bridge, &plan),
            "pure gate has no mutable side effect or time-dependent result"
        );
        let FinalEligibilityDecision::Eligible(eligibility) = decision else {
            panic!("matching sealed U14/U16/U20 triplet must be builder-ready");
        };
        assert_eq!(eligibility.bridge_commitment(), bridge.commitment());
        assert_eq!(eligibility.plan_commitment(), plan.commitment());
        assert!(!eligibility.allows_same_outcome_claim());
        assert!(!eligibility.authorizes_release());
    }

    #[test]
    fn final_gate_fails_closed_for_refuted_or_uncertain_u14_even_when_other_inputs_match() {
        let mut repo = InMemoryArtifactRepository::default();
        let (_, bridge, plan, source_snapshot_ref) = supported_triplet(&mut repo);

        for status in [VerificationStatus::Refuted, VerificationStatus::Uncertain] {
            let report =
                report_and_bridge_for_final_eligibility_test(source_snapshot_ref.clone(), status).0;
            let decision = FinalEligibilityGate::decide(&report, &bridge, &plan);
            let reasons = decision.reasons();
            assert!(reasons.contains(&FinalEligibilityReason::VerificationNotSupported));
            assert!(reasons.contains(&FinalEligibilityReason::VerificationBridgeMismatch));
        }
    }

    #[test]
    fn final_gate_rejects_cross_scope_tenant_and_snapshot_before_any_builder_can_use_plan() {
        let mut repo = InMemoryArtifactRepository::default();
        let (_, bridge, plan, source_snapshot_ref) = supported_triplet(&mut repo);
        let tenant_b_scope = CoreTaskScope::new("tenant_b", "job_a", "grant_a", "authority_a")
            .expect("fixed test scope is valid");
        let tenant_b_report = report_for_workflow_bridge_with_snapshot_and_scope_test(
            VerificationStatus::Supported,
            source_snapshot_ref.clone(),
            tenant_b_scope,
        );
        let decision = FinalEligibilityGate::decide(&tenant_b_report, &bridge, &plan);
        let reasons = decision.reasons();
        assert!(reasons.contains(&FinalEligibilityReason::ScopeMismatch));
        assert!(reasons.contains(&FinalEligibilityReason::VerificationBridgeMismatch));

        let other_snapshot = snapshot(&mut repo, "018f0f4e-7bbd-7000-8000-000000000510");
        let cross_snapshot_report = report_and_bridge_for_final_eligibility_test(
            other_snapshot,
            VerificationStatus::Supported,
        )
        .0;
        let decision = FinalEligibilityGate::decide(&cross_snapshot_report, &bridge, &plan);
        let reasons = decision.reasons();
        assert!(reasons.contains(&FinalEligibilityReason::SourceSnapshotMismatch));
        assert!(reasons.contains(&FinalEligibilityReason::VerificationBridgeMismatch));
    }

    #[test]
    fn final_gate_rejects_a_u20_plan_sealed_for_another_u16_bridge() {
        let mut repo = InMemoryArtifactRepository::default();
        let (report_a, bridge_a, _plan_a, _) = supported_triplet(&mut repo);
        let source_snapshot_b = snapshot(&mut repo, "018f0f4e-7bbd-7000-8000-000000000520");
        let (_, bridge_b) = report_and_bridge_for_final_eligibility_test(
            source_snapshot_b.clone(),
            VerificationStatus::Supported,
        );
        let plan_b = seal_plan(&mut repo, &bridge_b, source_snapshot_b, "52");

        let decision = FinalEligibilityGate::decide(&report_a, &bridge_a, &plan_b);
        let reasons = decision.reasons();
        assert_eq!(reasons, [FinalEligibilityReason::PlanBridgeMismatch]);
    }
}
