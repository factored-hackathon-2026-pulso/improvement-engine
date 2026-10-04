use improvement_engine_core::core_task::CoreTaskScope;
use improvement_engine_core::model_provider::{
    ModelPolicy, ModelProviderError, ProjectionBrokerPort, VerifiedProjection,
};
use improvement_engine_core::platform_source_policy::{
    PlatformColumn, PlatformEventCatalog, PlatformEventCatalogError,
    PlatformEventCatalogProvenance, PlatformEventDisposition, PlatformEventPayloadLocalOnly,
    PlatformRelation, PlatformSourcePolicyProvenance, PlatformSourceReadPlan, PlatformSourceText,
    PlatformTextEgressError, PlatformTurnBody, include_customer_in_population,
};

#[derive(Default)]
struct RecordingReader(
    Vec<(
        PlatformRelation,
        Vec<improvement_engine_core::platform_source_policy::PlatformColumn>,
    )>,
);

impl improvement_engine_core::platform_source_policy::PlatformSourceReader for RecordingReader {
    type Error = std::convert::Infallible;

    fn read_relation(
        &mut self,
        relation: PlatformRelation,
        columns: &[improvement_engine_core::platform_source_policy::PlatformColumn],
    ) -> Result<(), Self::Error> {
        self.0.push((relation, columns.to_vec()));
        Ok(())
    }
}

#[test]
fn default_read_plan_never_requests_credential_relations() {
    let mut reader = RecordingReader::default();
    PlatformSourceReadPlan::platform_live()
        .execute(&mut reader)
        .expect("all allow-listed relations can be read");

    assert_eq!(
        reader.0,
        vec![
            (
                PlatformRelation::Cases,
                vec![
                    PlatformColumn::CaseId,
                    PlatformColumn::CustomerId,
                    PlatformColumn::Channel,
                    PlatformColumn::Priority,
                    PlatformColumn::CreatedAt,
                    PlatformColumn::SlaDueAt,
                ],
            ),
            (
                PlatformRelation::Turns,
                vec![
                    PlatformColumn::CaseId,
                    PlatformColumn::TurnSequence,
                    PlatformColumn::TurnBody,
                ],
            ),
            (
                PlatformRelation::Assignments,
                vec![
                    PlatformColumn::CaseId,
                    PlatformColumn::StaffId,
                    PlatformColumn::AssignmentTime,
                ],
            ),
            (
                PlatformRelation::CustomerCaseSlots,
                vec![PlatformColumn::CaseId, PlatformColumn::CustomerId],
            ),
            (
                PlatformRelation::Customers,
                vec![PlatformColumn::CustomerId, PlatformColumn::SimulatorFlag],
            ),
            (
                PlatformRelation::EventLog,
                vec![
                    PlatformColumn::EventSequence,
                    PlatformColumn::EventType,
                    PlatformColumn::EventTime,
                    PlatformColumn::IngestedAt,
                    PlatformColumn::EventPayloadLocalOnly,
                ],
            ),
            (
                PlatformRelation::Staff,
                vec![
                    PlatformColumn::StaffId,
                    PlatformColumn::StaffRoles,
                    PlatformColumn::StaffLanguages,
                    PlatformColumn::StaffTeam,
                    PlatformColumn::StaffActive,
                ],
            ),
        ]
    );
}

#[test]
fn auth_events_are_denied_unless_the_exact_type_is_allowlisted() {
    let defaults = PlatformEventCatalog::default();
    assert_eq!(
        defaults.classify("auth.login_succeeded"),
        PlatformEventDisposition::DenyAuthNotAllowlisted
    );
    assert_eq!(
        defaults.classify("case.status_changed"),
        PlatformEventDisposition::AcceptBusiness(
            improvement_engine_core::platform_source_policy::PlatformBusinessEvent::CaseStatusChanged
        )
    );
    assert_eq!(
        defaults.classify("case.future_new_event"),
        PlatformEventDisposition::QuarantineUnknown
    );

    let allowlisted = PlatformEventCatalog::with_auth_allowlist(
        1,
        vec!["auth.login_succeeded".into(), "auth.login_succeeded".into()],
    )
    .expect("valid exact allow-list");
    assert_eq!(
        allowlisted.classify("auth.login_succeeded"),
        PlatformEventDisposition::AcceptAllowlistedAuth
    );
    assert_eq!(
        allowlisted.classify("auth.mfa_challenge_created"),
        PlatformEventDisposition::DenyAuthNotAllowlisted
    );
    assert_eq!(
        PlatformEventCatalog::with_auth_allowlist(1, vec!["auth.*".into()]),
        Err(PlatformEventCatalogError::InvalidAuthEventType)
    );
    assert_eq!(
        PlatformEventCatalog::with_auth_allowlist(0, Vec::new()),
        Err(PlatformEventCatalogError::InvalidVersion)
    );
}

