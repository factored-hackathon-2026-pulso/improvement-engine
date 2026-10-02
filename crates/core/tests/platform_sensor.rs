//! U30 deterministic measurement over U29's sealed read projection.
//!
//! These tests intentionally build observations through U29's public,
//! tenant-scoped repository. U30 never reconstructs a batch or coverage claim
//! from raw data.

use improvement_engine_core::platform_observations::{
    Coverage, CoverageState, EvidenceKind, InMemoryObservationRepository, InteractionEventKind,
    Layer, ObservationAccess, ObservationAuthorizationPort, ObservationBatchContext,
    ObservationEvent, ObservationRepository, ObservationSourceContract, ObservationSourceRegistry,
    PlatformObservationBatch, SourceSamplingMode, TargetSystem, TransportCursor,
    TransportSequenceMode,
};
use improvement_engine_core::platform_sensor::{
    LayerMetricSpec, PlatformLayerSensor, PlatformSensorError, PlatformSignalStatus,
};

const WINDOW_START: i64 = 1_759_320_000_000;
const WINDOW_END: i64 = 1_759_406_400_000;
const AS_OF: i64 = 1_759_406_500_000;

struct Authority;

impl ObservationAuthorizationPort for Authority {
    fn authorize(&self, access: &ObservationAccess) -> bool {
        access.tenant_id() == "bank_demo"
            && access.grant_id() == "grant-platform"
            && access.purpose() == "platform_observation"
    }
}

fn registry() -> ObservationSourceRegistry {
    ObservationSourceRegistry::from_trusted_configuration(vec![
        ObservationSourceContract::new(
            "attention-platform",
            "contract:attention-v1",
            TargetSystem::Attention,
            TransportSequenceMode::Contiguous,
            SourceSamplingMode::DurableAudit,
            true,
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
            "otel-platform",
            "contract:otel-v1",
            TargetSystem::Attention,
            TransportSequenceMode::Opaque,
            SourceSamplingMode::SampledDiagnostic,
            false,
        )
        .unwrap(),
        ObservationSourceContract::new(
            "other-attention-platform",
            "contract:attention-alt-v1",
            TargetSystem::Attention,
            TransportSequenceMode::Contiguous,
            SourceSamplingMode::DurableAudit,
            true,
        )
        .unwrap(),
    ])
    .unwrap()
}

fn repository() -> InMemoryObservationRepository {
    InMemoryObservationRepository::authorized(
        ObservationAccess::new("bank_demo", "grant-platform", "platform_observation").unwrap(),
        Box::new(Authority),
        registry(),
    )
    .unwrap()
}

fn event(
    id: &str,
    run: &str,
    target: TargetSystem,
    kind: InteractionEventKind,
    layer: Layer,
) -> ObservationEvent {
    ObservationEvent::new(
        id,
        run,
        target,
        EvidenceKind::PlatformAudit,
        kind,
        WINDOW_START + 1,
        WINDOW_START + 2,
    )
    .unwrap()
    .with_layer(layer)
    .unwrap()
}

fn batch(
    source: &str,
    contract: &str,
    sequence: u64,
    coverage: Coverage,
    events: Vec<ObservationEvent>,
) -> PlatformObservationBatch {
    PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "bank_demo",
            source,
            format!("partition-{sequence}"),
            contract,
            TransportCursor::sequence(sequence, sequence),
        )
        .unwrap(),
        coverage,
        events,
        WINDOW_START + 10,
        WINDOW_END + 1,
    )
    .unwrap()
}

fn complete_coverage(population_ref: &str, expected_population: u64) -> Coverage {
    Coverage::new(
        CoverageState::Complete,
        population_ref,
        WINDOW_START,
        WINDOW_END,
        Some(expected_population),
        None,
    )
    .unwrap()
}

fn measure_signal(
    repository: &mut InMemoryObservationRepository,
) -> improvement_engine_core::platform_sensor::PlatformLayerSignal {
    let projection = repository
        .window_projection("bank_demo", WINDOW_START, WINDOW_END, AS_OF)
        .unwrap();
    let spec =
        LayerMetricSpec::handoff_rate("tree_handoff_rate", 1, Layer::Tree, "tree_goals").unwrap();
    sensor().measure(&spec, &projection).unwrap()
}

fn sensor() -> PlatformLayerSensor {
    PlatformLayerSensor::for_platform_audit()
}

fn measure(repository: &mut InMemoryObservationRepository) -> PlatformSignalStatus {
    measure_signal(repository).status().clone()
}

