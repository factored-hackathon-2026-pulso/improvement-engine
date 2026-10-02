use improvement_engine_core::change_compiler::{
    ChangeCompiler, ChangeOperation, ChangeOperationKind, ChangeSpec, CompilerError, CoreEntityKind,
};
use improvement_engine_core::final_eligibility::{
    FinalEligibilityDecision, FinalEligibilityReason,
};
use serde_json::json;

fn valid_spec() -> ChangeSpec {
    ChangeSpec::new(
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "transaction_status_lookup",
        vec![ChangeOperation::new(
            ChangeOperationKind::Add,
            CoreEntityKind::Flow,
            json!({
                "id": "payment_status_resolution",
                "version": "1.0.0",
                "priority": 1,
                "nodes": [{"id":"complete","type":"end","config":{"outcome":"completed"}}]
            }),
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        )],
    )
    .expect("fixed test ChangeSpec is structurally valid")
}

#[test]
fn ineligible_sealed_readiness_cannot_compile_even_a_structurally_valid_change() {
    let readiness = FinalEligibilityDecision::Ineligible(vec![
        FinalEligibilityReason::VerificationNotSupported,
    ]);

    assert_eq!(
        ChangeCompiler::compile(&readiness, valid_spec()),
        Err(CompilerError::FinalEligibilityRequired)
    );
}
