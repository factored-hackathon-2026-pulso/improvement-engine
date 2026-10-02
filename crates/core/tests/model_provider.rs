use improvement_engine_core::core_task::CoreTaskScope;
use improvement_engine_core::model_provider::{
    BudgetAdmission, BudgetReleaseReason, BudgetSettlement, GovernedModelAdapter,
    HmacProjectionBroker, ModelAttemptRepository, ModelAttemptState, ModelBudgetLimits,
    ModelBudgetPort, ModelCapability, ModelInvocation, ModelOutcome, ModelPolicy, ModelPort,
    ModelProvider, ModelProviderError, ModelProviderSimulator, ModelUsage, OpenAiCompatibleRequest,
    OpenAiCompatibleTransport, ProjectionBrokerPort, RedactionPolicy, TransportResult,
};

fn scope() -> CoreTaskScope {
    CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap()
}
fn policy() -> ModelPolicy {
    ModelPolicy::with_budget(
        "policy_a",
        ModelCapability::new(
            ModelProvider::OpenRouter,
            "https://openrouter.ai/api/v1",
            "openai/gpt-4.1-mini",
            "secret://pulso/openrouter",
            "model-capability-v1",
        )
        .unwrap(),
        "investigate",
        RedactionPolicy::TokenizeKnownMarkers,
        2,
        1_500,
        ModelBudgetLimits::new(100, 100, 5_000).unwrap(),
    )
    .unwrap()
}
fn invocation(
    scope: CoreTaskScope,
    policy: ModelPolicy,
    attempt: &str,
    input: &str,
) -> ModelInvocation {
    let mut broker =
        HmacProjectionBroker::new_for_test(b"test-only-projection-authority-key-32b").unwrap();
    let projection = broker
        .authorize_projection(&scope, &policy, input.into())
        .unwrap();
    ModelInvocation::from_verified(scope, policy, attempt, projection).unwrap()
}

#[test]
fn approved_invocation_returns_a_redacted_receipt_bound_to_policy_scope_and_attempt() {
    let request = invocation(
        scope(),
        policy(),
        "attempt_a",
        "customer [PII:customer_id] payment error",
    );
    let mut provider = ModelProviderSimulator::new(policy());
    provider.script_success("classification:payments", "provider-request-a");
    let receipt = provider.invoke(request).unwrap();
    assert_eq!(receipt.outcome(), &ModelOutcome::Succeeded);
    assert_eq!(receipt.scope().tenant_id(), "tenant_a");
    assert_eq!(receipt.attempt_id(), "attempt_a");
    assert!(!format!("{receipt:?}").contains("customer"));
}

#[test]
fn only_authoritative_typed_projection_can_reach_the_model_boundary() {
    let policy = policy();
    let mut broker =
        HmacProjectionBroker::new_for_test(b"test-only-projection-authority-key-32b").unwrap();
    let projection = broker
        .authorize_projection(&scope(), &policy, "safe input".into())
        .unwrap();
    let other = CoreTaskScope::new("tenant_a", "job_a", "grant_b", "authority_a").unwrap();
    assert!(ModelInvocation::from_verified(other, policy, "attempt_a", projection).is_err());
    // No public string constructor exists: a commitment-shaped string cannot
    // be paired with arbitrary treated input by an engine caller.
}

#[test]
fn post_dispatch_unknown_is_persisted_as_no_blind_redispatch() {
    let request = invocation(scope(), policy(), "attempt_a", "safe input");
    let mut provider = ModelProviderSimulator::new(policy());
    provider.script_timeout_after_dispatch();
    let first = provider.invoke(request.clone()).unwrap();
    let retry = provider.invoke(request).unwrap();
    assert_eq!(first.outcome(), &ModelOutcome::Unknown);
    assert_eq!(retry, first);
    assert_eq!(provider.dispatch_count(), 1);
}

#[test]
fn pre_dispatch_retry_is_bounded_and_reservation_is_released() {
    let request = invocation(scope(), policy(), "attempt_a", "safe input");
    let mut provider = ModelProviderSimulator::new(policy());
    for _ in 0..4 {
        provider.script_pre_dispatch_transient("connect_refused");
    }
    let first = provider.invoke(request.clone()).unwrap();
    let second = provider.invoke(request.clone()).unwrap();
    let exhausted = provider.invoke(request).unwrap();
    assert_eq!(first.pre_dispatch_retries_used(), 1);
    assert_eq!(second.pre_dispatch_retries_used(), 2);
    assert!(exhausted.pre_dispatch_retry_exhausted());
    assert_eq!(exhausted.budget_settlement(), BudgetSettlement::Released);
    assert_eq!(provider.transport_attempt_count(), 2);
}

#[derive(Default)]
struct CapturingTransport {
    requests: Vec<OpenAiCompatibleRequest>,
}
impl OpenAiCompatibleTransport for CapturingTransport {
    fn complete(&mut self, request: OpenAiCompatibleRequest) -> TransportResult {
        self.requests.push(request);
        TransportResult::Succeeded {
            provider_request_id: "provider-request-a".into(),
            output: "classified".into(),
            usage: ModelUsage::new(1, 1, 1).unwrap(),
        }
    }
}

#[test]
fn transport_receives_only_treated_input_credential_reference_and_ceilings() {
    let mut adapter = GovernedModelAdapter::new(policy(), CapturingTransport::default());
    adapter
        .invoke(invocation(
            scope(),
            policy(),
            "attempt_a",
            "customer [PII:id] error",
        ))
        .unwrap();
    let request = adapter.into_transport().requests.remove(0);
    assert_eq!(request.input(), "customer [REDACTED] error");
    assert_eq!(request.max_output_units(), 100);
    assert_eq!(request.max_response_bytes(), 400);
    assert_eq!(request.max_cost_micros(), 5_000);
    assert_eq!(request.credential_ref(), "secret://pulso/openrouter");
}

