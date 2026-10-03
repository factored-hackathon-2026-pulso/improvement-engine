#![cfg(feature = "local-simulation")]

use improvement_engine_core::ArtifactReference;
use improvement_engine_core::e0_investigation_plan::{
    E0InvestigationAuthority, E0InvestigationExecutionState, E0InvestigationOption,
    E0InvestigationPlanError, build_e0_investigation_plan,
};
use improvement_engine_core::e0_mechanism_resolution::{
    E0MechanismEvidencePacket, E0RouteCatalog, E0RouteMapping, RouteResolution,
    SupportedCoreFlowRef, resolve_e0_mechanism_route,
};
use improvement_engine_core::e0_proposal_assembly::assemble_e0_proposals;
use improvement_engine_core::local_simulation::{
    LocalObservedQuery, LocalRunInput, LocalRunMetadata, LocalSourceKind, run_local_simulation,
};

fn source_snapshot() -> ArtifactReference {
    ArtifactReference {
        tenant_id: "pulso_local".into(),
        id: "0199b21c-7eab-7000-8000-000000000101".into(),
        revision: 1,
        digest: format!("sha256:{}", "a".repeat(64)),
    }
}

fn recurring_candidate() -> (
    improvement_engine_core::local_simulation::LocalRunResult,
    improvement_engine_core::e0_proposal_assembly::LocalProposalCandidate,
    E0MechanismEvidencePacket,
    RouteResolution,
) {
    recurring_candidate_with_signature(&format!("sha256_{}", "a".repeat(56)))
}

