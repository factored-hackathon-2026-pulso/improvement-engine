use improvement_engine_core::e0_deterministic_sensor::{
    DiagnosticMetricPolicy, DiagnosticMetricSpec, E0DiagnosticSensor, E0DiagnosticWindow,
};

#[test]
fn exposes_a_bounded_deterministic_e0_diagnostic_contract() {
    let spec = DiagnosticMetricSpec::from_policy(DiagnosticMetricPolicy::TechnicalErrorRateV1);
    let window = E0DiagnosticWindow::new(0, 100).expect("fixed observation window");

    assert_eq!(spec.metric_id(), "e0_technical_error_rate");
    assert_eq!(spec.policy_id(), "e0_diagnostic_allowlist");
    assert_eq!(spec.policy_version(), 1);
    assert_eq!(window.start_unix_seconds(), 0);
    assert_eq!(window.end_unix_seconds(), 100);
    assert!(!E0DiagnosticSensor::authorizes_execution_or_release());
}
