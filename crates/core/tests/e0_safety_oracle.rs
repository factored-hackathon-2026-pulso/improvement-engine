use improvement_engine_core::e0_safety_oracle::E0SafetyOracle;

#[test]
fn e0_safety_oracle_is_a_distinct_opaque_boundary() {
    let _ = std::mem::size_of::<E0SafetyOracle>();
}
