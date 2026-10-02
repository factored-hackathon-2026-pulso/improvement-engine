use std::collections::{BTreeMap, BTreeSet};

use improvement_engine_core::sandbox::{
    Action, ActionRequest, IdentityEvidence, ReadRequest, SandboxError, SandboxFixture,
    SandboxIdentityPolicy, SandboxPort, SandboxScope, StatefulSandbox,
};

const NOW: u64 = 1_000;

fn policy() -> SandboxIdentityPolicy {
    SandboxIdentityPolicy::new(
        "case-1",
        "app_chat",
        "policy:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "questions:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        BTreeSet::from(["customer-1".to_owned()]),
        2_000,
    )
}

fn evidence() -> IdentityEvidence {
    IdentityEvidence::new(
        "customer-1",
        "tenant-a",
        "case-1",
        "app_chat",
        "policy:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "questions:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2_000,
    )
}

fn evidence_with(
    tenant_id: &str,
    case_id: &str,
    channel: &str,
    policy_digest: &str,
    questions_digest: &str,
    valid_until: u64,
) -> IdentityEvidence {
    IdentityEvidence::new(
        "customer-1",
        tenant_id,
        case_id,
        channel,
        policy_digest,
        questions_digest,
        valid_until,
    )
}

fn fixture() -> SandboxFixture {
    SandboxFixture::with_identity_policy(
        "tenant-a",
        "evaluation",
        "payment-dispute",
        BTreeMap::from([("dispute-1".to_owned(), "open".to_owned())]),
        BTreeSet::from(["resolve_dispute".to_owned()]),
        policy(),
    )
}

fn action() -> Action {
    Action::replace("action-1", 0, "resolve_dispute", "dispute-1", "resolved")
}

#[test]
fn sensitive_actions_and_readback_require_a_current_matching_identity_policy_and_question_proof() {
    let mut sandbox = StatefulSandbox::default();
    let arm = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();

    let missing = ActionRequest::new("tenant-a", "evaluation", action());
    assert_eq!(
        sandbox.execute(&arm, missing).unwrap_err(),
        SandboxError::IdentityEvidenceMissing
    );

    for invalid in [
        IdentityEvidence::new(
            "customer-2",
            "tenant-a",
            "case-1",
            "app_chat",
            "policy:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "questions:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            2_000,
        ),
        evidence_with(
            "tenant-b",
            "case-1",
            "app_chat",
            "policy:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "questions:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            2_000,
        ),
        evidence_with(
            "tenant-a",
            "other-case",
            "app_chat",
            "policy:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "questions:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            2_000,
        ),
        evidence_with(
            "tenant-a",
            "case-1",
            "phone",
            "policy:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "questions:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            2_000,
        ),
        evidence_with(
            "tenant-a",
            "case-1",
            "app_chat",
            "policy:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "questions:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            2_000,
        ),
        evidence_with(
            "tenant-a",
            "case-1",
            "app_chat",
            "policy:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "questions:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            2_000,
        ),
    ] {
        assert_eq!(
            sandbox
                .execute(
                    &arm,
                    ActionRequest::with_identity("tenant-a", "evaluation", NOW, invalid, action())
                )
                .unwrap_err(),
            SandboxError::IdentityEvidenceMismatch
        );
    }

    let expired = IdentityEvidence::new(
        "customer-1",
        "tenant-a",
        "case-1",
        "app_chat",
        "policy:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "questions:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        NOW,
    );
    assert_eq!(
        sandbox
            .execute(
                &arm,
                ActionRequest::with_identity("tenant-a", "evaluation", NOW, expired, action())
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceExpired
    );

    let receipt = sandbox
        .execute(
            &arm,
            ActionRequest::with_identity("tenant-a", "evaluation", NOW, evidence(), action()),
        )
        .unwrap();
    assert_eq!(receipt.state_revision, 1);
    let readback = sandbox
        .read(
            &arm,
            ReadRequest::with_identity("tenant-a", "evaluation", NOW, evidence(), "dispute-1"),
        )
        .unwrap();
    assert_eq!(readback.value, "resolved");
    // Identity proof is consumed at the boundary, never copied to result
    // evidence that could later be logged or sent to a model.
    let result_debug = format!("{receipt:?}{readback:?}");
    assert!(!result_debug.contains("customer-1"));
    assert!(!result_debug.contains("questions:sha256"));
    assert!(!result_debug.contains("policy:sha256"));

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
                SandboxScope::with_identity("tenant-a", "evaluation", NOW, evidence()),
            )
            .unwrap()
            .state_revision,
        2
    );
}

#[test]
fn a_fixture_policy_expiring_in_flight_denies_the_effect_before_state_mutation() {
    let mut sandbox = StatefulSandbox::default();
    let expiring_fixture = SandboxFixture::with_identity_policy(
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
            NOW,
        ),
    );
    let arm = sandbox
        .start_arm("evaluation-1", "candidate", expiring_fixture)
        .unwrap();
    assert_eq!(
        sandbox
            .execute(
                &arm,
                ActionRequest::with_identity("tenant-a", "evaluation", NOW, evidence(), action()),
            )
            .unwrap_err(),
        SandboxError::IdentityEvidenceExpired
    );
}

#[test]
fn revocation_in_flight_blocks_only_the_revoked_arm_without_leaking_answers_or_cross_arm_state() {
    let mut sandbox = StatefulSandbox::default();
    let baseline = sandbox
        .start_arm("evaluation-1", "baseline", fixture())
        .unwrap();
    let candidate = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();

    sandbox
        .revoke_identity(
            &candidate,
            SandboxScope::new("tenant-a", "evaluation"),
            evidence(),
        )
        .unwrap();
    assert_eq!(
        sandbox
            .execute(
                &candidate,
                ActionRequest::with_identity("tenant-a", "evaluation", NOW, evidence(), action())
            )
            .unwrap_err(),
        SandboxError::IdentityRevoked
    );
    assert_eq!(
        sandbox
            .execute(
                &baseline,
                ActionRequest::with_identity("tenant-a", "evaluation", NOW, evidence(), action())
            )
            .unwrap()
            .state_revision,
        1
    );

    let denied = sandbox
        .read(
            &candidate,
            ReadRequest::with_identity("tenant-a", "evaluation", NOW, evidence(), "dispute-1"),
        )
        .unwrap_err();
    assert_eq!(denied, SandboxError::IdentityRevoked);
}
