use improvement_engine_core::autonomous_scout::AutonomousScout;
use improvement_engine_core::core_task::{
    CoreTaskBinding, CoreTaskBindingRegistry, CoreTaskPort, CoreTaskScope, CoreTaskSimulator,
    SimulatorDisposition,
};
use improvement_engine_core::model_provider::{
    HmacProjectionBroker, ModelBudgetLimits, ModelCapability, ModelPolicy, ModelPort,
    ModelProvider, ModelProviderError, ModelProviderSimulator, ProjectionBrokerPort,
    RedactionPolicy, VerifiedProjection,
};
use improvement_engine_core::platform_discovery::{
    PlatformDiscoveryInput, PlatformScoutInvocation, PlatformScoutResult,
};
use improvement_engine_core::platform_observations::{
    Coverage, CoverageState, EvidenceKind, InMemoryObservationRepository, InteractionEventKind,
    Layer, ObservationAccess, ObservationAuthorizationPort, ObservationBatchContext,
    ObservationEvent, ObservationRepository, ObservationSourceContract, ObservationSourceRegistry,
    PlatformObservationBatch, SourceSamplingMode, TargetSystem, TransportCursor,
    TransportSequenceMode,
};
use improvement_engine_core::platform_sensor::{
    ATTENTION_RUN_HANDOFF_RATE_METRIC_ID, ATTENTION_SOURCE_RUNS_POPULATION_REF, LayerMetricSpec,
    PlatformCapabilityProfile, PlatformCaseClosure, PlatformCustomerPopulationRow,
    PlatformLayerSensor, PlatformPopulationStatus, PlatformSignalStatus, PlatformSlaCase,
};
use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository, RepositoryError,
};

const WINDOW_START: i64 = 1_759_320_000_000;
const WINDOW_END: i64 = 1_759_406_400_000;
const AS_OF: i64 = 1_759_406_500_000;
const CONTRACT_SHA: &str = "53e729d624c8284e906249df84c1a1df84cc8d40";

struct PlatformAuthority;

impl ObservationAuthorizationPort for PlatformAuthority {
    fn authorize(&self, access: &ObservationAccess) -> bool {
        access.tenant_id() == "bank_demo"
            && access.grant_id() == "grant-platform"
            && access.purpose() == "platform_observation"
    }
}

fn observed_signal(
    coverage_population: &str,
) -> (
    CoreTaskScope,
    improvement_engine_core::platform_sensor::PlatformLayerSignal,
) {
    let source = ObservationSourceContract::new(
        "attention-platform",
        "contract:attention-v1",
        TargetSystem::Attention,
        TransportSequenceMode::Contiguous,
        SourceSamplingMode::DurableAudit,
        true,
    )
    .unwrap();
    let registry = ObservationSourceRegistry::from_trusted_configuration(vec![source]).unwrap();
    let mut observations = InMemoryObservationRepository::authorized(
        ObservationAccess::new("bank_demo", "grant-platform", "platform_observation").unwrap(),
        Box::new(PlatformAuthority),
        registry,
    )
    .unwrap();
    let event = ObservationEvent::new(
        "handoff-1",
        "platform-run-1",
        TargetSystem::Attention,
        EvidenceKind::PlatformAudit,
        InteractionEventKind::Handoff,
        WINDOW_START + 1,
        WINDOW_START + 2,
    )
    .unwrap()
    .with_layer(Layer::Tree)
    .unwrap();
    let batch = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "bank_demo",
            "attention-platform",
            "partition-1",
            "contract:attention-v1",
            TransportCursor::sequence(1, 1),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            coverage_population,
            WINDOW_START,
            WINDOW_END,
            Some(4),
            None,
        )
        .unwrap(),
        vec![event],
        WINDOW_START + 10,
        WINDOW_END + 1,
    )
    .unwrap();
    observations.ingest(batch).unwrap();
    let projection = observations
        .window_projection("bank_demo", WINDOW_START, WINDOW_END, AS_OF)
        .unwrap();
    let spec = LayerMetricSpec::handoff_rate(
        ATTENTION_RUN_HANDOFF_RATE_METRIC_ID,
        1,
        Layer::Tree,
        ATTENTION_SOURCE_RUNS_POPULATION_REF,
    )
    .unwrap();
    let signal = PlatformLayerSensor::for_platform_audit()
        .measure(&spec, &projection)
        .unwrap();
    let scope = CoreTaskScope::new(
        "bank_demo",
        "job-platform",
        "grant-platform",
        "authority-platform",
    )
    .unwrap();
    (scope, signal)
}

fn observed_input() -> (CoreTaskScope, PlatformDiscoveryInput) {
    let (scope, signal) = observed_signal(ATTENTION_SOURCE_RUNS_POPULATION_REF);
    let input = PlatformDiscoveryInput::from_measured_signal(&signal, &scope).unwrap();
    (scope, input)
}

fn model_policy() -> ModelPolicy {
    let capability = ModelCapability::new(
        ModelProvider::OpenRouter,
        "https://openrouter.ai/api/v1",
        "fixture/scout-model",
        "secret://fixture/model-key",
        "fixture-revision-1",
    )
    .unwrap();
    ModelPolicy::with_budget(
        "platform-scout-policy",
        capability,
        "platform_signal_investigation",
        RedactionPolicy::RejectMarkedInput,
        0,
        1_000,
        ModelBudgetLimits::new(4_096, 1_024, 100_000).unwrap(),
    )
    .unwrap()
}

