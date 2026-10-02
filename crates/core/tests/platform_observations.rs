use improvement_engine_core::platform_observations::{
    CoreChainVerification, CoreChainVerifierPort, Coverage, CoverageState, DiagnosticEvidence,
    EvidenceKind, InMemoryObservationRepository, InteractionEventKind, ObservationAccess,
    ObservationAuthorizationPort, ObservationBatchContext, ObservationError, ObservationEvent,
    ObservationRepository, ObservationSourceContract, ObservationSourceRegistry,
    PlatformObservationBatch, SourceSamplingMode, TargetSystem, TransportCursor,
    TransportSequenceMode,
};

fn audit_event() -> ObservationEvent {
    ObservationEvent::new(
        "event-1",
        "run-1",
        TargetSystem::Attention,
        EvidenceKind::PlatformAudit,
        InteractionEventKind::ResponseRequested,
        1_759_320_000_000,
        1_759_320_002_000,
    )
    .expect("treated event")
}

fn batch(event: ObservationEvent) -> PlatformObservationBatch {
    PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-2026-10-01",
            "contract:platform-v1",
            TransportCursor::sequence(7, 7),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(1),
            None,
        )
        .expect("declared coverage"),
        vec![event],
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .expect("batch")
}

fn repository() -> InMemoryObservationRepository {
    InMemoryObservationRepository::authorized(access(), authority(), registry()).unwrap()
}

fn registry() -> ObservationSourceRegistry {
    ObservationSourceRegistry::from_trusted_configuration(vec![
        ObservationSourceContract::new(
            "attention-platform",
            "contract:platform-v1",
            TargetSystem::Attention,
            TransportSequenceMode::Contiguous,
            SourceSamplingMode::DurableAudit,
            true,
        )
        .unwrap(),
        ObservationSourceContract::new(
            "otel-collector",
            "contract:otel-v1",
            TargetSystem::Attention,
            TransportSequenceMode::Opaque,
            SourceSamplingMode::SampledDiagnostic,
            false,
        )
        .unwrap(),
        ObservationSourceContract::new(
            "evolution-platform",
            "contract:evolution-v1",
            TargetSystem::Evolution,
            TransportSequenceMode::Contiguous,
            SourceSamplingMode::DurableAudit,
            true,
        )
        .unwrap(),
        ObservationSourceContract::new(
            "agent-core",
            "contract:core-v1",
            TargetSystem::Attention,
            TransportSequenceMode::Contiguous,
            SourceSamplingMode::DurableAudit,
            true,
        )
        .unwrap(),
    ])
    .unwrap()
}

fn access() -> ObservationAccess {
    ObservationAccess::new("tenant-a", "grant-a", "platform_observation").unwrap()
}

struct TestAuthority;
impl ObservationAuthorizationPort for TestAuthority {
    fn authorize(&self, access: &ObservationAccess) -> bool {
        access.tenant_id() == "tenant-a"
            && access.grant_id() == "grant-a"
            && access.purpose() == "platform_observation"
    }
}

fn authority() -> Box<dyn ObservationAuthorizationPort> {
    Box::new(TestAuthority)
}

#[test]
fn exact_replay_preserves_one_tenant_scoped_treated_observation() {
    let mut repository = repository();
    let batch = batch(audit_event());

    let first = repository.ingest(batch.clone()).expect("first delivery");
    let replay = repository.ingest(batch).expect("exact replay");

    assert_eq!(first, replay);
    assert_eq!(
        repository.list("tenant-a").expect("authorized read").len(),
        1
    );
    assert_eq!(
        repository.list("tenant-b"),
        Err(ObservationError::TenantAccessDenied)
    );
}

