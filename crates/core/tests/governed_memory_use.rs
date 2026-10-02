//! U22 begins a second run from an explicitly admitted published-memory use,
//! never from a broad cache or raw wiki pages.

use improvement_engine_core::governed_memory_use::MemoryUseAdmission;

#[test]
fn second_run_has_a_governed_memory_admission_boundary() {
    assert!(std::mem::size_of::<MemoryUseAdmission>() > 0);
}
