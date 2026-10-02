use improvement_engine_core::evaluation_plan::{
    EvaluationMetric, EvaluationPlanError, EvaluationSuite, OracleSpec,
};

#[test]
fn evaluation_inputs_reject_invalid_oracle_metric_and_suite_before_a_plan_can_exist() {
    assert_eq!(
        OracleSpec::new(
            "tenant-b",
            "payment_resolution",
            format!("sha256:{}", "a".repeat(64)),
            "not-a-digest",
        ),
        Err(EvaluationPlanError::InvalidOracle)
    );
    assert_eq!(
        EvaluationMetric::new("", "customer_goal"),
        Err(EvaluationPlanError::InvalidMetric)
    );
    assert_eq!(
        EvaluationSuite::new(
            format!("sha256:{}", "c".repeat(64)),
            format!("sha256:{}", "c".repeat(64)),
        ),
        Err(EvaluationPlanError::InvalidSuite)
    );
}