#[test]
fn an_unregistered_source_contract_cannot_claim_complete_coverage() {
    let mut repository = InMemoryObservationRepository::authorized(
        access(),
        authority(),
        ObservationSourceRegistry::from_trusted_configuration(vec![]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        repository.ingest(batch(audit_event())),
        Err(ObservationError::UnknownSourceContract)
    );
}

#[test]
fn ingest_cannot_elevate_immutable_sampled_source_to_complete_coverage() {
    let sampled = ObservationEvent::new(
        "metric-elevate",
        "run-1",
        TargetSystem::Attention,
        EvidenceKind::OtelMetric,
        InteractionEventKind::ToolResult,
        1_759_320_000_000,
        1_759_320_002_000,
    )
    .unwrap()
    .with_diagnostic(DiagnosticEvidence::Metric {
        name: "latency".into(),
        value_milli: 1,
        unit: "milliseconds".into(),
    })
    .unwrap();
    let asserted_complete = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "otel-collector",
            "partition-elevate",
            "contract:otel-v1",
            TransportCursor::opaque("cursor-elevate"),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(100),
            None,
        )
        .unwrap(),
        vec![sampled],
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .unwrap();
    assert_eq!(
        repository().ingest(asserted_complete),
        Err(ObservationError::CoverageDeclarationMismatch)
    );
}

#[test]
fn registered_source_contract_rejects_cursor_and_sampling_claims_it_cannot_support() {
    let mut repository = repository();
    let opaque = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-1",
            "contract:platform-v1",
            TransportCursor::opaque("cursor-a"),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(1),
            None,
        )
        .unwrap(),
        vec![audit_event()],
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .unwrap();
    assert_eq!(
        repository.ingest(opaque),
        Err(ObservationError::SourceSequenceModeMismatch)
    );

    let sampled = ObservationEvent::new(
        "span-1",
        "run-1",
        TargetSystem::Attention,
        EvidenceKind::OtelSampledSpan,
        InteractionEventKind::ToolResult,
        1_759_320_000_000,
        1_759_320_002_000,
    )
    .unwrap()
    .with_diagnostic(DiagnosticEvidence::SampledSpan {
        trace_ref: "trace-1".into(),
        duration_ms: 2,
    })
    .unwrap();
    assert_eq!(
        repository.ingest(batch(sampled)),
        Err(ObservationError::CoverageDeclarationMismatch)
    );
}

#[test]
fn sampled_trace_and_degraded_collector_cannot_supply_a_population_denominator() {
    let sampled = ObservationEvent::new(
        "span-1",
        "run-1",
        TargetSystem::Attention,
        EvidenceKind::OtelSampledSpan,
        InteractionEventKind::ToolResult,
        1_759_320_000_000,
        1_759_320_002_000,
    )
    .expect("treated sampled span")
    .with_diagnostic(DiagnosticEvidence::SampledSpan {
        trace_ref: "trace-1".into(),
        duration_ms: 2,
    })
    .unwrap();
    let degraded = Coverage::new(
        CoverageState::Degraded,
        "eligible-goals",
        1_759_320_000_000,
        1_759_406_400_000,
        Some(100),
        Some("collector_unavailable".into()),
    )
    .expect("collector outage is explicit");

    let sampled_batch = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "otel-collector",
            "partition-sampled",
            "contract:otel-v1",
            TransportCursor::opaque("cursor-sampled"),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Partial,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            None,
            Some("sampled".into()),
        )
        .unwrap(),
        vec![sampled],
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .unwrap();
    let mut repository = repository();
    repository.ingest(sampled_batch).unwrap();
    let projection = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();
    assert_eq!(
        projection.coverages()[0].denominator_for(&projection.observed_events()[0]),
        None
    );
    assert_eq!(degraded.state(), CoverageState::Degraded);
}

#[test]
fn sampled_observation_requires_typed_diagnostic_evidence() {
    let sampled = ObservationEvent::new(
        "metric-1",
        "run-1",
        TargetSystem::Attention,
        EvidenceKind::OtelMetric,
        InteractionEventKind::ToolResult,
        1_759_320_000_000,
        1_759_320_002_000,
    )
    .unwrap();
    let result = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-2026-10-01",
            "contract:platform-v1",
            TransportCursor::sequence(7, 7),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Partial,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            None,
            Some("sampled".into()),
        )
        .unwrap(),
        vec![sampled],
        1_759_320_002_000,
        1_800_000_000_000,
    );
    assert_eq!(result, Err(ObservationError::InvalidDiagnostic));
}