#[test]
fn event_catalog_version_and_digest_are_stable_and_manifest_ready() {
    let first = PlatformEventCatalog::with_auth_allowlist(
        1,
        vec!["auth.login_succeeded".into(), "auth.login_failed".into()],
    )
    .unwrap();
    let same_content = PlatformEventCatalog::with_auth_allowlist(
        1,
        vec![
            "auth.login_failed".into(),
            "auth.login_succeeded".into(),
            "auth.login_failed".into(),
        ],
    )
    .unwrap();
    let next_version = PlatformEventCatalog::with_auth_allowlist(
        2,
        vec!["auth.login_succeeded".into(), "auth.login_failed".into()],
    )
    .unwrap();

    assert_eq!(first.digest(), same_content.digest());
    assert_ne!(first.digest(), next_version.digest());
    assert_eq!(first.version(), 1);
    assert_eq!(
        first.provenance(),
        PlatformEventCatalogProvenance {
            version: 1,
            digest: first.digest().to_owned(),
        }
    );
    let serialized = serde_json::to_value(first.provenance()).unwrap();
    assert_eq!(serialized["version"], 1);
    assert_eq!(serialized["digest"], first.digest());

    let run_source_plan = PlatformSourceReadPlan::with_event_catalog(first.clone());
    assert_eq!(
        run_source_plan.provenance(),
        PlatformSourcePolicyProvenance {
            event_catalog: first.provenance(),
        }
    );
}

#[test]
fn simulated_customers_are_excluded_from_the_platform_population() {
    assert!(!include_customer_in_population(true));
    assert!(include_customer_in_population(false));
}

#[derive(Default)]
struct CapturingBroker {
    inputs: Vec<String>,
}

impl ProjectionBrokerPort for CapturingBroker {
    fn authorize_projection(
        &mut self,
        _scope: &CoreTaskScope,
        _policy: &ModelPolicy,
        treated_input: String,
    ) -> Result<VerifiedProjection, ModelProviderError> {
        self.inputs.push(treated_input);
        Err(ModelProviderError::InvalidProjectionAuthorization)
    }
}

#[test]
fn unmarked_personal_text_is_blocked_before_reaching_the_egress_broker() {
    let mut broker = CapturingBroker::default();
    let text = PlatformSourceText::turn_body(PlatformTurnBody::from_source(
        "Hi, I am Maria Perez. Email maria.perez@example.com or call +57 310 555 0198; case CUS-123",
    ));
    assert_eq!(
        format!("{text:?}"),
        "PlatformSourceText { value: \"<local-only/redacted>\" }"
    );
    let error = text
        .authorize_model_egress(
            None,
            &CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap(),
            &policy(),
            &mut broker,
        )
        .expect_err("unverified raw text must fail closed");

    assert_eq!(
        error,
        PlatformTextEgressError::TreatmentAuthorityUnavailable
    );
    assert!(
        broker.inputs.is_empty(),
        "raw text must never reach the broker"
    );
}

#[test]
fn local_only_event_payload_cannot_be_authorized_for_model_egress() {
    let payload = PlatformEventPayloadLocalOnly::from_source("{\"note\":\"secret\"}");
    let mut broker = CapturingBroker::default();
    let error = payload
        .authorize_model_egress(
            &CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap(),
            &policy(),
            &mut broker,
        )
        .expect_err("event payload is local-only regardless of model policy");

    assert_eq!(error, PlatformTextEgressError::LocalOnlyEventPayload);
    assert!(broker.inputs.is_empty());
}

fn policy() -> ModelPolicy {
    use improvement_engine_core::model_provider::{
        ModelBudgetLimits, ModelCapability, ModelProvider, RedactionPolicy,
    };
    ModelPolicy::with_budget(
        "policy_a",
        ModelCapability::new(
            ModelProvider::OpenRouter,
            "https://openrouter.ai/api/v1",
            "openai/gpt-4.1-mini",
            "secret://pulso/test",
            "test-v1",
        )
        .unwrap(),
        "platform_investigation",
        RedactionPolicy::TokenizeKnownMarkers,
        0,
        1_000,
        ModelBudgetLimits::new(2_000, 100, 1_000).unwrap(),
    )
    .unwrap()
}