#[test]
fn capability_debug_and_digests_never_leak_secret_or_treated_input() {
    let capability = ModelCapability::new_with_credential_alias(
        ModelProvider::OpenRouter,
        "https://openrouter.ai/api/v1",
        "openai/gpt-4.1-mini",
        "secret://pulso/prod-openrouter-key",
        "openrouter_primary",
        "model-capability-v1",
    )
    .unwrap();
    let policy = ModelPolicy::with_budget(
        "policy_a",
        capability.clone(),
        "investigate",
        RedactionPolicy::TokenizeKnownMarkers,
        2,
        1_500,
        ModelBudgetLimits::new(100, 100, 5_000).unwrap(),
    )
    .unwrap();
    let request = invocation(scope(), policy.clone(), "attempt_a", "raw [PII:tax_id]");
    let mut provider = ModelProviderSimulator::new(policy);
    provider.script_success_with_usage(
        "secret output",
        "provider-request-secret",
        ModelUsage::new(1, 1, 10).unwrap(),
    );
    let receipt = provider.invoke(request.clone()).unwrap();
    let combined = format!(
        "{capability:?} {request:?} {receipt:?} {}",
        capability.digest()
    );
    for forbidden in [
        "prod-openrouter-key",
        "raw",
        "secret output",
        "provider-request-secret",
    ] {
        assert!(!combined.contains(forbidden));
    }
}

struct DenyingBudget;
impl ModelBudgetPort for DenyingBudget {
    fn admit(
        &mut self,
        _: improvement_engine_core::model_provider::ModelBudgetRequest,
    ) -> BudgetAdmission {
        BudgetAdmission::Denied {
            reason_code: "grant_exhausted".into(),
        }
    }
}

#[test]
fn denied_budget_prevents_dispatch_and_post_dispatch_excess_is_not_accepted() {
    let request = invocation(scope(), policy(), "attempt_a", "safe input");
    let mut denied =
        GovernedModelAdapter::with_budget(policy(), CapturingTransport::default(), DenyingBudget);
    assert_eq!(
        denied.invoke(request).unwrap().outcome(),
        &ModelOutcome::BudgetDenied
    );
    assert!(denied.into_transport().requests.is_empty());
    let mut provider = ModelProviderSimulator::new(policy());
    provider.script_success_with_usage(
        "ok",
        "provider-request-a",
        ModelUsage::new(1, 101, 10).unwrap(),
    );
    let receipt = provider
        .invoke(invocation(scope(), policy(), "attempt_b", "safe input"))
        .unwrap();
    assert_eq!(
        receipt.outcome(),
        &ModelOutcome::BudgetExceededAfterDispatch
    );
}

#[test]
fn idempotency_key_binds_grant_and_authority_context() {
    let p = policy();
    let a = invocation(scope(), p.clone(), "attempt_a", "safe input");
    let b = invocation(
        CoreTaskScope::new("tenant_a", "job_a", "grant_b", "authority_a").unwrap(),
        p.clone(),
        "attempt_a",
        "safe input",
    );
    let c = invocation(
        CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_b").unwrap(),
        p,
        "attempt_a",
        "safe input",
    );
    assert_ne!(a.provider_idempotency_key(), b.provider_idempotency_key());
    assert_ne!(a.provider_idempotency_key(), c.provider_idempotency_key());
}

#[test]
fn received_output_that_exceeds_the_transport_byte_ceiling_is_rejected() {
    let mut provider = ModelProviderSimulator::new(policy());
    provider.script_success("x".repeat(401), "provider-request-a");
    let receipt = provider
        .invoke(invocation(scope(), policy(), "attempt_a", "safe input"))
        .unwrap();
    assert_eq!(
        receipt.outcome(),
        &ModelOutcome::OutputLimitExceededAfterDispatch
    );
    assert_eq!(receipt.output_digest(), None);
}

struct FailingClaimRepository;
impl ModelAttemptRepository for FailingClaimRepository {
    fn load(
        &mut self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<ModelAttemptState>, ModelProviderError> {
        Ok(None)
    }
    fn compare_and_store(
        &mut self,
        _: Option<u64>,
        _: ModelAttemptState,
    ) -> Result<ModelAttemptState, ModelProviderError> {
        Err(ModelProviderError::AttemptStateConflict)
    }
}
struct ReleasingBudget(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl ModelBudgetPort for ReleasingBudget {
    fn admit(
        &mut self,
        _: improvement_engine_core::model_provider::ModelBudgetRequest,
    ) -> BudgetAdmission {
        BudgetAdmission::Admitted {
            reservation_id: "reservation_a".into(),
        }
    }
    fn release(&mut self, _: &str, reason: BudgetReleaseReason) -> BudgetSettlement {
        assert_eq!(
            reason,
            BudgetReleaseReason::DurableClaimFailedBeforeDispatch
        );
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        BudgetSettlement::Released
    }
}

#[test]
fn reservation_is_released_when_durable_pre_dispatch_claim_fails() {
    let released = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut adapter = GovernedModelAdapter::with_dependencies(
        policy(),
        CapturingTransport::default(),
        ReleasingBudget(released.clone()),
        FailingClaimRepository,
    );
    assert!(
        adapter
            .invoke(invocation(scope(), policy(), "attempt_a", "safe input"))
            .is_err()
    );
    assert!(released.load(std::sync::atomic::Ordering::SeqCst));
    assert!(adapter.into_transport().requests.is_empty());
}
