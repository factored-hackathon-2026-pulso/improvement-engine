use improvement_engine_core::core_task::{CoreTaskBinding, CoreTaskScope};
use improvement_engine_core::jev_decision::{
    CalibrationRule, DecisionModelRef, DecisionPolicy, DecisionRoute, JevDecisionError,
    JevDecisionInvocation, JevDecisionPort, JevDecisionSimulator, JevStructuredDecision,
};

const SHA: &str = "53e729d624c8284e906249df84c1a1df84cc8d40";
const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn binding() -> CoreTaskBinding {
    CoreTaskBinding::new("verifier", "rel-verifier-1", "0.5.0", SHA).unwrap()
}
fn scope() -> CoreTaskScope {
    CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap()
}
fn model() -> DecisionModelRef {
    DecisionModelRef::new("decision_verdict", "1.0.0", DIGEST).unwrap()
}
fn policy() -> DecisionPolicy {
    DecisionPolicy::new(
        model(),
        "jev-1.13.0",
        "jev:returned-model",
        "calibration_1",
        vec!["corroborated", "refuted"],
        vec![
            CalibrationRule::new("es", "corroborated", 0.82, 0.91, 0.90).unwrap(),
            CalibrationRule::new("es", "refuted", 0.90, 0.95, 0.90).unwrap(),
        ],
    )
    .unwrap()
}
fn invocation(attempt: &str) -> JevDecisionInvocation {
    JevDecisionInvocation::new(scope(), binding(), policy(), attempt, "es", DIGEST).unwrap()
}

#[test]
fn receipt_binds_model_view_task_model_policy_and_calibration() {
    let mut simulator = JevDecisionSimulator::new(binding(), policy());
    simulator.script(JevStructuredDecision::new("corroborated", 0.82).unwrap());
    let receipt = simulator.decide(invocation("attempt_a")).unwrap();
    assert_eq!(
        receipt.route(),
        &DecisionRoute::Selected("corroborated".into())
    );
    assert_eq!(receipt.task_binding_digest(), binding().digest());
    assert_eq!(receipt.decision_model_digest(), DIGEST);
    assert_eq!(receipt.model_view_digest(), DIGEST);
    assert_eq!(receipt.p_raw(), 0.82);
    assert_eq!(receipt.p_cal(), Some(0.91));
    assert_eq!(receipt.threshold(), Some(0.90));
}

#[test]
fn missing_locale_calibration_routes_low_confidence() {
    let mut simulator = JevDecisionSimulator::new(binding(), policy());
    simulator.script(JevStructuredDecision::new("corroborated", 0.82).unwrap());
    let pt = JevDecisionInvocation::new(scope(), binding(), policy(), "attempt_a", "pt", DIGEST)
        .unwrap();
    assert_eq!(
        simulator.decide(pt).unwrap().route(),
        &DecisionRoute::LowConfidence
    );
}

#[test]
fn invalid_enum_unsorted_duplicate_or_unsupported_policy_are_rejected() {
    assert!(
        DecisionPolicy::new(
            model(),
            "jev-1.13.0",
            "jev:returned-model",
            "calibration_1",
            vec!["z", "a"],
            vec![]
        )
        .is_err()
    );
    assert!(
        DecisionPolicy::new(
            model(),
            "jev-1.13.0",
            "jev:returned-model",
            "calibration_1",
            vec!["a", "a"],
            vec![]
        )
        .is_err()
    );
    assert!(DecisionModelRef::new("decision_verdict", "v1", DIGEST).is_err());
    let mut simulator = JevDecisionSimulator::new(binding(), policy());
    simulator.script(JevStructuredDecision::new("unknown", 0.99).unwrap());
    assert!(matches!(
        simulator.decide(invocation("attempt_a")),
        Err(JevDecisionError::InvalidEnumOutput { .. })
    ));
}

#[test]
fn same_scope_mutation_conflicts_but_same_attempt_is_independent_across_jobs() {
    let mut simulator = JevDecisionSimulator::new(binding(), policy());
    simulator.script(JevStructuredDecision::new("refuted", 0.90).unwrap());
    simulator.decide(invocation("attempt_a")).unwrap();
    let changed = JevDecisionInvocation::new(
        scope(),
        binding(),
        policy(),
        "attempt_a",
        "es",
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .unwrap();
    assert!(matches!(
        simulator.decide(changed),
        Err(JevDecisionError::AttemptConflict { .. })
    ));
    let other = JevDecisionInvocation::new(
        CoreTaskScope::new("tenant_a", "job_b", "grant_a", "authority_a").unwrap(),
        binding(),
        policy(),
        "attempt_a",
        "es",
        DIGEST,
    )
    .unwrap();
    assert!(simulator.decide(other).is_ok());
    let other_tenant = JevDecisionInvocation::new(
        CoreTaskScope::new("tenant_b", "job_a", "grant_a", "authority_a").unwrap(),
        binding(),
        policy(),
        "attempt_a",
        "es",
        DIGEST,
    )
    .unwrap();
    assert!(simulator.decide(other_tenant).is_ok());
}
