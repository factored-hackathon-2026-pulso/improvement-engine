use improvement_engine_core::ArtifactReference;
use improvement_engine_core::autonomous_scout::{
    AutonomousScout, NonProductionScoutInvocationAuthority, ScoutError, ScoutInvocationAuthority,
    ScoutResult, sealed_expectation_for_test,
};
use improvement_engine_core::core_task::{
    CoreTaskBinding, CoreTaskBindingRegistry, CoreTaskInvocation, CoreTaskPort, CoreTaskScope,
    CoreTaskSimulator, SimulatorDisposition,
};
use improvement_engine_core::deterministic_sensor::{BooleanRateSpec, DeterministicSensor};
use improvement_engine_core::local_lab::{
    InMemoryLabGrantAuthority, InMemoryLabSourceAuthority, LabAccess, LabDataClassification,
    LabGrant, LabQuery, LabSource, LabSourceApprovalPort, LabSourceManifest, LabTable,
    LocalInvestigationLab,
};
use improvement_engine_core::model_provider::{
    HmacProjectionBroker, ModelBudgetLimits, ModelCapability, ModelInvocation, ModelPolicy,
    ModelPort, ModelProvider, ModelProviderSimulator, ProjectionBrokerPort, RedactionPolicy,
};
use std::collections::BTreeMap;

