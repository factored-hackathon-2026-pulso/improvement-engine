#[test]
fn identifies_the_improvement_engine_core() {
    assert_eq!(
        improvement_engine_core::service_name(),
        "improvement-engine-core"
    );
}