#[test]
fn run_level_handoff_rate_rejects_goal_grain_coverage() {
    let (scope, run_coverage_signal) = observed_signal(ATTENTION_SOURCE_RUNS_POPULATION_REF);
    assert!(matches!(
        run_coverage_signal.status(),
        PlatformSignalStatus::Measured { .. }
    ));

    let (goal_scope, goal_coverage_signal) = observed_signal("tree_goals");
    assert_eq!(
        goal_coverage_signal.status(),
        &PlatformSignalStatus::InsufficientEvidence
    );
    assert!(
        PlatformDiscoveryInput::from_measured_signal(&goal_coverage_signal, &goal_scope).is_err()
    );
    assert_eq!(scope.tenant_id(), goal_scope.tenant_id());
}

#[test]
fn measured_u30_evidence_yields_only_a_bounded_platform_scout_candidate() {
    let (scope, input) = observed_input();
    let binding = CoreTaskBinding::new("scout", "scout-release", "0.5.0", CONTRACT_SHA).unwrap();
    let mut broker =
        HmacProjectionBroker::new_for_test(b"test-only-platform-scout-projection-key-32-bytes")
            .unwrap();
    let invocation = PlatformScoutInvocation::prepare(
        scope.clone(),
        &input,
        binding.clone(),
        "core-attempt-1",
        model_policy(),
        "model-attempt-1",
        &mut broker,
    )
    .unwrap();

    let mut core = CoreTaskSimulator::new(CoreTaskBindingRegistry::new(vec![binding]).unwrap());
    core.script_success("core-run-1", format!("sha256:{}", "a".repeat(64)))
        .unwrap();
    let core_receipt = core.invoke(invocation.core_invocation().clone()).unwrap();

    let mut model = ModelProviderSimulator::new(invocation.model_invocation().policy().clone());
    model.script_success("fixture output ignored by the evidence packet", "req-1");
    let model_receipt = model.invoke(invocation.model_invocation().clone()).unwrap();

    let result =
        AutonomousScout::discover_platform(&input, &invocation, &core_receipt, &model_receipt)
            .unwrap();
    let PlatformScoutResult::Candidate(candidate) = result else {
        panic!("successful bound receipts should yield a descriptive candidate");
    };

    assert_eq!(candidate.tenant_id(), "bank_demo");
    assert_eq!(candidate.job_id(), "job-platform");
    assert_eq!(candidate.core_run_id(), "core-run-1");
    assert_eq!(candidate.metric_id(), ATTENTION_RUN_HANDOFF_RATE_METRIC_ID);
    assert_eq!((candidate.numerator(), candidate.denominator()), (1, 4));
    let explanation = candidate.explanation();
    assert_eq!(explanation.layer_label(), "tree");
    assert_eq!(explanation.numerator(), 1);
    assert_eq!(explanation.denominator(), 4);
    assert_eq!(explanation.missing(), 0);
    assert_eq!(explanation.rate_basis_points(), 2_500);
    assert_eq!(explanation.window_start_ms(), WINDOW_START);
    assert_eq!(explanation.window_end_ms(), WINDOW_END);
    assert_eq!(explanation.received_as_of_ms(), AS_OF);
    assert_eq!(
        explanation.metric_id(),
        ATTENTION_RUN_HANDOFF_RATE_METRIC_ID
    );
    assert_eq!(
        explanation.population_ref(),
        ATTENTION_SOURCE_RUNS_POPULATION_REF
    );
    assert_eq!(explanation.source_id(), "attention-platform");
    assert_eq!(explanation.contract_ref(), "contract:attention-v1");
    assert_eq!(
        explanation.coverage_evidence_digest(),
        input.coverage_evidence_digest()
    );
    assert_eq!(
        explanation.mapping_resolution_digest(),
        input.mapping_resolution_digest()
    );
    assert_eq!(
        explanation.signal_digest(),
        candidate.platform_signal_digest()
    );
    assert_eq!(
        explanation.eligibility_boundary(),
        candidate.eligibility_boundary()
    );
    assert_eq!(
        explanation.statement(),
        "Observed at least one handoff mapped to the tree layer in 1 of 4 eligible attention-platform source runs (25.00%)."
    );
    assert_eq!(
        explanation.coverage_note(),
        "A measured rate requires complete source coverage. `missing=0` marks this coverage-qualified measurement; it is not a count of failed or missing handoffs."
    );
    assert_eq!(
        explanation.limitation(),
        "This is a descriptive platform measurement; it does not establish cause, customer outcome, or business value."
    );
    let explanation_json = serde_json::to_value(&explanation).unwrap();
    assert!(explanation_json.get("numerator").is_some());
    assert!(explanation_json.get("denominator").is_some());
    assert!(explanation_json.get("coverage_evidence_digest").is_some());
    assert!(explanation_json.get("eligibility_boundary").is_some());
    assert!(explanation_json.get("customer_id").is_none());
    assert_eq!(candidate.platform_signal_digest(), input.signal_digest());
    assert_eq!(candidate.projection_digest(), input.projection_digest());
    assert_eq!(candidate.batch_digest(), input.batch_digest());
    assert_eq!(
        candidate.eligibility_boundary(),
        "not_eligible_for_U13A_U14_requires_platform_independent_verifier"
    );
    assert!(candidate.has_valid_digest());

    let mut artifacts = InMemoryArtifactRepository::default();
    let reference = candidate
        .append_immutable(&mut artifacts, "00000000-0000-7000-8000-000000000001")
        .unwrap();
    let stored = artifacts
        .get("bank_demo", &reference.id, reference.revision)
        .unwrap()
        .expect("the appended immutable signal must be readable");
    assert_eq!(stored.kind, ArtifactKind::Signal);
    assert_eq!(stored.reference(), reference);
    assert!(stored.source_snapshot_ref.is_none());
    assert_eq!(
        stored.payload["platform_input_commitment"],
        input.commitment()
    );
    assert_eq!(
        stored.payload["coverage_evidence_digest"],
        input.coverage_evidence_digest()
    );
    assert_eq!(
        stored.payload["metric_mapping_digest"],
        input.metric_mapping_digest()
    );
    assert_eq!(
        stored.payload["mapping_resolution_digest"],
        input.mapping_resolution_digest()
    );
    assert_eq!(
        stored.payload["eligibility_boundary"],
        candidate.eligibility_boundary()
    );
    assert!(
        !stored
            .payload
            .to_string()
            .contains("fixture output ignored by the evidence packet")
    );

    let idempotent = candidate
        .append_immutable(&mut artifacts, &reference.id)
        .unwrap();
    assert_eq!(idempotent, reference);

    let mut ambiguous = CommitThenReportConflict {
        inner: InMemoryArtifactRepository::default(),
    };
    let recovered = candidate
        .append_immutable(&mut ambiguous, "00000000-0000-7000-8000-000000000002")
        .unwrap();
    assert_eq!(recovered.revision, 1);
    assert!(
        ambiguous
            .get("bank_demo", &recovered.id, recovered.revision)
            .unwrap()
            .is_some()
    );
}