fn d(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}
fn reference() -> ArtifactReference {
    ArtifactReference {
        tenant_id: "tenant_a".into(),
        id: "018f3a54-7eaf-7c83-8a04-5bf4ec1a9d26".into(),
        revision: 1,
        digest: d('a'),
    }
}
fn scope() -> CoreTaskScope {
    CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap()
}
fn signal_named(
    metric_id: &str,
) -> improvement_engine_core::deterministic_sensor::DeterministicSignal {
    signal_named_for_access(metric_id, "grant_a", "authority_a")
}
fn signal_named_for_access(
    metric_id: &str,
    grant_id: &str,
    authority_ref: &str,
) -> improvement_engine_core::deterministic_sensor::DeterministicSignal {
    let access = LabAccess::new(
        "run_a",
        "tenant_a",
        "investigation",
        grant_id,
        authority_ref,
        reference(),
        200,
    );
    let mut grants = InMemoryLabGrantAuthority::default();
    grants.issue(LabGrant::from_access(&access));
    let mut lab = LocalInvestigationLab::new(grants);
    let row = BTreeMap::from([("resolved".into(), "true".into())]);
    let source = LabSource::new(
        LabSourceManifest {
            tenant_id: "tenant_a".into(),
            snapshot_ref: reference(),
            source_contract_digest: d('b'),
            source_digest: d('c'),
            transform_digest: d('d'),
            cutoff_unix_seconds: 100,
            classification: LabDataClassification::Treated,
            safe_for_discovery: true,
        },
        vec![LabTable::new("contacts", vec!["resolved"], vec![row])],
    )
    .unwrap();
    let approved = InMemoryLabSourceAuthority.approve(source).unwrap();
    let session = lab.open(access.clone(), approved, 100).unwrap();
    let result = lab
        .query(
            session.session_id(),
            &access,
            LabQuery::select("contacts", vec!["resolved"], None),
            101,
        )
        .unwrap();
    DeterministicSensor::measure(
        &BooleanRateSpec::new(metric_id, "resolved", "true").unwrap(),
        &[result],
    )
    .unwrap()
}
fn signal() -> improvement_engine_core::deterministic_sensor::DeterministicSignal {
    signal_named("contact_rate")
}
fn core_with(
    input: &str,
    attempt_id: &str,
    release_id: &str,
) -> improvement_engine_core::core_task::CoreTaskReceipt {
    core_with_outcome(input, attempt_id, release_id, "core_a", d('e'))
}
fn core_with_outcome(
    input: &str,
    attempt_id: &str,
    release_id: &str,
    run_id: &str,
    output_digest: String,
) -> improvement_engine_core::core_task::CoreTaskReceipt {
    let binding = CoreTaskBinding::new(
        "scout",
        release_id,
        "0.5.0",
        "53e729d624c8284e906249df84c1a1df84cc8d40",
    )
    .unwrap();
    let mut p =
        CoreTaskSimulator::new(CoreTaskBindingRegistry::new(vec![binding.clone()]).unwrap());
    p.script_success(run_id, output_digest).unwrap();
    p.invoke(CoreTaskInvocation::new(scope(), binding, attempt_id, input).unwrap())
        .unwrap()
}
fn core(input: &str) -> improvement_engine_core::core_task::CoreTaskReceipt {
    core_with(input, "attempt_a", "rel_scout_1")
}
fn core_unknown(input: &str) -> improvement_engine_core::core_task::CoreTaskReceipt {
    let binding = CoreTaskBinding::new(
        "scout",
        "rel_scout_1",
        "0.5.0",
        "53e729d624c8284e906249df84c1a1df84cc8d40",
    )
    .unwrap();
    let mut port =
        CoreTaskSimulator::new(CoreTaskBindingRegistry::new(vec![binding.clone()]).unwrap());
    port.script(SimulatorDisposition::TimeoutAfterDispatch);
    port.invoke(CoreTaskInvocation::new(scope(), binding, "attempt_a", input).unwrap())
        .unwrap()
}
fn model_with_input(
    policy_id: &str,
    capability_revision: &str,
    attempt_id: &str,
    treated_input: &str,
    response: &str,
) -> improvement_engine_core::model_provider::ModelReceipt {
    let cap = ModelCapability::new(
        ModelProvider::OpenRouter,
        "https://openrouter.ai/api/v1",
        "openai/gpt-4.1-mini",
        "secret://pulso/key",
        capability_revision,
    )
    .unwrap();
    let policy = ModelPolicy::with_budget(
        policy_id,
        cap,
        "investigate",
        RedactionPolicy::TokenizeKnownMarkers,
        1,
        100,
        ModelBudgetLimits::new(10, 10, 100).unwrap(),
    )
    .unwrap();
    let mut broker =
        HmacProjectionBroker::new_for_test(b"test-only-projection-authority-key-32b").unwrap();
    let projection = broker
        .authorize_projection(&scope(), &policy, treated_input.into())
        .unwrap();
    let invocation =
        ModelInvocation::from_verified(scope(), policy, attempt_id, projection).unwrap();
    let mut p = ModelProviderSimulator::new(invocation.policy().clone());
    p.script_success(response, "request_a");
    p.invoke(invocation).unwrap()
}
fn model_with_attempt(
    policy_id: &str,
    capability_revision: &str,
    attempt_id: &str,
) -> improvement_engine_core::model_provider::ModelReceipt {
    model_with_input(policy_id, capability_revision, attempt_id, "safe", "ok")
}
fn model_with(
    policy_id: &str,
    capability_revision: &str,
) -> improvement_engine_core::model_provider::ModelReceipt {
    model_with_attempt(policy_id, capability_revision, "attempt_a")
}
fn model() -> improvement_engine_core::model_provider::ModelReceipt {
    model_with("policy_a", "rev_a")
}
fn model_unavailable() -> improvement_engine_core::model_provider::ModelReceipt {
    let cap = ModelCapability::new(
        ModelProvider::OpenRouter,
        "https://openrouter.ai/api/v1",
        "openai/gpt-4.1-mini",
        "secret://pulso/key",
        "rev_a",
    )
    .unwrap();
    let policy = ModelPolicy::with_budget(
        "policy_a",
        cap,
        "investigate",
        RedactionPolicy::TokenizeKnownMarkers,
        1,
        100,
        ModelBudgetLimits::new(10, 10, 100).unwrap(),
    )
    .unwrap();
    let mut broker =
        HmacProjectionBroker::new_for_test(b"test-only-projection-authority-key-32b").unwrap();
    let projection = broker
        .authorize_projection(&scope(), &policy, "safe".into())
        .unwrap();
    let invocation =
        ModelInvocation::from_verified(scope(), policy, "attempt_a", projection).unwrap();
    let mut p = ModelProviderSimulator::new(invocation.policy().clone());
    p.script_dependency_unavailable("offline");
    p.invoke(invocation).unwrap()
}
fn expectation_for(
    signal: &improvement_engine_core::deterministic_sensor::DeterministicSignal,
    core: &improvement_engine_core::core_task::CoreTaskReceipt,
    model: &improvement_engine_core::model_provider::ModelReceipt,
) -> improvement_engine_core::autonomous_scout::ScoutInvocationExpectation {
    sealed_expectation_for_test(scope(), signal, core, model)
}

