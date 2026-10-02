use std::collections::{BTreeMap, BTreeSet};

use improvement_engine_core::sandbox::{
    Action, ActionRequest, FixedSandboxClock, ReadRequest, SandboxError, SandboxFixture,
    SandboxPort, SandboxScope, StatefulSandbox,
};

fn fixture() -> SandboxFixture {
    SandboxFixture::new(
        "tenant-a",
        "evaluation",
        "payment-dispute",
        BTreeMap::from([("dispute-1".to_owned(), "open".to_owned())]),
        BTreeSet::from(["resolve_dispute".to_owned()]),
    )
}

fn authorized(action: Action) -> ActionRequest {
    ActionRequest::new("tenant-a", "evaluation", action)
}

fn scope() -> SandboxScope {
    SandboxScope::new("tenant-a", "evaluation")
}

fn sandbox() -> StatefulSandbox {
    StatefulSandbox::with_clock(FixedSandboxClock::new(0))
}

#[test]
fn allowed_action_changes_only_its_arm_and_authorized_readback_proves_the_effect() {
    let mut sandbox = sandbox();
    let arm = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();

    let receipt = sandbox
        .execute(
            &arm,
            authorized(Action::replace(
                "action-1",
                0,
                "resolve_dispute",
                "dispute-1",
                "resolved",
            )),
        )
        .unwrap();
    assert_eq!(receipt.state_revision, 1);

    let readback = sandbox
        .read(
            &arm,
            ReadRequest::new("tenant-a", "evaluation", "dispute-1"),
        )
        .unwrap();
    assert_eq!(readback.value, "resolved");
    assert_eq!(readback.fixture_id, "payment-dispute");
    assert_eq!(readback.evaluation_id, "evaluation-1");
    assert_eq!(readback.arm_id, "candidate");
    assert_eq!(readback.state_revision, receipt.state_revision);
    assert_eq!(receipt.fixture_id, "payment-dispute");
    assert_eq!(receipt.evaluation_id, "evaluation-1");
    assert_eq!(receipt.arm_id, "candidate");
}

#[test]
fn evaluation_arms_start_from_the_same_fixture_and_reset_does_not_leak_state() {
    let mut sandbox = sandbox();
    let baseline = sandbox
        .start_arm("evaluation-1", "baseline", fixture())
        .unwrap();
    let candidate = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();

    sandbox
        .execute(
            &candidate,
            authorized(Action::replace(
                "action-1",
                0,
                "resolve_dispute",
                "dispute-1",
                "resolved",
            )),
        )
        .unwrap();

    assert_eq!(
        sandbox
            .read(
                &baseline,
                ReadRequest::new("tenant-a", "evaluation", "dispute-1")
            )
            .unwrap()
            .value,
        "open"
    );

    let reset = sandbox.reset_arm(&candidate, scope()).unwrap();
    assert_eq!(reset.evaluation_id, "evaluation-1");
    assert_eq!(reset.arm_id, "candidate");
    assert_eq!(reset.fixture_id, "payment-dispute");
    assert_eq!(reset.before_revision, 1);
    assert_eq!(reset.state_revision, 2);
    assert_eq!(
        reset.state_digest,
        "sha256:372e5989e524fbf648c71cf0321211d7925b97a2d39b3f13f19953154fe4d828"
    );
    assert_eq!(
        sandbox
            .read(
                &candidate,
                ReadRequest::new("tenant-a", "evaluation", "dispute-1")
            )
            .unwrap()
            .value,
        "open"
    );

    let retried_after_reset = sandbox
        .execute(
            &candidate,
            authorized(Action::replace(
                "action-1",
                2,
                "resolve_dispute",
                "dispute-1",
                "resolved",
            )),
        )
        .unwrap();
    assert_eq!(retried_after_reset.state_revision, 3);

    let repeated_reset = sandbox.reset_arm(&candidate, scope()).unwrap();
    assert_eq!(repeated_reset.before_revision, 3);
    assert_eq!(repeated_reset.state_revision, 4);
    assert_eq!(repeated_reset.state_digest, reset.state_digest);
}

#[test]
fn evaluation_rejects_a_different_fixture_for_another_arm() {
    let mut sandbox = sandbox();
    sandbox
        .start_arm("evaluation-1", "baseline", fixture())
        .unwrap();
    let different_fixture = SandboxFixture::new(
        "tenant-a",
        "evaluation",
        "different-fixture",
        BTreeMap::from([("dispute-1".to_owned(), "different".to_owned())]),
        BTreeSet::from(["resolve_dispute".to_owned()]),
    );

    assert_eq!(
        sandbox
            .start_arm("evaluation-1", "candidate", different_fixture)
            .unwrap_err(),
        SandboxError::FixtureMismatch
    );
}

