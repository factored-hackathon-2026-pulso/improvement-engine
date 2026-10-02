use improvement_engine_core::e0_deterministic_sensor::{
    E0BooleanRateSpec, E0DiagnosticSensor, E0DiagnosticWindow,
};

#[test]
fn exposes_a_bounded_deterministic_e0_diagnostic_contract() {
    let spec = E0BooleanRateSpec::new("contact_resolution", "resolved", "event_time", "true")
        .expect("fixed diagnostic specification");
    let window = E0DiagnosticWindow::new(0, 100).expect("fixed observation window");

    assert_eq!(spec.metric_id(), "contact_resolution");
    assert_eq!(window.start_unix_seconds(), 0);
    assert_eq!(window.end_unix_seconds(), 100);
    assert!(!E0DiagnosticSensor::authorizes_execution_or_release());
}
