use improvement_engine_core::independent_verifier::{
    IndependentVerificationReceipt, VerificationStatus,
};

#[test]
fn receipt_contract_accepts_only_committed_evidence() {
    let digest = format!("sha256:{}", "a".repeat(64));
    assert!(
        IndependentVerificationReceipt::new(
            digest.clone(),
            "independent_evidence",
            "v1",
            digest,
            VerificationStatus::Supported,
        )
        .is_ok()
    );
}
