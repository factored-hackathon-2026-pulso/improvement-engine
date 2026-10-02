use improvement_engine_core::governed_registry::GovernedRegistryWriter;

#[test]
fn registry_writer_is_a_distinct_governed_boundary() {
    let _ = std::mem::size_of::<GovernedRegistryWriter>();
}
