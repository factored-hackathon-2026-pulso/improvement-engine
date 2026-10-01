use improvement_engine_core::core_task::{
    CoreTaskBinding, CoreTaskBindingRegistry, CoreTaskError, CoreTaskInvocation, CoreTaskOutcome,
    CoreTaskPort, CoreTaskScope, CoreTaskSimulator, SimulatorDisposition,
};

const SHA: &str = "53e729d624c8284e906249df84c1a1df84cc8d40";
const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn binding() -> CoreTaskBinding {
    CoreTaskBinding::new("scout", "rel-scout-1", "0.5.0", SHA).unwrap()
}

fn simulator() -> CoreTaskSimulator {
    CoreTaskSimulator::new(CoreTaskBindingRegistry::new(vec![binding()]).unwrap())
}

fn scope() -> CoreTaskScope {
    CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap()
}

fn invocation(attempt: &str) -> CoreTaskInvocation {
    CoreTaskInvocation::new(scope(), binding(), attempt, DIGEST).unwrap()
}

#[test]
fn fixed_binding_dispatches_once_and_receipt_keeps_exact_binding_digest() {
    let mut simulator = simulator();
    simulator.script_success("core-run-a", DIGEST).unwrap();

    let receipt = simulator.invoke(invocation("attempt_a")).unwrap();
    assert_eq!(receipt.binding_digest(), binding().digest());
    assert_eq!(receipt.outcome(), &CoreTaskOutcome::Succeeded);
    assert_eq!(receipt.core_run_id(), Some("core-run-a"));
    assert_eq!(simulator.dispatch_count(), 1);

    let repeat = simulator.invoke(invocation("attempt_a")).unwrap();
    assert_eq!(repeat, receipt);
    assert_eq!(simulator.dispatch_count(), 1);
}

#[test]
fn unsupported_or_unpinned_binding_is_rejected_before_dispatch() {
    assert!(matches!(
        CoreTaskBinding::new("scout", "rel-scout-1", "0.5.0", "main"),
        Err(CoreTaskError::UnpinnedContract)
    ));

    let mut simulator = simulator();
    let unsupported = CoreTaskBinding::new(
        "scout",
        "rel-scout-1",
        "9.9.9",
        "ffffffffffffffffffffffffffffffffffffffff",
    )
    .unwrap();
    let request = CoreTaskInvocation::new(scope(), unsupported, "attempt_a", DIGEST).unwrap();
    assert!(matches!(
        simulator.invoke(request),
        Err(CoreTaskError::UnsupportedBinding { .. })
    ));
    assert_eq!(simulator.dispatch_count(), 0);

    let wrong_contract = CoreTaskBinding::new(
        "scout",
        "rel-scout-2",
        "9.9.9",
        "ffffffffffffffffffffffffffffffffffffffff",
    )
    .unwrap();
    assert!(matches!(
        CoreTaskBindingRegistry::new(vec![wrong_contract]),
        Err(CoreTaskError::UnsupportedContractSnapshot)
    ));
}

#[test]
fn changed_scope_or_payload_cannot_reuse_attempt_identity() {
    let mut simulator = simulator();
    simulator.script_success("core-run-a", DIGEST).unwrap();
    simulator.invoke(invocation("attempt_a")).unwrap();

    for changed_scope in [
        CoreTaskScope::new("tenant_a", "job_a", "grant_b", "authority_a").unwrap(),
        CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_b").unwrap(),
    ] {
        let request =
            CoreTaskInvocation::new(changed_scope, binding(), "attempt_a", DIGEST).unwrap();
        assert!(matches!(
            simulator.invoke(request),
            Err(CoreTaskError::ScopeMismatch { .. })
        ));
    }

    for independent_scope in [
        CoreTaskScope::new("tenant_b", "job_a", "grant_a", "authority_a").unwrap(),
        CoreTaskScope::new("tenant_a", "job_b", "grant_a", "authority_a").unwrap(),
    ] {
        let request =
            CoreTaskInvocation::new(independent_scope, binding(), "attempt_a", DIGEST).unwrap();
        assert!(simulator.invoke(request).is_ok());
    }

    let changed_input = CoreTaskInvocation::new(
        scope(),
        binding(),
        "attempt_a",
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .unwrap();
    assert!(matches!(
        simulator.invoke(changed_input),
        Err(CoreTaskError::AttemptConflict { .. })
    ));
    assert_eq!(simulator.dispatch_count(), 3);
}

#[test]
fn timeout_or_crash_after_dispatch_is_unknown_and_never_redispatched() {
    for disposition in [
        SimulatorDisposition::TimeoutAfterDispatch,
        SimulatorDisposition::CrashAfterDispatch,
    ] {
        let mut simulator = simulator();
        simulator.script(disposition);
        let first = simulator.invoke(invocation("attempt_a")).unwrap();
        assert_eq!(first.outcome(), &CoreTaskOutcome::Unknown);
        assert_eq!(simulator.dispatch_count(), 1);

        let retry = simulator.invoke(invocation("attempt_a")).unwrap();
        assert_eq!(retry, first);
        assert_eq!(simulator.dispatch_count(), 1);
    }
}