#[test]
fn reset_denies_cross_scope_without_changing_the_arm() {
    let mut sandbox = sandbox();
    let arm = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();
    sandbox
        .execute(
            &arm,
            authorized(Action::replace(
                "action-1",
                0,
                "resolve_dispute",
                "dispute-1",
                "resolved",
            )),
        )
        .unwrap();

    assert_eq!(
        sandbox
            .reset_arm(&arm, SandboxScope::new("tenant-b", "evaluation"))
            .unwrap_err(),
        SandboxError::TenantDenied
    );
    assert_eq!(
        sandbox
            .reset_arm(&arm, SandboxScope::new("tenant-a", "wrong-namespace"))
            .unwrap_err(),
        SandboxError::NamespaceDenied
    );
    assert_eq!(
        sandbox
            .read(
                &arm,
                ReadRequest::new("tenant-a", "evaluation", "dispute-1"),
            )
            .unwrap()
            .value,
        "resolved"
    );
}

#[test]
fn readback_denies_cross_tenant_and_namespace_requests() {
    let mut sandbox = sandbox();
    let arm = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();

    assert_eq!(
        sandbox
            .read(
                &arm,
                ReadRequest::new("tenant-b", "evaluation", "dispute-1")
            )
            .unwrap_err(),
        SandboxError::TenantDenied
    );
    assert_eq!(
        sandbox
            .read(
                &arm,
                ReadRequest::new("tenant-a", "another-namespace", "dispute-1")
            )
            .unwrap_err(),
        SandboxError::NamespaceDenied
    );
}

#[test]
fn stale_forbidden_and_unknown_actions_are_explicitly_rejected_without_effect() {
    let mut sandbox = sandbox();
    let arm = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();

    assert_eq!(
        sandbox
            .execute(
                &arm,
                authorized(Action::replace(
                    "forbidden",
                    0,
                    "issue_refund",
                    "dispute-1",
                    "refunded",
                )),
            )
            .unwrap_err(),
        SandboxError::ActionForbidden {
            action: "issue_refund".to_owned()
        }
    );
    assert_eq!(
        sandbox
            .execute(
                &arm,
                authorized(Action::replace(
                    "unknown",
                    0,
                    "resolve_dispute",
                    "missing",
                    "resolved",
                )),
            )
            .unwrap_err(),
        SandboxError::ResourceUnknown {
            resource: "missing".to_owned()
        }
    );
    let unchanged = sandbox
        .read(
            &arm,
            ReadRequest::new("tenant-a", "evaluation", "dispute-1"),
        )
        .unwrap();
    assert_eq!(unchanged.value, "open");
    assert_eq!(unchanged.state_revision, 0);

    sandbox
        .execute(
            &arm,
            authorized(Action::replace(
                "action-1",
                0,
                "resolve_dispute",
                "dispute-1",
                "resolved",
            )),
        )
        .unwrap();
    assert_eq!(
        sandbox
            .execute(
                &arm,
                authorized(Action::replace(
                    "stale",
                    0,
                    "resolve_dispute",
                    "dispute-1",
                    "resolved",
                )),
            )
            .unwrap_err(),
        SandboxError::StaleAction {
            expected_revision: 0,
            actual_revision: 1,
        }
    );
}

#[test]
fn equivalent_retry_returns_the_original_receipt_without_applying_a_second_effect() {
    let mut sandbox = sandbox();
    let arm = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();
    let action = Action::replace("action-1", 0, "resolve_dispute", "dispute-1", "resolved");

    let first = sandbox.execute(&arm, authorized(action.clone())).unwrap();
    let retry = sandbox.execute(&arm, authorized(action)).unwrap();

    assert_eq!(retry, first);
    assert_eq!(
        sandbox
            .read(
                &arm,
                ReadRequest::new("tenant-a", "evaluation", "dispute-1")
            )
            .unwrap()
            .state_revision,
        1
    );
}

#[test]
fn execution_denies_cross_tenant_and_namespace_before_attempting_an_effect() {
    let mut sandbox = sandbox();
    let arm = sandbox
        .start_arm("evaluation-1", "candidate", fixture())
        .unwrap();
    let action = Action::replace("action-1", 0, "resolve_dispute", "dispute-1", "resolved");

    assert_eq!(
        sandbox
            .execute(
                &arm,
                ActionRequest::new("tenant-b", "evaluation", action.clone()),
            )
            .unwrap_err(),
        SandboxError::TenantDenied
    );
    assert_eq!(
        sandbox
            .execute(
                &arm,
                ActionRequest::new("tenant-a", "wrong-namespace", action),
            )
            .unwrap_err(),
        SandboxError::NamespaceDenied
    );
}