#[test]
fn measures_attention_handoffs_per_layer_from_one_complete_bound_denominator() {
    let mut repository = repository();
    repository
        .ingest(batch(
            "attention-platform",
            "contract:attention-v1",
            1,
            complete_coverage("tree_goals", 4),
            vec![
                event(
                    "attention-handoff-1",
                    "attention-run-1",
                    TargetSystem::Attention,
                    InteractionEventKind::Handoff,
                    Layer::Tree,
                ),
                event(
                    "attention-handoff-retry",
                    "attention-run-1",
                    TargetSystem::Attention,
                    InteractionEventKind::Handoff,
                    Layer::Tree,
                ),
                event(
                    "attention-tree-attempt",
                    "attention-run-2",
                    TargetSystem::Attention,
                    InteractionEventKind::LayerAttempt,
                    Layer::Tree,
                ),
            ],
        ))
        .unwrap();
    repository
        .ingest(batch(
            "evolution-platform",
            "contract:evolution-v1",
            1,
            complete_coverage("tree_goals", 99),
            vec![event(
                "evolution-handoff",
                "evolution-run-1",
                TargetSystem::Evolution,
                InteractionEventKind::Handoff,
                Layer::Tree,
            )],
        ))
        .unwrap();

    let signal = measure_signal(&mut repository);
    assert_eq!(
        signal.status().clone(),
        PlatformSignalStatus::Measured {
            numerator: 1,
            denominator: 4,
            missing: 0,
        }
    );
    assert_eq!(signal.source_id(), Some("attention-platform"));
    assert_eq!(signal.contract_ref(), Some("contract:attention-v1"));
    assert_eq!(signal.metric_version(), 1);
    assert!(
        signal
            .batch_digest()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(
        signal
            .coverage_evidence_digest()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(signal.digest().starts_with("sha256:"));
    assert!(
        signal
            .metric_mapping_digest()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(signal.mapping_resolution_digest().starts_with("sha256:"));

    let projection = repository
        .window_projection("bank_demo", WINDOW_START, WINDOW_END, AS_OF)
        .unwrap();
    let spec =
        LayerMetricSpec::handoff_rate("tree_handoff_rate", 1, Layer::Tree, "tree_goals").unwrap();
    assert_eq!(signal, sensor().measure(&spec, &projection).unwrap());

    let later_projection = repository
        .window_projection("bank_demo", WINDOW_START, WINDOW_END, AS_OF + 1)
        .unwrap();
    let later = sensor().measure(&spec, &later_projection).unwrap();
    assert_eq!(signal.received_as_of_ms(), AS_OF);
    assert_eq!(later.received_as_of_ms(), AS_OF + 1);
    assert_ne!(signal.projection_digest(), later.projection_digest());
    assert_ne!(signal.digest(), later.digest());
}

#[test]
fn refuses_a_rate_when_coverage_is_partial_or_population_semantics_do_not_match() {
    let mut partial = repository();
    partial
        .ingest(batch(
            "attention-platform",
            "contract:attention-v1",
            1,
            Coverage::new(
                CoverageState::Partial,
                "tree_goals",
                WINDOW_START,
                WINDOW_END,
                None,
                Some("transport_gap".to_owned()),
            )
            .unwrap(),
            vec![event(
                "partial-handoff",
                "attention-run-1",
                TargetSystem::Attention,
                InteractionEventKind::Handoff,
                Layer::Tree,
            )],
        ))
        .unwrap();
    assert_eq!(
        measure(&mut partial),
        PlatformSignalStatus::InsufficientEvidence
    );

    let mut incompatible_population = repository();
    incompatible_population
        .ingest(batch(
            "attention-platform",
            "contract:attention-v1",
            1,
            complete_coverage("all_attention_goals", 4),
            vec![event(
                "wrong-population",
                "attention-run-1",
                TargetSystem::Attention,
                InteractionEventKind::Handoff,
                Layer::Tree,
            )],
        ))
        .unwrap();
    assert_eq!(
        measure(&mut incompatible_population),
        PlatformSignalStatus::InsufficientEvidence
    );
}

#[test]
fn refuses_to_sum_overlapping_or_inconsistent_coverage_claims() {
    let mut repository = repository();
    for (sequence, event_id) in [(1, "first-handoff"), (2, "second-handoff")] {
        repository
            .ingest(batch(
                "attention-platform",
                "contract:attention-v1",
                sequence,
                complete_coverage("tree_goals", 1),
                vec![event(
                    event_id,
                    format!("attention-run-{sequence}").as_str(),
                    TargetSystem::Attention,
                    InteractionEventKind::Handoff,
                    Layer::Tree,
                )],
            ))
            .unwrap();
    }
    assert_eq!(
        measure(&mut repository),
        PlatformSignalStatus::AmbiguousCoverage
    );
}

#[test]
fn refuses_a_rate_when_distinct_handoffs_exceed_the_trusted_population() {
    let mut repository = repository();
    repository
        .ingest(batch(
            "attention-platform",
            "contract:attention-v1",
            1,
            complete_coverage("tree_goals", 1),
            vec![
                event(
                    "first-handoff",
                    "attention-run-1",
                    TargetSystem::Attention,
                    InteractionEventKind::Handoff,
                    Layer::Tree,
                ),
                event(
                    "second-handoff",
                    "attention-run-2",
                    TargetSystem::Attention,
                    InteractionEventKind::Handoff,
                    Layer::Tree,
                ),
            ],
        ))
        .unwrap();

    assert_eq!(
        measure(&mut repository),
        PlatformSignalStatus::InconsistentEvidence
    );
}

#[test]
fn refuses_a_rate_when_the_same_trusted_batch_contains_an_unknown_layer() {
    let mut repository = repository();
    repository
        .ingest(batch(
            "attention-platform",
            "contract:attention-v1",
            1,
            complete_coverage("tree_goals", 2),
            vec![
                event(
                    "tree-handoff",
                    "attention-run-1",
                    TargetSystem::Attention,
                    InteractionEventKind::Handoff,
                    Layer::Tree,
                ),
                event(
                    "unknown-layer",
                    "attention-run-2",
                    TargetSystem::Attention,
                    InteractionEventKind::LayerAttempt,
                    Layer::Unknown,
                ),
            ],
        ))
        .unwrap();

    assert_eq!(
        measure(&mut repository),
        PlatformSignalStatus::InsufficientEvidence
    );
}

#[test]
fn refuses_a_rate_when_the_trusted_batch_omits_a_layer_mapping() {
    let mut repository = repository();
    let unmapped = ObservationEvent::new(
        "unmapped-layer",
        "attention-run-2",
        TargetSystem::Attention,
        EvidenceKind::PlatformAudit,
        InteractionEventKind::LayerAttempt,
        WINDOW_START + 1,
        WINDOW_START + 2,
    )
    .unwrap();
    repository
        .ingest(batch(
            "attention-platform",
            "contract:attention-v1",
            1,
            complete_coverage("tree_goals", 2),
            vec![
                event(
                    "tree-handoff",
                    "attention-run-1",
                    TargetSystem::Attention,
                    InteractionEventKind::Handoff,
                    Layer::Tree,
                ),
                unmapped,
            ],
        ))
        .unwrap();

    assert_eq!(
        measure(&mut repository),
        PlatformSignalStatus::InsufficientEvidence
    );
}

#[test]
fn rejects_an_unknown_layer_or_an_unversioned_metric_spec_before_reading_evidence() {
    assert_eq!(
        LayerMetricSpec::handoff_rate("tree_handoff_rate", 1, Layer::Unknown, "tree_goals"),
        Err(PlatformSensorError::InvalidSpec)
    );
    assert_eq!(
        LayerMetricSpec::handoff_rate("Tree Handoff", 1, Layer::Tree, "tree_goals"),
        Err(PlatformSensorError::InvalidSpec)
    );
    assert_eq!(
        LayerMetricSpec::handoff_rate("tree_handoff_rate", 0, Layer::Tree, "tree_goals"),
        Err(PlatformSensorError::InvalidSpec)
    );
}

#[test]
fn refuses_registered_but_unmapped_source_contracts_and_never_measures_zero_denominators() {
    let mut unmapped = repository();
    unmapped
        .ingest(batch(
            "other-attention-platform",
            "contract:attention-alt-v1",
            1,
            complete_coverage("tree_goals", 1),
            vec![event(
                "unmapped-source-handoff",
                "attention-run-1",
                TargetSystem::Attention,
                InteractionEventKind::Handoff,
                Layer::Tree,
            )],
        ))
        .unwrap();
    assert_eq!(
        measure(&mut unmapped),
        PlatformSignalStatus::InsufficientEvidence
    );

    let mut zero = repository();
    zero.ingest(batch(
        "attention-platform",
        "contract:attention-v1",
        1,
        complete_coverage("tree_goals", 0),
        vec![event(
            "zero-population-attempt",
            "attention-run-1",
            TargetSystem::Attention,
            InteractionEventKind::LayerAttempt,
            Layer::Tree,
        )],
    ))
    .unwrap();
    assert_eq!(
        measure(&mut zero),
        PlatformSignalStatus::InsufficientEvidence
    );
}

#[test]
fn commits_population_semantics_even_when_evidence_is_insufficient() {
    let mut repository = repository();
    let projection = repository
        .window_projection("bank_demo", WINDOW_START, WINDOW_END, AS_OF)
        .unwrap();
    let tree_goals =
        LayerMetricSpec::handoff_rate("tree_handoff_rate", 1, Layer::Tree, "tree_goals").unwrap();
    let different_population = LayerMetricSpec::handoff_rate(
        "tree_handoff_rate",
        1,
        Layer::Tree,
        "tree_goals_excluding_retries",
    )
    .unwrap();
    let sensor = sensor();
    let first = sensor.measure(&tree_goals, &projection).unwrap();
    let second = sensor.measure(&different_population, &projection).unwrap();
    assert_eq!(first.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert_eq!(second.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(first.metric_mapping_digest().is_some());
    assert_eq!(second.metric_mapping_digest(), None);
    assert!(first.mapping_resolution_digest().starts_with("sha256:"));
    assert!(second.mapping_resolution_digest().starts_with("sha256:"));
    assert_ne!(
        first.mapping_resolution_digest(),
        second.mapping_resolution_digest(),
        "the missing-mapping decision is a distinct, sealed receipt"
    );
    assert_ne!(first.population_ref(), second.population_ref());
    assert_ne!(first.digest(), second.digest());
}