#[test]
fn scout_emits_three_provenance_bound_candidates() {
    let signal = signal();
    let core = core(&signal.digest);
    let model = model();
    let expectation = expectation_for(&signal, &core, &model);
    match AutonomousScout::discover(&scope(), &expectation, &signal, &core, &model).unwrap() {
        ScoutResult::Candidates(c) => {
            assert_eq!(c.len(), 3);
            assert!(c.iter().all(|d| !d.query_receipt_digests.is_empty()
                && !d.digest.is_empty()
                && d.source_snapshot_ref == reference()
                && d.tenant_id == "tenant_a"
                && d.core_attempt_id == "attempt_a"
                && d.model_attempt_id == "attempt_a"));
        }
        _ => panic!(),
    }
}
#[test]
fn scout_blocks_wrong_scope_before_candidate_publication() {
    let wrong = CoreTaskScope::new("tenant_b", "job_a", "grant_a", "authority_a").unwrap();
    assert_eq!(
        {
            let signal = signal();
            let core = core(&signal.digest);
            let model = model();
            AutonomousScout::discover(
                &wrong,
                &expectation_for(&signal, &core, &model),
                &signal,
                &core,
                &model,
            )
        },
        Err(ScoutError::ScopeMismatch)
    );
}

#[test]
fn scout_rejects_same_tenant_receipts_from_another_grant_or_authority() {
    for signal in [
        signal_named_for_access("contact_rate", "grant_b", "authority_a"),
        signal_named_for_access("contact_rate", "grant_a", "authority_b"),
    ] {
        let core = core(&signal.digest);
        let model = model();
        let mut issuer = NonProductionScoutInvocationAuthority;
        assert_eq!(
            issuer.seal(&scope(), &signal, &core, &model),
            Err(ScoutError::EvidenceDenied)
        );
    }
}

#[test]
fn scout_rejects_tampered_lab_provenance_before_candidate_publication() {
    let original = signal();
    let core = core(&original.digest);
    let model = model();
    let expectation = expectation_for(&original, &core, &model);
    let mut tampered = original;
    tampered.query_receipts[0].digest = d('f');
    assert_eq!(
        AutonomousScout::discover(&scope(), &expectation, &tampered, &core, &model),
        Err(ScoutError::EvidenceDenied)
    );
}

#[test]
fn scout_exposes_unknown_core_dependency_without_drafts() {
    assert_eq!(
        {
            let signal = signal();
            let core = core_unknown(&signal.digest);
            let model = model();
            AutonomousScout::discover(
                &scope(),
                &expectation_for(&signal, &core, &model),
                &signal,
                &core,
                &model,
            )
            .unwrap()
        },
        ScoutResult::DependencyBlocked {
            reason: "core_task_unknown"
        }
    );
}
#[test]
fn scout_blocks_every_non_success_model_outcome_without_drafts() {
    let signal = signal();
    let core = core(&signal.digest);
    let model = model_unavailable();
    assert_eq!(
        AutonomousScout::discover(
            &scope(),
            &expectation_for(&signal, &core, &model),
            &signal,
            &core,
            &model
        )
        .unwrap(),
        ScoutResult::DependencyBlocked {
            reason: "model_dependency_unavailable"
        }
    );
}