struct CommitThenReportConflict {
    inner: InMemoryArtifactRepository,
}

impl ArtifactRepository for CommitThenReportConflict {
    fn append(
        &mut self,
        expected_head: Option<u64>,
        draft: ArtifactDraft,
    ) -> Result<ArtifactDraft, RepositoryError> {
        let artifact_id = draft.id.clone();
        let appended = self.inner.append(expected_head, draft)?;
        Err(RepositoryError::RevisionConflict {
            artifact_id,
            expected_head,
            actual_head: Some(appended.revision),
        })
    }

    fn get(
        &mut self,
        tenant_id: &str,
        artifact_id: &str,
        revision: u64,
    ) -> Result<Option<ArtifactDraft>, RepositoryError> {
        self.inner.get(tenant_id, artifact_id, revision)
    }
}

struct CountingProjectionBroker {
    inner: HmacProjectionBroker,
    calls: usize,
}

impl ProjectionBrokerPort for CountingProjectionBroker {
    fn authorize_projection(
        &mut self,
        scope: &CoreTaskScope,
        policy: &ModelPolicy,
        treated_input: String,
    ) -> Result<VerifiedProjection, ModelProviderError> {
        self.calls += 1;
        self.inner
            .authorize_projection(scope, policy, treated_input)
    }
}

#[test]
fn cross_tenant_scope_is_rejected_before_projection_or_model_invocation() {
    let (_original_scope, input) = observed_input();
    let wrong_scope = CoreTaskScope::new(
        "other_bank",
        "job-platform",
        "grant-platform",
        "authority-platform",
    )
    .unwrap();
    let mut broker = CountingProjectionBroker {
        inner: HmacProjectionBroker::new_for_test(
            b"test-only-platform-scout-projection-key-32-bytes",
        )
        .unwrap(),
        calls: 0,
    };

    let result = PlatformScoutInvocation::prepare(
        wrong_scope,
        &input,
        CoreTaskBinding::new("scout", "scout-release", "0.5.0", CONTRACT_SHA).unwrap(),
        "core-attempt-cross-tenant",
        model_policy(),
        "model-attempt-cross-tenant",
        &mut broker,
    );

    assert!(result.is_err());
    assert_eq!(
        broker.calls, 0,
        "rejected scope must not reach egress broker"
    );
}