#[test]
fn event_is_not_visible_before_its_batch_was_observed() {
    let mut repository = repository();
    let delayed = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-2026-10-01",
            "contract:platform-v1",
            TransportCursor::sequence(7, 7),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(1),
            None,
        )
        .unwrap(),
        vec![audit_event()],
        1_759_320_010_000,
        1_800_000_000_000,
    )
    .unwrap();
    repository.ingest(delayed).unwrap();
    let before = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_005_000,
        )
        .unwrap();
    assert!(before.events().is_empty());
    let after = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_010_000,
        )
        .unwrap();
    assert_eq!(after.events().len(), 1);
}

#[test]
fn denominator_is_bound_to_exact_tenant_source_contract_and_batch() {
    let mut first_repository = repository();
    first_repository.ingest(batch(audit_event())).unwrap();
    let projection = first_repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();
    let observed = &projection.observed_events()[0];
    let coverage = &projection.coverages()[0];
    let denominator = coverage
        .denominator_for(observed)
        .expect("same verified batch");
    assert_eq!(denominator.expected_population(), 1);
    assert_eq!(denominator.batch_digest(), observed.batch_digest());
    let another_batch = batch(
        ObservationEvent::new(
            "event-2",
            "run-2",
            TargetSystem::Attention,
            EvidenceKind::PlatformAudit,
            InteractionEventKind::ResponseRequested,
            1_759_320_000_000,
            1_759_320_002_000,
        )
        .unwrap(),
    );
    let mut second_repository = repository();
    second_repository.ingest(another_batch).unwrap();
    let second = second_repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();
    assert_eq!(coverage.denominator_for(&second.observed_events()[0]), None);
}

#[test]
fn replayed_event_in_new_batch_keeps_its_first_accepted_provenance() {
    let first = batch(audit_event());
    let second = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-2026-10-01",
            "contract:platform-v1",
            TransportCursor::sequence(8, 8),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Partial,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            None,
            Some("replay".into()),
        )
        .unwrap(),
        vec![audit_event()],
        1_759_320_010_000,
        1_800_000_000_000,
    )
    .unwrap();
    let mut repository = repository();
    repository.ingest(first.clone()).unwrap();
    repository.ingest(second).unwrap();
    let projection = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_010_000,
        )
        .unwrap();
    assert_eq!(projection.observed_events().len(), 1);
    assert_eq!(
        projection.observed_events()[0].batch_digest(),
        first.batch_digest()
    );
    assert!(
        projection.coverages()[0]
            .denominator_for(&projection.observed_events()[0])
            .is_some()
    );
}

#[test]
fn access_requires_trusted_tenant_grant_and_purpose() {
    struct Authority;
    impl ObservationAuthorizationPort for Authority {
        fn authorize(&self, access: &ObservationAccess) -> bool {
            access.tenant_id() == "tenant-a"
                && access.grant_id() == "grant-a"
                && access.purpose() == "platform_observation"
        }
    }
    let authorized = ObservationAccess::new("tenant-a", "grant-a", "platform_observation").unwrap();
    let wrong_grant =
        ObservationAccess::new("tenant-a", "grant-b", "platform_observation").unwrap();
    let mut repository =
        InMemoryObservationRepository::authorized(authorized, Box::new(Authority), registry())
            .unwrap();
    assert!(repository.list("tenant-a").unwrap().is_empty());
    assert_eq!(
        InMemoryObservationRepository::authorized(wrong_grant, Box::new(Authority), registry())
            .err(),
        Some(ObservationError::TenantAccessDenied),
    );
}

