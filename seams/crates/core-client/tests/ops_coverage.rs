//! Acceptance: all 11 bridge-contract operations are typed, and the typed surface covers exactly the contract routes.
use core_client::{ops::TYPED_OPERATIONS, routes};
use serde_json::Value;
use std::path::PathBuf;

#[test]
fn all_eleven_contract_operations_have_a_typed_method() {
    let c: Value = serde_json::from_str(
        &std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../bridge-contract/contract.json")).unwrap(),
    )
    .unwrap();
    let contract: Vec<String> = c["routes"].as_array().unwrap().iter().map(|r| format!("{} {}", r["method"].as_str().unwrap(), r["path"].as_str().unwrap())).collect();
    assert_eq!(contract.len(), 11);
    let typed: Vec<String> = TYPED_OPERATIONS.iter().map(|(r, _)| format!("{} {}", r.method, r.path)).collect();
    for r in &contract {
        assert!(typed.contains(r), "no typed method for {r}");
    }
    assert_eq!(typed.len(), 11);
    assert_eq!(routes::ALL.len(), 11);
    let mut names: Vec<&str> = TYPED_OPERATIONS.iter().map(|(_, n)| *n).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 11, "one distinct typed method per operation");
}