#[test]
fn unknown_core_receipt_blocks_candidate_creation() {
    let (scope, input) = observed_input();
    let binding = CoreTaskBinding::new("scout", "scout-release", "0.5.0", CONTRACT_SHA).unwrap();
    let mut broker =
        HmacProjectionBroker::new_for_test(b"test-only-platform-scout-projection-key-32-bytes")
            .unwrap();
    let invocation = PlatformScoutInvocation::prepare(
        scope,
        &input,
        binding.clone(),
        "core-attempt-unknown",
        model_policy(),
        "model-attempt-unknown",
        &mut broker,
    )
    .unwrap();
    let mut core = CoreTaskSimulator::new(CoreTaskBindingRegistry::new(vec![binding]).unwrap());
    core.script(SimulatorDisposition::TimeoutAfterDispatch);
    let core_receipt = core.invoke(invocation.core_invocation().clone()).unwrap();
    let mut model = ModelProviderSimulator::new(invocation.model_invocation().policy().clone());
    model.script_success("must not produce a candidate", "req-unknown");
    let model_receipt = model.invoke(invocation.model_invocation().clone()).unwrap();

    let result =
        AutonomousScout::discover_platform(&input, &invocation, &core_receipt, &model_receipt)
            .unwrap();

    assert_eq!(
        result,
        PlatformScoutResult::DependencyBlocked {
            reason: "core_task_unknown_or_unsuccessful"
        }
    );
}

#[test]
fn phase_one_profile_rejects_tool_call_until_superset_051() {
    let phase_one = PlatformCapabilityProfile::product_phase_one();
    let superset = PlatformCapabilityProfile::product_superset_0_5_1();

    let unsupported = phase_one.assess_tool_call();
    assert_eq!(unsupported.capability(), "tool_call");
    assert_eq!(unsupported.status(), "unsupported");
    assert!(!unsupported.is_supported());
    assert_eq!(unsupported.profile_version(), "phase-1");

    let supported = superset.assess_tool_call();
    assert_eq!(supported.status(), "supported");
    assert!(supported.is_supported());
    assert_eq!(supported.profile_version(), "0.5.1");
}

#[test]
fn repeated_case_chain_counts_once_and_customer_unresponsive_is_not_failure() {
    let cases = [
        eligible_case("case-a", None, Some(true), Some("error")),
        eligible_case("case-b", None, Some(true), Some("customer_unresponsive")),
        eligible_case("case-c", Some("case-a"), Some(false), Some("resolved")),
        eligible_case("case-d", None, Some(true), Some("error")),
    ];

    let result = PlatformLayerSensor::assess_case_closures(&cases);
    let PlatformSignalStatus::Measured {
        numerator,
        denominator,
        missing,
    } = result.status()
    else {
        panic!("complete case-chain evidence should be measured");
    };

    assert_eq!((*numerator, *denominator, *missing), (1, 3, 0));
    assert_eq!(result.excluded_reason_count("customer_unresponsive"), 1);
    assert!(!result.is_publishable());
}

#[test]
fn missing_case_chain_fields_stop_at_insufficient_evidence() {
    let cases = [
        eligible_case("case-a", None, Some(true), Some("resolved")).without_previous_case_field()
    ];

    let result = PlatformLayerSensor::assess_case_closures(&cases);

    assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(result.missing_fields().contains(&"previous_case_id"));
    assert!(!result.is_publishable());
}

#[test]
fn dangling_previous_case_link_is_insufficient() {
    let cases = [eligible_case(
        "case-a",
        Some("missing-case"),
        Some(true),
        Some("error"),
    )];

    let result = PlatformLayerSensor::assess_case_closures(&cases);

    assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(
        result
            .missing_fields()
            .contains(&"resolved_previous_case_id")
    );
}

#[test]
fn cyclic_previous_case_links_are_insufficient() {
    let cases = [
        eligible_case("case-a", Some("case-b"), Some(true), Some("error")),
        eligible_case("case-b", Some("case-a"), Some(false), Some("resolved")),
    ];

    let result = PlatformLayerSensor::assess_case_closures(&cases);

    assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(
        result
            .missing_fields()
            .contains(&"acyclic_previous_case_id")
    );
}

#[test]
fn branched_previous_case_chain_is_insufficient() {
    let cases = [
        eligible_case("case-a", None, Some(false), Some("resolved")),
        eligible_case("case-b", Some("case-a"), Some(false), Some("resolved")),
        eligible_case("case-c", Some("case-a"), Some(true), Some("error")),
    ];

    let result = PlatformLayerSensor::assess_case_closures(&cases);

    assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(
        result
            .missing_fields()
            .contains(&"unbranched_previous_case_id")
    );
}

#[test]
fn duplicate_case_ids_are_insufficient() {
    let cases = [
        eligible_case("case-a", None, Some(false), Some("resolved")),
        eligible_case("case-a", None, Some(true), Some("error")),
    ];

    let result = PlatformLayerSensor::assess_case_closures(&cases);

    assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(result.missing_fields().contains(&"unique_case_id"));
}

#[test]
fn long_case_chain_resolves_to_one_terminal_without_recursion() {
    let cases = (0..512)
        .rev()
        .map(|index| {
            let previous_id = (index > 0).then(|| format!("case-{}", index - 1));
            eligible_case(
                format!("case-{index}"),
                previous_id.as_deref(),
                Some(false),
                Some("resolved"),
            )
        })
        .collect::<Vec<_>>();

    let result = PlatformLayerSensor::assess_case_closures(&cases);

    assert!(matches!(
        result.status(),
        PlatformSignalStatus::Measured {
            numerator: 0,
            denominator: 1,
            missing: 0
        }
    ));
}

