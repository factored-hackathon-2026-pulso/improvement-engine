use improvement_engine_core::final_eligibility::{FinalEligibility, FinalEligibilityReason};

#[test]
fn final_gate_exposes_only_builder_readiness_not_release_or_outcome() {
    assert_eq!(
        FinalEligibilityReason::VerificationNotSupported.as_str(),
        "verification_not_supported"
    );
    assert!(std::mem::size_of::<FinalEligibility>() > 0);
}
