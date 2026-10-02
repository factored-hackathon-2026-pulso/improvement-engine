//! RED contract for the U06-backed durable model-attempt ledger.

use improvement_engine_core::core_task::CoreTaskScope;
use improvement_engine_core::model_provider::{
    AttemptLifecycle, GovernedModelAdapter, ModelAttemptRepository, ModelOutcome, ModelPort,
    OpenAiCompatibleRequest, OpenAiCompatibleTransport, PostgresModelAttemptRepository,
    SharedModelAttemptRepository,
};
use improvement_engine_core::model_provider::{
    HmacProjectionBroker, ModelBudgetLimits, ModelCapability, ModelInvocation, ModelPolicy,
    ModelProvider, ProjectionBrokerPort, RedactionPolicy,
};

#[test]
fn postgres_attempt_ledger_is_the_restart_boundary_not_an_adapter_btreemap() {
    let _repository: Option<PostgresModelAttemptRepository> = None;
    let _required = (
        AttemptLifecycle::ReconcileBeforeRetry,
        ModelOutcome::Unknown,
    );
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
        100,
        ModelBudgetLimits::new(100, 100, 5_000).unwrap(),
    )
    .unwrap()
}
fn invocation() -> ModelInvocation {
    let scope = CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap();
    let p = policy();
    let mut broker =
        HmacProjectionBroker::new_for_test(b"test-only-projection-authority-key-32b").unwrap();
    let projection = broker
        .authorize_projection(&scope, &p, "safe input".into())
        .unwrap();
    ModelInvocation::from_verified(scope, p, "attempt_a", projection).unwrap()
}

struct TimeoutTransport;
impl OpenAiCompatibleTransport for TimeoutTransport {
    fn complete(
        &mut self,
        _: OpenAiCompatibleRequest,
    ) -> improvement_engine_core::model_provider::TransportResult {
        improvement_engine_core::model_provider::TransportResult::UnknownAfterDispatch {
            class: "timeout".into(),
        }
    }
}
#[derive(Default)]
struct CountingTransport {
    calls: u8,
}
impl OpenAiCompatibleTransport for CountingTransport {
    fn complete(
        &mut self,
        _: OpenAiCompatibleRequest,
    ) -> improvement_engine_core::model_provider::TransportResult {
        self.calls += 1;
        improvement_engine_core::model_provider::TransportResult::DependencyUnavailable {
            class: "should_not_call".into(),
        }
    }
}

#[test]
fn restart_reads_unknown_from_the_shared_u06_contract_and_never_redispatches() {
    let ledger = SharedModelAttemptRepository::default();
    let mut first = GovernedModelAdapter::with_dependencies(
        policy(),
        TimeoutTransport,
        improvement_engine_core::model_provider::PermissiveModelBudget,
        ledger.clone(),
    );
    assert_eq!(
        first.invoke(invocation()).unwrap().outcome(),
        &ModelOutcome::Unknown
    );
    let mut restarted = GovernedModelAdapter::with_dependencies(
        policy(),
        CountingTransport::default(),
        improvement_engine_core::model_provider::PermissiveModelBudget,
        ledger,
    );
    assert_eq!(
        restarted.invoke(invocation()).unwrap().outcome(),
        &ModelOutcome::Unknown
    );
    assert_eq!(restarted.into_transport().calls, 0);
}

#[test]
#[ignore = "requires an isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn migration_persists_model_attempt_cas_and_unknown_across_postgres_restart() {
    assert_eq!(
        std::env::var("PULSO_ALLOW_DESTRUCTIVE_TEST_DB").as_deref(),
        Ok("1")
    );
    let url = std::env::var("PULSO_TEST_POSTGRES_URL").unwrap();
    let mut setup = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    setup
        .batch_execute(include_str!(
            "../../../migrations/0001_pulso_artifact_revisions.sql"
        ))
        .unwrap();
    setup
        .batch_execute(include_str!(
            "../../../migrations/0002_model_attempt_ledger.sql"
        ))
        .unwrap();
    setup
        .batch_execute("TRUNCATE pulso_model_attempts")
        .unwrap();
    drop(setup);

    let first_repository = PostgresModelAttemptRepository::new(
        postgres::Client::connect(&url, postgres::NoTls).unwrap(),
    );
    let mut first = GovernedModelAdapter::with_dependencies(
        policy(),
        TimeoutTransport,
        improvement_engine_core::model_provider::PermissiveModelBudget,
        first_repository,
    );
    assert_eq!(
        first.invoke(invocation()).unwrap().outcome(),
        &ModelOutcome::Unknown
    );

    let mut persisted = PostgresModelAttemptRepository::new(
        postgres::Client::connect(&url, postgres::NoTls).unwrap(),
    );
    let state = persisted
        .load("tenant_a", "job_a", "attempt_a")
        .unwrap()
        .unwrap();
    assert_eq!(state.receipt().outcome(), &ModelOutcome::Unknown);
    assert!(persisted.compare_and_store(Some(0), state.clone()).is_err());
    assert!(
        persisted
            .load("tenant_b", "job_a", "attempt_a")
            .unwrap()
            .is_none()
    );

    let second_repository = PostgresModelAttemptRepository::new(
        postgres::Client::connect(&url, postgres::NoTls).unwrap(),
    );
    let mut restarted = GovernedModelAdapter::with_dependencies(
        policy(),
        CountingTransport::default(),
        improvement_engine_core::model_provider::PermissiveModelBudget,
        second_repository,
    );
    assert_eq!(
        restarted.invoke(invocation()).unwrap().outcome(),
        &ModelOutcome::Unknown
    );
    assert_eq!(restarted.into_transport().calls, 0);
}