fn eligible_case(
    case_id: impl Into<String>,
    previous_case_id: Option<&str>,
    is_failure: Option<bool>,
    close_reason: Option<&str>,
) -> PlatformCaseClosure {
    let digest = format!("sha256:{}", "d".repeat(64));
    PlatformCaseClosure::new(case_id, previous_case_id, is_failure, close_reason)
        .with_snapshot_provenance(
            Some("cases:v1"),
            Some(&digest),
            Some(50),
            Some(100),
            Some(false),
            Some(EvidenceKind::PlatformAudit),
        )
        .with_available_at_ms(Some(75))
}

#[test]
fn case_closures_require_as_of_closure_time_and_snapshot_provenance() {
    let unprovenanced = [PlatformCaseClosure::new(
        "case-unprovenanced",
        None,
        Some(true),
        Some("error"),
    )];
    let future_closure =
        [
            PlatformCaseClosure::new("case-future-close", None, Some(true), Some("error"))
                .with_snapshot_provenance(
                    Some("cases:v1"),
                    Some(&format!("sha256:{}", "e".repeat(64))),
                    Some(101),
                    Some(100),
                    Some(false),
                    Some(EvidenceKind::PlatformAudit),
                )
                .with_available_at_ms(Some(101)),
        ];

    let unprovenanced_result = PlatformLayerSensor::assess_case_closures(&unprovenanced);
    let future_result = PlatformLayerSensor::assess_case_closures(&future_closure);

    assert_eq!(
        unprovenanced_result.status(),
        &PlatformSignalStatus::InsufficientEvidence
    );
    assert!(
        unprovenanced_result
            .missing_fields()
            .contains(&"source_ref")
    );
    assert!(
        unprovenanced_result
            .missing_fields()
            .contains(&"source_digest")
    );
    assert!(
        unprovenanced_result
            .missing_fields()
            .contains(&"observed_as_of")
    );
    assert!(unprovenanced_result.missing_fields().contains(&"closed_at"));
    assert_eq!(
        future_result.status(),
        &PlatformSignalStatus::InsufficientEvidence
    );
    assert!(
        future_result
            .missing_fields()
            .contains(&"closed_by_observed_as_of")
    );
}

#[test]
fn case_closure_arriving_after_cutoff_is_not_included_in_as_of_measurement() {
    let digest = format!("sha256:{}", "9".repeat(64));
    let late_arrival =
        [
            PlatformCaseClosure::new("case-late-arrival", None, Some(true), Some("error"))
                .with_snapshot_provenance(
                    Some("cases:v1"),
                    Some(&digest),
                    Some(50),
                    Some(100),
                    Some(false),
                    Some(EvidenceKind::PlatformAudit),
                )
                .with_available_at_ms(Some(101)),
        ];

    let result = PlatformLayerSensor::assess_case_closures(&late_arrival);

    assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(
        result
            .missing_fields()
            .contains(&"available_by_observed_as_of")
    );
    assert!(!result.is_publishable());
}

#[test]
fn case_closure_measurement_digest_binds_closure_values_and_observation_cutoff() {
    let digest = format!("sha256:{}", "f".repeat(64));
    let first = [
        PlatformCaseClosure::new("case-private-a", None, Some(true), Some("error"))
            .with_snapshot_provenance(
                Some("cases:v1"),
                Some(&digest),
                Some(50),
                Some(100),
                Some(false),
                Some(EvidenceKind::PlatformAudit),
            )
            .with_available_at_ms(Some(75)),
    ];
    let changed_cutoff =
        [
            PlatformCaseClosure::new("case-private-a", None, Some(true), Some("error"))
                .with_snapshot_provenance(
                    Some("cases:v1"),
                    Some(&digest),
                    Some(50),
                    Some(101),
                    Some(false),
                    Some(EvidenceKind::PlatformAudit),
                )
                .with_available_at_ms(Some(75)),
        ];
    let first_result = PlatformLayerSensor::assess_case_closures(&first);
    let changed_result = PlatformLayerSensor::assess_case_closures(&changed_cutoff);

    assert_eq!(
        first_result.status(),
        &PlatformSignalStatus::Measured {
            numerator: 1,
            denominator: 1,
            missing: 0,
        }
    );
    assert_eq!(first_result.source_ref(), Some("cases:v1"));
    assert_eq!(first_result.source_digest(), Some(digest.as_str()));
    assert_eq!(first_result.observed_as_of_ms(), Some(100));
    let serialized = serde_json::to_value(&first_result).unwrap();
    assert_eq!(serialized["observed_as_of_ms"], 100);
    assert_eq!(serialized["source_ref"], "cases:v1");
    assert_eq!(serialized["source_digest"], digest);
    assert!(!serialized.to_string().contains("case-private-a"));
    assert_ne!(
        first_result.measurement_digest(),
        changed_result.measurement_digest()
    );
    assert!(!format!("{first_result:?}").contains("case-private-a"));
}