fn recurring_candidate_with_signature(
    query_signature: &str,
) -> (
    improvement_engine_core::local_simulation::LocalRunResult,
    improvement_engine_core::e0_proposal_assembly::LocalProposalCandidate,
    E0MechanismEvidencePacket,
    RouteResolution,
) {
    let result = run_local_simulation(
        LocalRunInput::new(
            LocalRunMetadata::new(
                "run_123_1775000000",
                "pulso_local",
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
                .map(|case_ordinal| {
                    LocalObservedQuery::new(
                        case_ordinal,
                        query_signature.to_owned(),
                        format!("2026-03-01T00:00:{:02}Z", case_ordinal % 60),
                    )
                })
                .collect(),
        ),
    )
    .expect("safe local E0 simulation");
    let assembly = assemble_e0_proposals(&result).expect("descriptive E0 seeds");
    let candidate = assembly
        .candidates
        .into_iter()
        .find(|candidate| candidate.metric_id == "e0_recurring_copilot_query_cases")
        .expect("recurring-query candidate");
    let signal = result
        .signals
        .iter()
        .find(|signal| signal.metric_id == candidate.metric_id)
        .expect("candidate-bound signal summary");
    let packet = E0MechanismEvidencePacket::from_candidate(&result, &candidate, signal)
        .expect("candidate-bound evidence");
    let catalog = E0RouteCatalog::empty(ArtifactReference {
        tenant_id: "pulso_local".into(),
        id: "0199b21c-7eab-7000-8000-000000000202".into(),
        revision: 1,
        digest: E0RouteCatalog::content_digest(&[]),
    })
    .expect("versioned empty catalog");
    let resolution = resolve_e0_mechanism_route(&packet, &catalog);
    (result, candidate, packet, resolution)
}

#[test]
fn unlinked_e0_candidate_gets_read_only_mapping_investigation_and_do_nothing() {
    let (_result, candidate, packet, resolution) = recurring_candidate();
    assert_eq!(resolution.status(), "unlinked");

    let plan = build_e0_investigation_plan(&candidate, &packet, &resolution)
        .expect("candidate-bound read-only plan");

    assert_eq!(plan.schema_version, 1);
    assert_eq!(
        plan.artifact_kind,
        "e0_read_only_investigation_plan_not_agent_core_proposal"
    );
    assert_eq!(plan.proposal_ref, candidate.proposal_ref);
    assert_eq!(plan.signal_digest, candidate.signal_digest);
    assert_eq!(
        plan.decision.recommended_option,
        E0InvestigationOption::InvestigateMapping
    );
    assert_eq!(
        plan.decision.available_options,
        vec![
            E0InvestigationOption::InvestigateMapping,
            E0InvestigationOption::DoNothing
        ]
    );
    assert_eq!(plan.decision.authority, E0InvestigationAuthority::None);
    assert_eq!(
        plan.decision.execution_state,
        E0InvestigationExecutionState::NotExecutable
    );
    assert_eq!(plan.claim_level, "descriptive_only");
    assert_eq!(plan.business_lift, None);
    assert!(plan.validate_integrity().is_ok());
}

#[test]
fn plan_integrity_digest_detects_decision_or_evidence_drift() {
    let (_result, candidate, packet, resolution) = recurring_candidate();
    let plan = build_e0_investigation_plan(&candidate, &packet, &resolution)
        .expect("candidate-bound read-only plan");

    let mut drifted = plan.clone();
    drifted.signal_digest = format!("sha256:{}", "c".repeat(64));
    assert_eq!(
        drifted.validate_integrity(),
        Err(E0InvestigationPlanError::DigestMismatch)
    );

    let mut drifted_decision = plan.clone();
    drifted_decision.decision.recommended_option = E0InvestigationOption::DoNothing;
    assert_eq!(
        drifted_decision.validate_integrity(),
        Err(E0InvestigationPlanError::DigestMismatch)
    );

    let mut drifted_schema = plan.clone();
    drifted_schema.schema_version = 2;
    assert_eq!(
        drifted_schema.validate_integrity(),
        Err(E0InvestigationPlanError::DigestMismatch)
    );

    let mut drifted_snapshot = plan.clone();
    drifted_snapshot.source_snapshot_ref.digest = format!("sha256:{}", "e".repeat(64));
    assert_eq!(
        drifted_snapshot.validate_integrity(),
        Err(E0InvestigationPlanError::DigestMismatch)
    );

    let other_catalog = E0RouteCatalog::empty(ArtifactReference {
        tenant_id: "pulso_local".into(),
        id: "0199b21c-7eab-7000-8000-000000000204".into(),
        revision: 9,
        digest: E0RouteCatalog::content_digest(&[]),
    })
    .expect("another immutable catalog snapshot");
    let other_resolution = resolve_e0_mechanism_route(&packet, &other_catalog);
    let mut drifted_catalog = plan.clone();
    drifted_catalog.route_resolution = other_resolution;
    assert_eq!(
        drifted_catalog.validate_integrity(),
        Err(E0InvestigationPlanError::DigestMismatch)
    );
}

#[test]
fn packet_from_another_candidate_cannot_be_rebound_into_a_plan() {
    let (_result, candidate, packet, resolution) = recurring_candidate();
    let mut drifted_candidate = candidate;
    drifted_candidate.signal_digest = format!("sha256:{}", "d".repeat(64));

    assert_eq!(
        build_e0_investigation_plan(&drifted_candidate, &packet, &resolution),
        Err(E0InvestigationPlanError::CandidateEvidenceMismatch)
    );

    let mut drifted_summary = recurring_candidate().1;
    drifted_summary.summary_commitment = format!("sha256:{}", "e".repeat(64));
    assert_eq!(
        build_e0_investigation_plan(&drifted_summary, &packet, &resolution),
        Err(E0InvestigationPlanError::CandidateEvidenceMismatch)
    );

    let mut drifted_run = recurring_candidate().1;
    drifted_run.source_run_id = "run_124_1775000000".into();
    assert_eq!(
        build_e0_investigation_plan(&drifted_run, &packet, &resolution),
        Err(E0InvestigationPlanError::CandidateEvidenceMismatch)
    );

    let mut drifted_snapshot_candidate = recurring_candidate().1;
    drifted_snapshot_candidate.source_snapshot_ref.digest = format!("sha256:{}", "d".repeat(64));
    assert_eq!(
        build_e0_investigation_plan(&drifted_snapshot_candidate, &packet, &resolution),
        Err(E0InvestigationPlanError::CandidateEvidenceMismatch)
    );
}

#[test]
fn serialized_plan_contains_no_tenant_or_source_query_signature() {
    let (_result, candidate, packet, resolution) = recurring_candidate();
    let plan = build_e0_investigation_plan(&candidate, &packet, &resolution)
        .expect("candidate-bound read-only plan");
    let serialized = serde_json::to_string(&plan).expect("safe plan serialization");

    assert!(!serialized.contains("pulso_local"));
    assert!(!serialized.contains(&format!("sha256_{}", "a".repeat(56))));
    assert!(serialized.contains("investigate_mapping"));
    assert!(serialized.contains("do_nothing"));
}

#[test]
fn identical_candidate_evidence_and_catalog_produce_identical_plan_digest() {
    let (_result, candidate, packet, resolution) = recurring_candidate();
    let first = build_e0_investigation_plan(&candidate, &packet, &resolution)
        .expect("first deterministic plan");
    let second = build_e0_investigation_plan(&candidate, &packet, &resolution)
        .expect("second deterministic plan");

    assert_eq!(first, second);
    assert_eq!(first.plan_digest, second.plan_digest);
}

#[test]
fn mapped_catalog_suggests_only_read_only_exact_flow_investigation() {
    let (_result, candidate, packet, _) = recurring_candidate();
    let flow_ref = SupportedCoreFlowRef::new(
        "86a767474042a566a0dbd6ed23588959f27ebdb3",
        "recurring_query_support",
        "1.0.0",
        format!("sha256:{}", "f".repeat(64)),
    )
    .expect("pinned synthetic Flow reference");
    let mapping = E0RouteMapping::new(
        packet.metric_id(),
        packet.pattern_ref(),
        "flow/recurring-query",
        "recurring-query",
        flow_ref,
    )
    .expect("explicit exact mapping");
    let catalog = E0RouteCatalog::new(
        ArtifactReference {
            tenant_id: "pulso_local".into(),
            id: "0199b21c-7eab-7000-8000-000000000203".into(),
            revision: 2,
            digest: E0RouteCatalog::content_digest(std::slice::from_ref(&mapping)),
        },
        vec![mapping],
    )
    .expect("immutable mapped catalog fixture");
    let resolution = resolve_e0_mechanism_route(&packet, &catalog);
    let plan = build_e0_investigation_plan(&candidate, &packet, &resolution)
        .expect("mapped read-only investigation plan");

    assert_eq!(
        plan.decision.recommended_option,
        E0InvestigationOption::InvestigateMappedFlow
    );
    assert_eq!(
        plan.decision.available_options,
        vec![
            E0InvestigationOption::InvestigateMappedFlow,
            E0InvestigationOption::DoNothing
        ]
    );
    assert_eq!(plan.decision.authority, E0InvestigationAuthority::None);
    assert_eq!(
        plan.decision.execution_state,
        E0InvestigationExecutionState::NotExecutable
    );
    assert!(!resolution.may_compile());
    assert!(!resolution.may_start_sandbox_trial());
    assert_eq!(plan.route_resolution, resolution);
    assert!(plan.validate_integrity().is_ok());
}

#[test]
fn cross_tenant_catalog_resolution_cannot_be_promoted_to_an_investigation_plan() {
    let (_result, candidate, packet, _) = recurring_candidate();
    let foreign_catalog = E0RouteCatalog::empty(ArtifactReference {
        tenant_id: "another_tenant".into(),
        id: "0199b21c-7eab-7000-8000-000000000204".into(),
        revision: 1,
        digest: E0RouteCatalog::content_digest(&[]),
    })
    .expect("separate tenant catalog snapshot");
    let foreign_resolution = resolve_e0_mechanism_route(&packet, &foreign_catalog);

    assert_eq!(
        build_e0_investigation_plan(&candidate, &packet, &foreign_resolution),
        Err(E0InvestigationPlanError::UnsupportedRouteResolution)
    );
}

#[test]
fn mapped_resolution_for_another_packet_cannot_be_rebound_to_candidate_a() {
    let (_result_a, candidate_a, packet_a, _) = recurring_candidate();
    let (_result_b, _candidate_b, packet_b, _) =
        recurring_candidate_with_signature(&format!("sha256_{}", "b".repeat(56)));
    assert_ne!(packet_a.pattern_ref(), packet_b.pattern_ref());

    let flow_ref = SupportedCoreFlowRef::new(
        "86a767474042a566a0dbd6ed23588959f27ebdb3",
        "mapped_pattern_b_flow",
        "1.0.0",
        format!("sha256:{}", "f".repeat(64)),
    )
    .expect("pinned synthetic Flow reference");
    let mapping = E0RouteMapping::new(
        packet_b.metric_id(),
        packet_b.pattern_ref(),
        "flow/packet-b",
        "packet-b",
        flow_ref,
    )
    .expect("explicit exact mapping for packet B");
    let catalog = E0RouteCatalog::new(
        ArtifactReference {
            tenant_id: "pulso_local".into(),
            id: "0199b21c-7eab-7000-8000-000000000206".into(),
            revision: 2,
            digest: E0RouteCatalog::content_digest(std::slice::from_ref(&mapping)),
        },
        vec![mapping],
    )
    .expect("immutable catalog for packet B");
    let resolution_for_b = resolve_e0_mechanism_route(&packet_b, &catalog);

    assert_eq!(
        build_e0_investigation_plan(&candidate_a, &packet_a, &resolution_for_b),
        Err(E0InvestigationPlanError::RouteEvidenceMismatch)
    );
}