#[test]
fn scout_rejects_same_scope_wrong_model_policy_or_capability() {
    let signal = signal();
    let core = core(&signal.digest);
    let expected_model = model();
    let expected = expectation_for(&signal, &core, &expected_model);
    let wrong_policy = model_with("policy_b", "rev_a");
    assert_eq!(
        AutonomousScout::discover(&scope(), &expected, &signal, &core, &wrong_policy),
        Err(ScoutError::EvidenceDenied)
    );
    let wrong_capability = model_with("policy_a", "rev_b");
    assert_eq!(
        AutonomousScout::discover(&scope(), &expected, &signal, &core, &wrong_capability),
        Err(ScoutError::EvidenceDenied)
    );
}

#[test]
fn scout_rejects_same_scope_wrong_core_binding_or_attempt() {
    let signal = signal();
    let expected_core = core(&signal.digest);
    let model = model();
    let expectation = expectation_for(&signal, &expected_core, &model);
    for wrong_core in [
        core_with(&signal.digest, "attempt_a", "rel_scout_2"),
        core_with(&signal.digest, "attempt_b", "rel_scout_1"),
    ] {
        assert_eq!(
            AutonomousScout::discover(&scope(), &expectation, &signal, &wrong_core, &model),
            Err(ScoutError::EvidenceDenied)
        );
    }
}

#[test]
fn scout_rejects_core_input_digest_mismatch_before_candidate_publication() {
    let signal = signal();
    let expected_core = core(&signal.digest);
    let model = model();
    let expectation = expectation_for(&signal, &expected_core, &model);
    let wrong_input = core(&d('f'));
    assert_eq!(
        AutonomousScout::discover(&scope(), &expectation, &signal, &wrong_input, &model),
        Err(ScoutError::EvidenceDenied)
    );
}

#[test]
fn scout_rejects_changed_core_run_or_output_under_the_same_scope_and_attempt() {
    let signal = signal();
    let expected_core = core(&signal.digest);
    let model = model();
    let expectation = expectation_for(&signal, &expected_core, &model);
    for wrong_core in [
        core_with_outcome(&signal.digest, "attempt_a", "rel_scout_1", "core_b", d('e')),
        core_with_outcome(&signal.digest, "attempt_a", "rel_scout_1", "core_a", d('f')),
    ] {
        assert_eq!(
            AutonomousScout::discover(&scope(), &expectation, &signal, &wrong_core, &model),
            Err(ScoutError::EvidenceDenied)
        );
    }
}

#[test]
fn scout_rejects_model_attempt_mismatch_and_signal_replay() {
    let signal = signal();
    let core_receipt = core(&signal.digest);
    let expected_model = model();
    let expectation = expectation_for(&signal, &core_receipt, &expected_model);
    let wrong_attempt = model_with_attempt("policy_a", "rev_a", "attempt_b");
    assert_eq!(
        AutonomousScout::discover(
            &scope(),
            &expectation,
            &signal,
            &core_receipt,
            &wrong_attempt,
        ),
        Err(ScoutError::EvidenceDenied)
    );

    let replayed_signal = signal_named("different_contact_rate");
    let replayed_core = core(&replayed_signal.digest);
    assert_eq!(
        AutonomousScout::discover(
            &scope(),
            &expectation,
            &replayed_signal,
            &replayed_core,
            &expected_model,
        ),
        Err(ScoutError::EvidenceDenied)
    );
}

#[test]
fn scout_rejects_same_policy_model_input_or_receipt_evidence_output_mismatch() {
    let signal = signal();
    let core = core(&signal.digest);
    let expected_model = model();
    let expectation = expectation_for(&signal, &core, &expected_model);
    for mismatched in [
        model_with_input("policy_a", "rev_a", "attempt_a", "other_safe", "ok"),
        model_with_input("policy_a", "rev_a", "attempt_a", "safe", "other_response"),
    ] {
        assert_eq!(
            AutonomousScout::discover(&scope(), &expectation, &signal, &core, &mismatched),
            Err(ScoutError::EvidenceDenied)
        );
    }
}