#[test]
fn case_closures_reject_mixed_snapshot_references_digests_or_cutoffs() {
    let digest_a = format!("sha256:{}", "7".repeat(64));
    let digest_b = format!("sha256:{}", "8".repeat(64));
    let mismatched = [
        PlatformCaseClosure::new("case-a", None, Some(false), Some("resolved"))
            .with_snapshot_provenance(
                Some("cases:v1"),
                Some(&digest_a),
                Some(50),
                Some(100),
                Some(false),
                Some(EvidenceKind::PlatformAudit),
            )
            .with_available_at_ms(Some(75)),
        PlatformCaseClosure::new("case-b", None, Some(false), Some("resolved"))
            .with_snapshot_provenance(
                Some("cases:v2"),
                Some(&digest_b),
                Some(50),
                Some(101),
                Some(false),
                Some(EvidenceKind::PlatformAudit),
            )
            .with_available_at_ms(Some(75)),
    ];

    let result = PlatformLayerSensor::assess_case_closures(&mismatched);

    assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(result.missing_fields().contains(&"source_ref"));
    assert!(result.missing_fields().contains(&"source_digest"));
    assert!(
        result
            .missing_fields()
            .contains(&"consistent_observed_as_of")
    );
    assert!(!result.is_publishable());
}

#[test]
fn case_closures_exclude_simulators_and_diagnostic_evidence_with_counts() {
    let digest = format!("sha256:{}", "d".repeat(64));
    let cases = [
        eligible_case("case-real", None, Some(true), Some("error")),
        PlatformCaseClosure::new("case-simulator", None, Some(true), Some("error"))
            .with_snapshot_provenance(
                Some("cases:v1"),
                Some(&digest),
                Some(50),
                Some(100),
                Some(true),
                Some(EvidenceKind::PlatformAudit),
            )
            .with_available_at_ms(Some(75)),
        PlatformCaseClosure::new("case-sampled", None, Some(true), Some("error"))
            .with_snapshot_provenance(
                Some("cases:v1"),
                Some(&digest),
                Some(50),
                Some(100),
                Some(false),
                Some(EvidenceKind::OtelSampledSpan),
            )
            .with_available_at_ms(Some(75)),
    ];

    let result = PlatformLayerSensor::assess_case_closures(&cases);

    assert_eq!(
        result.status(),
        &PlatformSignalStatus::Measured {
            numerator: 1,
            denominator: 1,
            missing: 0,
        }
    );
    assert_eq!(result.excluded_reason_count("team_generated"), 1);
    assert_eq!(
        result.excluded_reason_count("non_platform_audit_evidence"),
        1
    );
    let serialized = serde_json::to_value(&result).unwrap();
    assert_eq!(serialized["excluded_reason_counts"]["team_generated"], 1);
    assert!(!serialized.to_string().contains("case-real"));
    assert!(!serialized.to_string().contains("case-simulator"));
    assert!(!serialized.to_string().contains("case-sampled"));
}

#[test]
fn case_closures_fail_closed_when_simulator_or_evidence_kind_is_unknown() {
    let digest = format!("sha256:{}", "2".repeat(64));
    let unknown_simulator = [PlatformCaseClosure::new(
        "case-unknown-simulator",
        None,
        Some(false),
        Some("resolved"),
    )
    .with_snapshot_provenance(
        Some("cases:v1"),
        Some(&digest),
        Some(50),
        Some(100),
        None,
        Some(EvidenceKind::PlatformAudit),
    )
    .with_available_at_ms(Some(75))];
    let unknown_kind =
        [
            PlatformCaseClosure::new("case-unknown-kind", None, Some(false), Some("resolved"))
                .with_snapshot_provenance(
                    Some("cases:v1"),
                    Some(&digest),
                    Some(50),
                    Some(100),
                    Some(false),
                    None,
                )
                .with_available_at_ms(Some(75)),
        ];

    for rows in [&unknown_simulator[..], &unknown_kind[..]] {
        let result = PlatformLayerSensor::assess_case_closures(rows);
        assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
        assert!(!result.is_publishable());
    }
}

#[test]
fn platform_sensor_debug_formats_redact_source_row_identifiers() {
    let case = eligible_case("private-case-7f31", None, Some(false), Some("resolved"));
    let customer = PlatformCustomerPopulationRow::new("private-customer-7f31", Some(false));
    let sla = PlatformSlaCase::new(
        "private-sla-case-7f31",
        Some(10),
        Some(11),
        Some("private-cases-source"),
        Some(&format!("sha256:{}", "3".repeat(64))),
    );

    assert!(!format!("{case:?}").contains("private-case-7f31"));
    assert!(!format!("{customer:?}").contains("private-customer-7f31"));
    assert!(!format!("{sla:?}").contains("private-sla-case-7f31"));
    assert!(!format!("{sla:?}").contains("private-cases-source"));
}

#[test]
fn simulator_customers_are_excluded_and_marked_team_generated() {
    let rows = [
        PlatformCustomerPopulationRow::new("customer-real", Some(false)),
        PlatformCustomerPopulationRow::new("customer-simulator", Some(true)),
    ];

    let result = PlatformLayerSensor::assess_customer_population(&rows);

    assert_eq!(result.status(), &PlatformPopulationStatus::Measured);
    assert_eq!(result.eligible_customer_count(), Some(1));
    assert_eq!(result.team_generated_excluded_count(), 1);
    assert!(!result.is_publishable());
    let serialized = serde_json::to_value(&result).unwrap();
    assert_eq!(serialized["eligible_customer_count"], 1);
    assert_eq!(
        serialized["excluded_reason_counts"],
        serde_json::json!({"team_generated": 1})
    );
    assert!(!serialized.to_string().contains("customer-real"));
    assert!(!serialized.to_string().contains("customer-simulator"));
    assert!(
        !serialized.is_array(),
        "this is an aggregate, not row-level output"
    );
}

