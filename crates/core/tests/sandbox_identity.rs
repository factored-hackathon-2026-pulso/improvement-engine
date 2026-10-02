use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use improvement_engine_core::sandbox::{
    Action, ActionRequest, FixedSandboxClock, ReadRequest, SandboxClock, SandboxError,
    SandboxFixture, SandboxIdentityPolicy, SandboxPort, SandboxScope, StatefulSandbox,
};

const NOW: u64 = 1_000;

#[derive(Clone)]
struct TestClock(Arc<AtomicU64>);
impl SandboxClock for TestClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}
fn sandbox_at(now: u64) -> (StatefulSandbox, TestClock) {
    let clock = TestClock(Arc::new(AtomicU64::new(now)));
    (StatefulSandbox::with_clock(clock.clone()), clock)
}
fn fixture() -> SandboxFixture {
    SandboxFixture::with_identity_policy(
        "tenant-a",
        "evaluation",
        "payment-dispute",
        BTreeMap::from([("dispute-1".to_owned(), "open".to_owned())]),
        BTreeSet::from(["resolve_dispute".to_owned()]),
        SandboxIdentityPolicy::new(
            "case-1",
            "app_chat",
            "policy:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "questions:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            BTreeSet::from(["customer-1".to_owned()]),
            2_000,
        ),
    )
}
fn action(id: &str, expected: u64, replacement: &str) -> Action {
    Action::replace(id, expected, "resolve_dispute", "dispute-1", replacement)
}
fn start_protected(
    sandbox: &mut StatefulSandbox,
    arm_id: &str,
) -> (
    improvement_engine_core::sandbox::SandboxArmRef,
    improvement_engine_core::sandbox::SandboxIdentityIssuer,
) {
    sandbox
        .start_protected_arm("evaluation-1", arm_id, fixture())
        .unwrap()
}

