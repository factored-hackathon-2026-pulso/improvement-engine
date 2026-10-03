#![cfg(feature = "local-simulation")]

use improvement_engine_core::ArtifactReference;
use improvement_engine_core::e0_mechanism_resolution::{
    E0MechanismEvidenceError, E0MechanismEvidencePacket, E0RouteCatalog, E0RouteCatalogError,
    E0RouteMapping, SupportedCoreFlowRef, resolve_e0_mechanism_route,
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

fn workflow_catalog_ref() -> ArtifactReference {
    ArtifactReference {
        tenant_id: "pulso_local".into(),
        id: "0199b21c-7eab-7000-8000-000000000202".into(),
        revision: 1,
        digest: E0RouteCatalog::content_digest(&[]),
    }
}

/// The query rows have team-generated signatures; they exercise the same
/// recurrence contract as the E0 smoke but are not source-observed records.
struct QueryFixture {
    provenance: &'static str,
    query: LocalObservedQuery,
}

fn recurring_query_fixtures() -> Vec<QueryFixture> {
    (1..=20)
        .map(|case_ordinal| QueryFixture {
            provenance: "team_generated",
            query: LocalObservedQuery::new(
                case_ordinal,
                format!("sha256_{}", "a".repeat(56)),
                format!("2026-03-01T00:00:{:02}Z", case_ordinal % 60),
            ),
        })
        .collect()
}

#[test]
fn recurring_query_candidate_without_explicit_core_mapping_is_unlinked() {
    let query_fixtures = recurring_query_fixtures();
    assert!(
        query_fixtures
            .iter()
            .all(|fixture| fixture.provenance == "team_generated")
    );

    // This constructs a source-shaped contract fixture. It does not ingest
    // E0 files or claim that the generated signature was observed in E0.
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
            query_fixtures
                .into_iter()
                .map(|fixture| fixture.query)
                .collect(),
        ),
    )
    .expect("safe local E0 simulation");
    let assembly = assemble_e0_proposals(&result).expect("descriptive E0 seeds");
    let candidate = assembly
        .candidates
        .iter()
        .find(|candidate| candidate.metric_id == "e0_recurring_copilot_query_cases")
        .expect("recurring-query candidate");
    let signal = result
        .signals
        .iter()
        .find(|signal| signal.metric_id == candidate.metric_id)
        .expect("candidate-bound signal summary");
    assert!(signal.pattern_ref.is_some());

    let packet = E0MechanismEvidencePacket::from_candidate(&result, candidate, signal)
        .expect("candidate-bound recurring-query evidence");
    assert_eq!(packet.metric_id(), "e0_recurring_copilot_query_cases");
    assert_eq!(packet.proposal_ref(), candidate.proposal_ref);
    assert_eq!(packet.signal_digest(), candidate.signal_digest);
    assert_eq!(packet.summary_commitment(), candidate.summary_commitment);
    assert_eq!(
        packet.pattern_ref(),
        signal.pattern_ref.as_deref().expect("pattern commitment")
    );
    assert_eq!(
        packet.source_snapshot_ref().id,
        candidate.source_snapshot_ref.id
    );
    assert_eq!(
        packet.source_snapshot_ref().revision,
        candidate.source_snapshot_ref.revision
    );
    assert_eq!(
        packet.source_snapshot_ref().digest,
        candidate.source_snapshot_ref.digest
    );
    assert_eq!(
        packet.observed_cutoff_rfc3339(),
        candidate.observed_cutoff_rfc3339
    );
    assert!(packet.numerator() >= signal.minimum_support);
    assert_eq!(packet.claim_level(), "descriptive_only");

    let mut tampered_candidate = candidate.clone();
    tampered_candidate.source_snapshot_ref.digest = format!("sha256:{}", "d".repeat(64));
    assert_eq!(
        E0MechanismEvidencePacket::from_candidate(&result, &tampered_candidate, signal),
        Err(E0MechanismEvidenceError::CandidateNotInSourceRun)
    );

    let mut tampered_signal = signal.clone();
    tampered_signal.pattern_ref = Some(format!("sha256:{}", "e".repeat(64)));
    assert_eq!(
        E0MechanismEvidencePacket::from_candidate(&result, candidate, &tampered_signal),
        Err(E0MechanismEvidenceError::CandidateSignalMismatch)
    );

    // An opaque recurring query is evidence of recurrence, not an observed
    // route or proof that a compatible pinned Agent Core Flow exists.
    // Catalog identity is immutable and explicit even when it has no routes;
    // an absent exact mapping must never fall back to a guessed route.
    let catalog =
        E0RouteCatalog::empty(workflow_catalog_ref()).expect("versioned route catalog reference");
    let mut tampered_catalog_ref = workflow_catalog_ref();
    tampered_catalog_ref.digest = format!("sha256:{}", "d".repeat(64));
    assert_eq!(
        E0RouteCatalog::empty(tampered_catalog_ref),
        Err(E0RouteCatalogError::InvalidCatalogReference)
    );
    let mut unsafe_catalog_ref = workflow_catalog_ref();
    unsafe_catalog_ref.id = "secret-catalog-id".into();
    assert_eq!(
        E0RouteCatalog::empty(unsafe_catalog_ref),
        Err(E0RouteCatalogError::InvalidCatalogReference)
    );
    let resolution = resolve_e0_mechanism_route(&packet, &catalog);
    assert_eq!(resolution.status(), "unlinked");
    assert_eq!(resolution.reason(), Some("no_exact_supported_flow_mapping"));
    assert_eq!(resolution.catalog_ref().id(), workflow_catalog_ref().id);
    assert_eq!(
        resolution.catalog_ref().revision(),
        workflow_catalog_ref().revision
    );
    assert_eq!(
        resolution.catalog_ref().digest(),
        workflow_catalog_ref().digest
    );
    assert_eq!(resolution.metric_id(), packet.metric_id());
    assert_eq!(resolution.pattern_ref(), packet.pattern_ref());
    assert!(!resolution.may_compile());
    assert!(!resolution.may_start_sandbox_trial());

    // This deliberately team-generated catalog entry tests exact matching; it
    // is not a real Core registry mapping and grants no compile/trial authority.
    let flow_ref = SupportedCoreFlowRef::new(
        "86a767474042a566a0dbd6ed23588959f27ebdb3",
        "recurring_query_support",
        "1.0.0",
        format!("sha256:{}", "f".repeat(64)),
    )
    .expect("synthetic pinned Flow reference");
    assert_eq!(
        SupportedCoreFlowRef::new(
            "86a767474042a566a0dbd6ed23588959f27ebdb3",
            "customer@example.com",
            "1.0.0",
            format!("sha256:{}", "f".repeat(64)),
        ),
        Err(E0RouteCatalogError::InvalidFlowReference)
    );
    for invalid_version in ["01.0.0", "1.0.0-beta"] {
        assert_eq!(
            SupportedCoreFlowRef::new(
                "86a767474042a566a0dbd6ed23588959f27ebdb3",
                "recurring_query_support",
                invalid_version,
                format!("sha256:{}", "f".repeat(64)),
            ),
            Err(E0RouteCatalogError::InvalidFlowReference)
        );
    }
    assert_eq!(
        SupportedCoreFlowRef::new(
            "86a767474042a566a0dbd6ed23588959f27ebdb3",
            "recurring_query_support",
            "1.0.0\nsecret-flow-version",
            format!("sha256:{}", "f".repeat(64)),
        ),
        Err(E0RouteCatalogError::InvalidFlowReference)
    );
    let mapping = E0RouteMapping::new(
        packet.metric_id(),
        packet.pattern_ref(),
        "flow/recurring-query",
        "recurring-query",
        flow_ref,
    )
    .expect("explicit exact mapping");
    let mapped_catalog_digest = E0RouteCatalog::content_digest(std::slice::from_ref(&mapping));
    let foreign_mapping = mapping.clone();
    let mapped_catalog = E0RouteCatalog::new(
        ArtifactReference {
            tenant_id: "pulso_local".into(),
            id: "0199b21c-7eab-7000-8000-000000000203".into(),
            revision: 2,
            digest: mapped_catalog_digest.clone(),
        },
        vec![mapping],
    )
    .expect("explicitly versioned mapping catalog");
    let mapped = resolve_e0_mechanism_route(&packet, &mapped_catalog);
    assert_eq!(mapped.status(), "mapped");
    assert!(!mapped.may_compile());
    assert!(!mapped.may_start_sandbox_trial());

    // A valid mapping catalog from another tenant is not evidence for this
    // candidate. The resolver must fail closed without exposing tenant scope.
    let foreign_catalog = E0RouteCatalog::new(
        ArtifactReference {
            tenant_id: "another_tenant".into(),
            id: "0199b21c-7eab-7000-8000-000000000204".into(),
            revision: 1,
            digest: mapped_catalog_digest,
        },
        vec![foreign_mapping],
    )
    .expect("same content, separately scoped catalog");
    let foreign_resolution = resolve_e0_mechanism_route(&packet, &foreign_catalog);
    assert_eq!(foreign_resolution.status(), "unlinked");
    assert_eq!(foreign_resolution.reason(), Some("catalog_tenant_mismatch"));
    assert!(!foreign_resolution.may_compile());
    assert!(!foreign_resolution.may_start_sandbox_trial());
    assert!(
        !serde_json::to_string(&foreign_resolution)
            .expect("foreign resolution serialization")
            .contains("another_tenant")
    );

    assert!(
        !serde_json::to_string(&packet)
            .expect("packet serialization")
            .contains(&format!("sha256_{}", "a".repeat(56)))
    );
    assert!(
        !serde_json::to_string(&packet)
            .expect("packet serialization")
            .contains("pulso_local")
    );
    assert!(
        !serde_json::to_string(&resolution)
            .expect("resolution serialization")
            .contains("pulso_local")
    );
}