#[test]
fn unknown_simulator_flag_stops_population_measurement() {
    let rows = [PlatformCustomerPopulationRow::new("customer-unknown", None)];

    let result = PlatformLayerSensor::assess_customer_population(&rows);

    assert_eq!(
        result.status(),
        &PlatformPopulationStatus::InsufficientEvidence
    );
    assert!(result.missing_fields().contains(&"customers.simulator"));
    assert_eq!(result.eligible_customer_count(), None);
    assert!(!result.is_publishable());
}

#[test]
fn sla_measurement_is_provenance_bound_and_descriptive_only() {
    let source_digest = format!("sha256:{}", "a".repeat(64));
    let rows = [
        PlatformSlaCase::new(
            "case-a",
            Some(100),
            Some(101),
            Some("cases:v1"),
            Some(&source_digest),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit)),
        PlatformSlaCase::new(
            "case-b",
            Some(102),
            Some(101),
            Some("cases:v1"),
            Some(&source_digest),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit)),
    ];

    let result = PlatformLayerSensor::measure_sla_breaches(&rows);

    assert!(matches!(
        result.status(),
        PlatformSignalStatus::Measured {
            numerator: 1,
            denominator: 2,
            missing: 0
        }
    ));
    assert_eq!(result.source_ref(), Some("cases:v1"));
    assert_eq!(result.source_digest(), Some(source_digest.as_str()));
    assert_eq!(result.observed_as_of_ms(), Some(101));
    assert!(result.measurement_digest().is_some());
    let serialized = serde_json::to_value(&result).unwrap();
    assert_eq!(serialized["observed_as_of_ms"], 101);
    assert_eq!(
        serialized["measurement_digest"],
        result.measurement_digest().unwrap()
    );
    assert!(result.statement().to_ascii_lowercase().contains("sla"));
    assert!(
        result
            .statement()
            .to_ascii_lowercase()
            .contains("as of the supplied observation cutoff")
    );
    assert!(
        !result
            .statement()
            .to_ascii_lowercase()
            .contains("regulatory")
    );
    assert!(!result.is_publishable());
}

#[test]
fn sla_deadline_is_not_a_breach_at_the_exact_observation_time() {
    let source_digest = format!("sha256:{}", "b".repeat(64));
    let rows = [PlatformSlaCase::new(
        "case-at-deadline",
        Some(100),
        Some(100),
        Some("cases:v1"),
        Some(&source_digest),
    )
    .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit))];

    let result = PlatformLayerSensor::measure_sla_breaches(&rows);

    assert!(matches!(
        result.status(),
        PlatformSignalStatus::Measured {
            numerator: 0,
            denominator: 1,
            missing: 0
        }
    ));
}

#[test]
fn sla_measurement_rejects_mixed_source_refs_or_digests() {
    let digest_a = format!("sha256:{}", "a".repeat(64));
    let digest_b = format!("sha256:{}", "b".repeat(64));
    let ref_mismatch = [
        PlatformSlaCase::new(
            "case-a",
            Some(100),
            Some(99),
            Some("cases:v1"),
            Some(&digest_a),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit)),
        PlatformSlaCase::new(
            "case-b",
            Some(100),
            Some(99),
            Some("cases:v2"),
            Some(&digest_a),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit)),
    ];
    let digest_mismatch = [
        PlatformSlaCase::new(
            "case-a",
            Some(100),
            Some(99),
            Some("cases:v1"),
            Some(&digest_a),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit)),
        PlatformSlaCase::new(
            "case-b",
            Some(100),
            Some(99),
            Some("cases:v1"),
            Some(&digest_b),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit)),
    ];

    let ref_result = PlatformLayerSensor::measure_sla_breaches(&ref_mismatch);
    let digest_result = PlatformLayerSensor::measure_sla_breaches(&digest_mismatch);

    assert_eq!(
        ref_result.status(),
        &PlatformSignalStatus::InsufficientEvidence
    );
    assert!(ref_result.missing_fields().contains(&"source_ref"));
    assert_eq!(
        serde_json::to_value(&ref_result).unwrap()["observed_as_of_ms"],
        serde_json::Value::Null
    );
    assert_eq!(
        digest_result.status(),
        &PlatformSignalStatus::InsufficientEvidence
    );
    assert!(digest_result.missing_fields().contains(&"source_digest"));
}

#[test]
fn sla_measurement_requires_one_common_observation_cutoff() {
    let source_digest = format!("sha256:{}", "c".repeat(64));
    let rows = [
        PlatformSlaCase::new(
            "case-a",
            Some(100),
            Some(101),
            Some("cases:v1"),
            Some(&source_digest),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit)),
        PlatformSlaCase::new(
            "case-b",
            Some(100),
            Some(102),
            Some("cases:v1"),
            Some(&source_digest),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit)),
    ];

    let result = PlatformLayerSensor::measure_sla_breaches(&rows);

    assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(
        result
            .missing_fields()
            .contains(&"consistent_observed_as_of")
    );
    assert_eq!(result.observed_as_of_ms(), None);
}

