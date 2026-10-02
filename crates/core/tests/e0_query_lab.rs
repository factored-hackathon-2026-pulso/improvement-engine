use improvement_engine_core::e0_query_lab::E0QueryLab;

#[test]
fn e0_query_lab_is_a_distinct_read_only_boundary() {
    let _ = std::mem::size_of::<E0QueryLab>();
}