#[test]
fn untrusted_arm_handle_cannot_bypass_the_identity_authority_boundary() {
    let (mut sandbox, _) = sandbox_at(NOW);
    let arm = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();

    assert_eq!(
        sandbox
            .execute(
                &arm,
                ActionRequest::new("tenant-a", "evaluation", action("a1", 0, "resolved")),
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceMissing
    );
}

#[test]
fn registered_fixture_issuer_mints_the_only_valid_proof_for_action_readback_and_reset() {
    let (mut sandbox, _) = sandbox_at(NOW);
    let (arm, issuer) = start_protected(&mut sandbox, "candidate");
    assert_eq!(
        sandbox
            .execute(
                &arm,
                ActionRequest::new("tenant-a", "evaluation", action("a1", 0, "resolved"))
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceMissing
    );
    assert!(matches!(
        sandbox.issue_identity(&issuer, "customer-2"),
        Err(SandboxError::IdentityEvidenceMismatch)
    ));
    let proof = sandbox.issue_identity(&issuer, "customer-1").unwrap();
    let receipt = sandbox
        .execute(
            &arm,
            ActionRequest::with_identity(
                "tenant-a",
                "evaluation",
                proof.clone(),
                action("a1", 0, "resolved"),
            ),
        )
        .unwrap();
    let readback = sandbox
        .read(
            &arm,
            ReadRequest::with_identity("tenant-a", "evaluation", proof.clone(), "dispute-1"),
        )
        .unwrap();
    assert_eq!(readback.value, "resolved");
    let output = format!("{receipt:?}{readback:?}");
    assert!(
        !output.contains("customer-1")
            && !output.contains("questions:sha256")
            && !output.contains("policy:sha256")
    );
    assert_eq!(
        sandbox
            .reset_arm(&arm, SandboxScope::new("tenant-a", "evaluation"))
            .unwrap_err(),
        SandboxError::IdentityEvidenceMissing
    );
    assert_eq!(
        sandbox
            .reset_arm(
                &arm,
                SandboxScope::with_identity("tenant-a", "evaluation", proof)
            )
            .unwrap()
            .state_revision,
        2
    );
}

#[test]
fn proof_from_another_arm_is_forged_and_denied_before_action_read_or_reset() {
    let (mut sandbox, _) = sandbox_at(NOW);
    let (_candidate, candidate_issuer) = start_protected(&mut sandbox, "candidate");
    let (baseline, baseline_issuer) = start_protected(&mut sandbox, "baseline");
    let candidate_proof = sandbox
        .issue_identity(&candidate_issuer, "customer-1")
        .unwrap();
    assert_eq!(
        sandbox
            .execute(
                &baseline,
                ActionRequest::with_identity(
                    "tenant-a",
                    "evaluation",
                    candidate_proof.clone(),
                    action("a1", 0, "resolved")
                )
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceMismatch
    );
    assert_eq!(
        sandbox
            .read(
                &baseline,
                ReadRequest::with_identity(
                    "tenant-a",
                    "evaluation",
                    candidate_proof.clone(),
                    "dispute-1"
                )
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceMismatch
    );
    assert_eq!(
        sandbox
            .reset_arm(
                &baseline,
                SandboxScope::with_identity("tenant-a", "evaluation", candidate_proof)
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceMismatch
    );
    let baseline_proof = sandbox
        .issue_identity(&baseline_issuer, "customer-1")
        .unwrap();
    assert_eq!(
        sandbox
            .execute(
                &baseline,
                ActionRequest::with_identity(
                    "tenant-a",
                    "evaluation",
                    baseline_proof,
                    action("a1", 0, "resolved")
                )
            )
            .unwrap()
            .state_revision,
        1
    );
}

#[test]
fn sealed_clock_controls_expiry_at_every_sensitive_boundary() {
    let (mut sandbox, clock) = sandbox_at(NOW - 1);
    let (arm, issuer) = start_protected(&mut sandbox, "candidate");
    let proof = sandbox.issue_identity(&issuer, "customer-1").unwrap();
    sandbox
        .execute(
            &arm,
            ActionRequest::with_identity(
                "tenant-a",
                "evaluation",
                proof.clone(),
                action("a1", 0, "resolved"),
            ),
        )
        .unwrap();
    clock.0.store(2_000, Ordering::Relaxed);
    assert_eq!(
        sandbox
            .execute(
                &arm,
                ActionRequest::with_identity(
                    "tenant-a",
                    "evaluation",
                    proof.clone(),
                    action("a2", 1, "open")
                )
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceExpired
    );
    assert_eq!(
        sandbox
            .read(
                &arm,
                ReadRequest::with_identity("tenant-a", "evaluation", proof.clone(), "dispute-1")
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceExpired
    );
    assert_eq!(
        sandbox
            .reset_arm(
                &arm,
                SandboxScope::with_identity("tenant-a", "evaluation", proof)
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceExpired
    );
}

#[test]
fn revocation_is_arm_local_and_invalid_revocation_does_not_mutate_the_arm() {
    let (mut sandbox, _) = sandbox_at(NOW);
    let (candidate, candidate_issuer) = start_protected(&mut sandbox, "candidate");
    let (baseline, baseline_issuer) = start_protected(&mut sandbox, "baseline");
    let candidate_proof = sandbox
        .issue_identity(&candidate_issuer, "customer-1")
        .unwrap();
    let baseline_proof = sandbox
        .issue_identity(&baseline_issuer, "customer-1")
        .unwrap();
    assert_eq!(
        sandbox
            .revoke_identity(
                &candidate,
                SandboxScope::new("tenant-a", "evaluation"),
                baseline_proof.clone()
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceMismatch
    );
    assert_eq!(
        sandbox
            .execute(
                &candidate,
                ActionRequest::with_identity(
                    "tenant-a",
                    "evaluation",
                    candidate_proof.clone(),
                    action("a1", 0, "resolved")
                )
            )
            .unwrap()
            .state_revision,
        1
    );
    sandbox
        .revoke_identity(
            &candidate,
            SandboxScope::new("tenant-a", "evaluation"),
            candidate_proof.clone(),
        )
        .unwrap();
    assert_eq!(
        sandbox
            .execute(
                &candidate,
                ActionRequest::with_identity(
                    "tenant-a",
                    "evaluation",
                    candidate_proof,
                    action("a2", 1, "open")
                )
            )
            .unwrap_err(),
        SandboxError::IdentityRevoked
    );
    assert_eq!(
        sandbox
            .execute(
                &baseline,
                ActionRequest::with_identity(
                    "tenant-a",
                    "evaluation",
                    baseline_proof,
                    action("a1", 0, "resolved")
                )
            )
            .unwrap()
            .state_revision,
        1
    );
}

#[test]
fn local_clock_is_explicit_not_a_default_runtime_choice() {
    let mut sandbox = StatefulSandbox::with_clock(FixedSandboxClock::new(NOW));
    let (arm, issuer) = start_protected(&mut sandbox, "candidate");
    let proof = sandbox.issue_identity(&issuer, "customer-1").unwrap();
    assert_eq!(
        sandbox
            .execute(
                &arm,
                ActionRequest::with_identity(
                    "tenant-a",
                    "evaluation",
                    proof,
                    action("a1", 0, "resolved")
                )
            )
            .unwrap()
            .state_revision,
        1
    );
}