#[test]
fn typed_metric_is_auxiliary_evidence_without_a_population_denominator() {
    let metric = ObservationEvent::new(
        "metric-1",
        "run-1",
        TargetSystem::Attention,
        EvidenceKind::OtelMetric,
        InteractionEventKind::ToolResult,
        1_759_320_000_000,
        1_759_320_002_000,
    )
    .unwrap()
    .with_diagnostic(DiagnosticEvidence::Metric {
        name: "tool_latency".into(),
        value_milli: 2500,
        unit: "milliseconds".into(),
    })
    .unwrap();
    let coverage = Coverage::new(
        CoverageState::Partial,
        "eligible-goals",
        1_759_320_000_000,
        1_759_406_400_000,
        None,
        Some("sampled".into()),
    )
    .unwrap();
    let sampled_batch = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "otel-collector",
            "partition-2026-10-01",
            "contract:otel-v1",
            TransportCursor::opaque("cursor-1"),
        )
        .unwrap(),
        coverage,
        vec![metric.clone()],
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .unwrap();
    let mut repository = repository();
    repository.ingest(sampled_batch).unwrap();
    assert_eq!(repository.list("tenant-a").unwrap(), vec![metric.clone()]);
    let projection = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();
    assert_eq!(
        projection.coverages()[0].denominator_for(&projection.observed_events()[0]),
        None
    );
}

#[test]
fn evolution_telemetry_never_counts_as_customer_attention_population() {
    let engine_event = ObservationEvent::new(
        "engine-event-1",
        "engine-run-1",
        TargetSystem::Evolution,
        EvidenceKind::PlatformAudit,
        InteractionEventKind::ToolResult,
        1_759_320_000_000,
        1_759_320_002_000,
    )
    .unwrap();
    assert_eq!(
        repository().ingest(batch(engine_event.clone())),
        Err(ObservationError::SourceTargetMismatch)
    );
    let mut repository = repository();
    let evolution_batch = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "evolution-platform",
            "partition-evolution",
            "contract:evolution-v1",
            TransportCursor::sequence(0, 0),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "engine-runs",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(100),
            None,
        )
        .unwrap(),
        vec![engine_event],
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .unwrap();
    repository.ingest(evolution_batch).unwrap();
    let projection = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();
    assert_eq!(
        projection.coverages()[0].denominator_for(&projection.observed_events()[0]),
        None
    );
}

#[test]
fn unverified_core_event_cannot_enter_the_durable_observation_projection() {
    let mut repository = repository();
    let core = ObservationEvent::new(
        "core-event-1",
        "core-run-1",
        TargetSystem::Attention,
        EvidenceKind::CoreAudit,
        InteractionEventKind::ModelDecision,
        1_759_320_000_000,
        1_759_320_002_000,
    )
    .expect("typed core projection is not yet chain verification");

    assert_eq!(
        repository.ingest(batch(core)),
        Err(ObservationError::CoreChainUnavailable)
    );
    assert!(repository.list("tenant-a").unwrap().is_empty());
}

struct TrustedCoreVerifier;
impl CoreChainVerifierPort for TrustedCoreVerifier {
    fn verify(&self, _event: &ObservationEvent) -> CoreChainVerification {
        CoreChainVerification::Verified {
            receipt_digest:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        }
    }
}

#[test]
fn verified_core_event_keeps_a_separate_verification_receipt() {
    let mut repository = InMemoryObservationRepository::authorized_with_core_verifier(
        access(),
        authority(),
        registry(),
        Box::new(TrustedCoreVerifier),
    )
    .unwrap();
    let core = ObservationEvent::new(
        "core-1",
        "core-run-1",
        TargetSystem::Attention,
        EvidenceKind::CoreAudit,
        InteractionEventKind::ModelDecision,
        1_759_320_000_000,
        1_759_320_002_000,
    )
    .unwrap()
    .with_source_event_digest(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .unwrap()
    .with_core_run_sequence(0);
    let batch = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "agent-core",
            "core-partition",
            "contract:core-v1",
            TransportCursor::sequence(0, 0),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "core-runs",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(1),
            None,
        )
        .unwrap(),
        vec![core],
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .unwrap();

    let receipt = repository.ingest(batch.clone()).unwrap();
    assert_eq!(receipt.core_verification_refs.len(), 1);
    assert_eq!(receipt.core_verification_refs[0].source_event_id, "core-1");
    assert_eq!(repository.ingest(batch).unwrap(), receipt);
    let projection = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();
    assert_eq!(
        projection.coverages()[0].denominator_for(&projection.observed_events()[0]),
        None,
        "verified Core audit is engine evidence, never a bank-population denominator"
    );
}