#[test]
fn sla_measurement_without_deadline_or_source_provenance_is_insufficient() {
    let rows = [PlatformSlaCase::new("case-a", None, Some(101), None, None)
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit))];

    let result = PlatformLayerSensor::measure_sla_breaches(&rows);

    assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
    assert!(result.missing_fields().contains(&"sla_due_at"));
    assert!(result.missing_fields().contains(&"source_ref"));
    assert!(result.missing_fields().contains(&"source_digest"));
    assert!(!result.is_publishable());
}

#[test]
fn sla_measurement_digest_binds_values_and_cutoff_even_when_source_digest_is_reused() {
    let source_digest = format!("sha256:{}", "4".repeat(64));
    let first = [PlatformSlaCase::new(
        "private-sla-case-a",
        Some(100),
        Some(101),
        Some("cases:v1"),
        Some(&source_digest),
    )
    .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit))];
    let changed_due = [PlatformSlaCase::new(
        "private-sla-case-a",
        Some(101),
        Some(101),
        Some("cases:v1"),
        Some(&source_digest),
    )
    .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit))];
    let changed_cutoff = [PlatformSlaCase::new(
        "private-sla-case-a",
        Some(100),
        Some(102),
        Some("cases:v1"),
        Some(&source_digest),
    )
    .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit))];
    let first_result = PlatformLayerSensor::measure_sla_breaches(&first);
    let changed_result = PlatformLayerSensor::measure_sla_breaches(&changed_due);
    let changed_cutoff_result = PlatformLayerSensor::measure_sla_breaches(&changed_cutoff);

    assert_ne!(
        first_result.measurement_digest(),
        changed_result.measurement_digest()
    );
    assert_ne!(
        first_result.measurement_digest(),
        changed_cutoff_result.measurement_digest()
    );
    assert!(!format!("{first_result:?}").contains("private-sla-case-a"));
}

#[test]
fn sla_measurement_excludes_simulators_and_non_audit_rows_and_counts_them() {
    let digest = format!("sha256:{}", "5".repeat(64));
    let rows = [
        PlatformSlaCase::new(
            "case-real",
            Some(100),
            Some(101),
            Some("cases:v1"),
            Some(&digest),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::PlatformAudit)),
        PlatformSlaCase::new(
            "case-sim",
            Some(100),
            Some(101),
            Some("cases:v1"),
            Some(&digest),
        )
        .with_evidence_metadata(Some(true), Some(EvidenceKind::PlatformAudit)),
        PlatformSlaCase::new(
            "case-sampled",
            Some(100),
            Some(101),
            Some("cases:v1"),
            Some(&digest),
        )
        .with_evidence_metadata(Some(false), Some(EvidenceKind::OtelSampledLog)),
    ];

    let result = PlatformLayerSensor::measure_sla_breaches(&rows);

    assert_eq!(
        result.status(),
        &PlatformSignalStatus::Measured {
            numerator: 1,
            denominator: 1,
            missing: 0,
        }
    );
    assert_eq!(result.excluded_reason_count("team_generated"), 1);
    assert_eq!(
        result.excluded_reason_count("non_platform_audit_evidence"),
        1
    );
    let serialized = serde_json::to_value(&result).unwrap();
    assert_eq!(serialized["excluded_reason_counts"]["team_generated"], 1);
    assert!(!serialized.to_string().contains("case-real"));
    assert!(!serialized.to_string().contains("case-sim"));
    assert!(!serialized.to_string().contains("case-sampled"));
}

#[test]
fn sla_measurement_fails_closed_when_simulator_or_evidence_kind_is_unknown() {
    let digest = format!("sha256:{}", "6".repeat(64));
    let unknown_simulator = [PlatformSlaCase::new(
        "case-unknown-simulator",
        Some(100),
        Some(101),
        Some("cases:v1"),
        Some(&digest),
    )
    .with_evidence_metadata(None, Some(EvidenceKind::PlatformAudit))];
    let unknown_kind = [PlatformSlaCase::new(
        "case-unknown-kind",
        Some(100),
        Some(101),
        Some("cases:v1"),
        Some(&digest),
    )
    .with_evidence_metadata(Some(false), None)];

    for rows in [&unknown_simulator[..], &unknown_kind[..]] {
        let result = PlatformLayerSensor::measure_sla_breaches(rows);
        assert_eq!(result.status(), &PlatformSignalStatus::InsufficientEvidence);
        assert!(!result.is_publishable());
    }
}

#[test]
fn absent_core_target_stops_at_waiting_dependency_without_publish_path() {
    let result = PlatformLayerSensor::check_core_target(None);

    assert_eq!(result.reason(), "missing_core_target");
    assert_eq!(result.dependency_state(), "waiting_dependency");
    assert!(!result.is_publishable());

    let declared = PlatformLayerSensor::check_core_target(Some("core-task:triage"));
    assert_eq!(declared.reason(), "core_target_declared");
    assert_eq!(declared.dependency_state(), "declared_unverified");
    assert!(!declared.is_publishable());
}
