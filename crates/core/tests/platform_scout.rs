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
    PlatformLayerSensor, PlatformSignalStatus,
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