#[test]
fn oversized_treated_batch_requires_a_different_authorized_blob_capability() {
    let mut repository = repository();
    let many = (0..2_000)
        .map(|n| {
            ObservationEvent::new(
                format!("event-{n}"),
                "run-1",
                TargetSystem::Attention,
                EvidenceKind::PlatformAudit,
                InteractionEventKind::ToolResult,
                1_759_320_000_000,
                1_759_320_002_000,
            )
            .unwrap()
        })
        .collect();
    let batch = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-1",
            "contract:platform-v1",
            TransportCursor::sequence(0, 1_999),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(2_000),
            None,
        )
        .unwrap(),
        many,
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .unwrap();

    assert_eq!(
        repository.ingest(batch),
        Err(ObservationError::InlineBlobTooLarge)
    );
    assert!(repository.list("tenant-a").unwrap().is_empty());
}

#[test]
fn raw_customer_text_is_rejected_before_any_projection_can_be_persisted() {
    let mut value = serde_json::to_value(audit_event()).unwrap();
    value["customer_text"] = serde_json::Value::String("my account number is 123".into());

    assert!(serde_json::from_value::<ObservationEvent>(value).is_err());
    assert!(
        ObservationEvent::new(
            "person@example.com",
            "run-1",
            TargetSystem::Attention,
            EvidenceKind::PlatformAudit,
            InteractionEventKind::HumanAction,
            1_759_320_000_000,
            1_759_320_002_000,
        )
        .is_err()
    );
}

#[test]
fn late_event_creates_a_new_as_of_projection_without_rewriting_the_old_one() {
    let mut repository = repository();
    repository.ingest(batch(audit_event())).unwrap();
    let before = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();

    let late = ObservationEvent::new(
        "event-2",
        "run-2",
        TargetSystem::Attention,
        EvidenceKind::PlatformAudit,
        InteractionEventKind::HumanAction,
        1_759_320_001_000,
        1_759_320_010_000,
    )
    .unwrap();
    repository
        .ingest(
            PlatformObservationBatch::new(
                ObservationBatchContext::new(
                    "tenant-a",
                    "attention-platform",
                    "partition-2026-10-01",
                    "contract:platform-v1",
                    TransportCursor::sequence(8, 8),
                )
                .unwrap(),
                Coverage::new(
                    CoverageState::Partial,
                    "eligible-goals",
                    1_759_320_000_000,
                    1_759_406_400_000,
                    None,
                    Some("late_delivery".into()),
                )
                .unwrap(),
                vec![late],
                1_759_320_010_000,
                1_800_000_000_000,
            )
            .unwrap(),
        )
        .unwrap();

    let prior = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();
    let after = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_010_000,
        )
        .unwrap();
    assert_eq!(prior, before);
    assert_eq!(before.events().len(), 1);
    assert_eq!(before.coverages().len(), 1);
    assert_eq!(after.events().len(), 2);
    assert_eq!(after.coverages().len(), 2);
    assert_eq!(
        after.coverages()[1].coverage().state(),
        CoverageState::Partial
    );
    assert_ne!(before.digest(), after.digest());
}

#[test]
fn empty_degraded_collector_batch_is_visible_as_missing_coverage_not_zero_failures() {
    let mut repository = repository();
    let outage = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-outage",
            "contract:platform-v1",
            TransportCursor::sequence(20, 20),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Degraded,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            None,
            Some("collector_unavailable".into()),
        )
        .unwrap(),
        Vec::new(),
        1_759_320_020_000,
        1_800_000_000_000,
    )
    .unwrap();
    repository.ingest(outage).unwrap();

    let before = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_019_999,
        )
        .unwrap();
    let after = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_020_000,
        )
        .unwrap();
    assert!(before.coverages().is_empty());
    assert!(after.events().is_empty());
    assert_eq!(after.coverages().len(), 1);
    assert_eq!(
        after.coverages()[0].coverage().state(),
        CoverageState::Degraded
    );
}
