//! Governed consumer-side boundary for external model providers.
//!
//! This is deliberately neither an LLM gateway nor an Agent Core runtime. It
//! accepts a capability already configured by the control plane, reduces an
//! authorized view according to a pinned redaction policy, and records a
//! conservative receipt. Secret material is referenced but never accepted,
//! stored, logged, hashed or returned by this module.

use crate::core_task::CoreTaskScope;
use postgres::Client;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ModelProvider {
    OpenRouter,
    AgentCore,
}

impl ModelProvider {
    fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::AgentCore => "agent_core",
        }
    }

    fn approved_endpoint(self, endpoint: &str) -> bool {
        match self {
            Self::OpenRouter => endpoint == "https://openrouter.ai/api/v1",
            // Agent Core owns its endpoint configuration. Pulso only sends a
            // request to the pinned local bridge in a future adapter.
            Self::AgentCore => endpoint == "https://agent-core.internal/v1",
        }
    }
}

/// A provider/model pair and a *reference* to externally-held credentials.
/// This is a configuration artifact, not a secret store.
#[derive(Clone, Eq, PartialEq)]
pub struct ModelCapability {
    provider: ModelProvider,
    endpoint: String,
    model: String,
    credential_ref: String,
    credential_alias: String,
    revision: String,
    digest: String,
}

impl ModelCapability {
    pub fn new(
        provider: ModelProvider,
        endpoint: impl Into<String>,
        model: impl Into<String>,
        credential_ref: impl Into<String>,
        revision: impl Into<String>,
    ) -> Result<Self, ModelProviderError> {
        let revision = revision.into();
        Self::new_with_credential_alias(
            provider,
            endpoint,
            model,
            credential_ref,
            revision.clone(),
            revision,
        )
    }

    pub fn new_with_credential_alias(
        provider: ModelProvider,
        endpoint: impl Into<String>,
        model: impl Into<String>,
        credential_ref: impl Into<String>,
        credential_alias: impl Into<String>,
        revision: impl Into<String>,
    ) -> Result<Self, ModelProviderError> {
        let endpoint = endpoint.into();
        let model = model.into();
        let credential_ref = credential_ref.into();
        let credential_alias = credential_alias.into();
        let revision = revision.into();
        if !provider.approved_endpoint(&endpoint) {
            return Err(ModelProviderError::UnapprovedEndpoint);
        }
        if !is_model_name(&model) || !is_identifier(&revision) || !is_identifier(&credential_alias)
        {
            return Err(ModelProviderError::InvalidCapability);
        }
        if !is_secret_reference(&credential_ref) {
            return Err(ModelProviderError::InvalidSecretReference);
        }
        let digest = digest(
            "model-capability",
            &[
                ("provider", provider.as_str()),
                ("endpoint", &endpoint),
                ("model", &model),
                // Secret locators are never hashed. A configured, non-secret
                // alias supports safe rotation/correlation instead.
                ("credential_alias", &credential_alias),
                ("revision", &revision),
            ],
        );
        Ok(Self {
            provider,
            endpoint,
            model,
            credential_ref,
            credential_alias,
            revision,
            digest,
        })
    }

    pub fn provider(&self) -> ModelProvider {
        self.provider
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn credential_ref(&self) -> &str {
        &self.credential_ref
    }
    pub fn credential_alias(&self) -> &str {
        &self.credential_alias
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

impl fmt::Debug for ModelCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelCapability")
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("credential_alias", &self.credential_alias)
            .field("credential_ref", &"<redacted>")
            .field("revision", &self.revision)
            .field("digest", &self.digest)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedactionPolicy {
    TokenizeKnownMarkers,
    RejectMarkedInput,
}

impl RedactionPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::TokenizeKnownMarkers => "tokenize_known_markers",
            Self::RejectMarkedInput => "reject_marked_input",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelBudgetLimits {
    max_input_units: u64,
    max_output_units: u64,
    max_cost_micros: u64,
}
impl ModelBudgetLimits {
    pub fn new(
        max_input_units: u64,
        max_output_units: u64,
        max_cost_micros: u64,
    ) -> Result<Self, ModelProviderError> {
        if max_input_units == 0 || max_output_units == 0 || max_cost_micros == 0 {
            return Err(ModelProviderError::InvalidPolicy);
        }
        Ok(Self {
            max_input_units,
            max_output_units,
            max_cost_micros,
        })
    }
    pub fn max_input_units(&self) -> u64 {
        self.max_input_units
    }
    pub fn max_output_units(&self) -> u64 {
        self.max_output_units
    }
    pub fn max_cost_micros(&self) -> u64 {
        self.max_cost_micros
    }
}

/// Immutable operating policy; live quotas/grants remain owned by U05/U06.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelPolicy {
    id: String,
    capability: ModelCapability,
    purpose: String,
    redaction: RedactionPolicy,
    max_pre_dispatch_retries: u8,
    timeout_ms: u32,
    limits: ModelBudgetLimits,
    digest: String,
}

impl ModelPolicy {
    pub fn new(
        id: impl Into<String>,
        capability: ModelCapability,
        purpose: impl Into<String>,
        redaction: RedactionPolicy,
        max_pre_dispatch_retries: u8,
        timeout_ms: u32,
    ) -> Result<Self, ModelProviderError> {
        Self::with_budget(
            id,
            capability,
            purpose,
            redaction,
            max_pre_dispatch_retries,
            timeout_ms,
            ModelBudgetLimits::new(10_000, 10_000, 1_000_000).expect("fixed defaults are valid"),
        )
    }

    pub fn with_budget(
        id: impl Into<String>,
        capability: ModelCapability,
        purpose: impl Into<String>,
        redaction: RedactionPolicy,
        max_pre_dispatch_retries: u8,
        timeout_ms: u32,
        limits: ModelBudgetLimits,
    ) -> Result<Self, ModelProviderError> {
        let id = id.into();
        let purpose = purpose.into();
        if !is_identifier(&id)
            || !is_identifier(&purpose)
            || timeout_ms == 0
            || max_pre_dispatch_retries > 5
        {
            return Err(ModelProviderError::InvalidPolicy);
        }
        let digest = digest(
            "model-policy",
            &[
                ("id", &id),
                ("capability", capability.digest()),
                ("purpose", &purpose),
                ("redaction", redaction.as_str()),
                (
                    "max_pre_dispatch_retries",
                    &max_pre_dispatch_retries.to_string(),
                ),
                ("timeout_ms", &timeout_ms.to_string()),
                ("max_input_units", &limits.max_input_units.to_string()),
                ("max_output_units", &limits.max_output_units.to_string()),
                ("max_cost_micros", &limits.max_cost_micros.to_string()),
            ],
        );
        Ok(Self {
            id,
            capability,
            purpose,
            redaction,
            max_pre_dispatch_retries,
            timeout_ms,
            limits,
            digest,
        })
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn capability(&self) -> &ModelCapability {
        &self.capability
    }
    pub fn purpose(&self) -> &str {
        &self.purpose
    }
    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
    pub fn max_pre_dispatch_retries(&self) -> u8 {
        self.max_pre_dispatch_retries
    }
    pub fn limits(&self) -> &ModelBudgetLimits {
        &self.limits
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ModelInvocation {
    scope: CoreTaskScope,
    policy: ModelPolicy,
    attempt_id: String,
    /// Treated projection only; it never enters a receipt or error.
    redacted_input: String,
    input_commitment: String,
}

impl fmt::Debug for ModelInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelInvocation")
            .field("scope", &self.scope)
            .field("policy_digest", &self.policy.digest)
            .field("attempt_id", &self.attempt_id)
            .field("input", &"<redacted>")
            .field("input_commitment", &self.input_commitment)
            .finish()
    }
}

impl ModelInvocation {
    pub fn from_verified(
        scope: CoreTaskScope,
        policy: ModelPolicy,
        attempt_id: impl Into<String>,
        projection: VerifiedProjection,
    ) -> Result<Self, ModelProviderError> {
        let attempt_id = attempt_id.into();
        if !is_identifier(&attempt_id)
            || projection.scope != scope
            || projection.policy_digest != policy.digest
        {
            return Err(ModelProviderError::InvalidProjectionAuthorization);
        }
        if projection.treated_input.is_empty()
            || projection.treated_input.len() as u64 > policy.limits.max_input_units
        {
            return Err(ModelProviderError::InvalidInput);
        }
        Ok(Self {
            scope,
            policy,
            attempt_id,
            redacted_input: projection.treated_input,
            input_commitment: projection.commitment,
        })
    }
    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }
    pub fn policy(&self) -> &ModelPolicy {
        &self.policy
    }
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
    pub fn input_commitment(&self) -> &str {
        &self.input_commitment
    }
    pub fn provider_idempotency_key(&self) -> String {
        provider_idempotency_key(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelOutcome {
    Succeeded,
    BudgetDenied,
    BudgetExceededAfterDispatch,
    OutputLimitExceededAfterDispatch,
    DependencyUnavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptLifecycle {
    Terminal,
    RetryableBeforeDispatch,
    /// A durable worker claimed the effect before egress. A restart must ask
    /// the reconciliation path, not issue another provider request.
    Dispatching,
    ReconcileBeforeRetry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelUsage {
    input_units: u64,
    output_units: u64,
    cost_micros: u64,
}
impl ModelUsage {
    pub fn new(
        input_units: u64,
        output_units: u64,
        cost_micros: u64,
    ) -> Result<Self, ModelProviderError> {
        if input_units == 0 && output_units == 0 {
            return Err(ModelProviderError::InvalidUsage);
        }
        Ok(Self {
            input_units,
            output_units,
            cost_micros,
        })
    }
    pub fn input_units(&self) -> u64 {
        self.input_units
    }
    pub fn output_units(&self) -> u64 {
        self.output_units
    }
    pub fn cost_micros(&self) -> u64 {
        self.cost_micros
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ModelReceipt {
    scope: CoreTaskScope,
    attempt_id: String,
    provider: ModelProvider,
    capability_digest: String,
    policy_digest: String,
    input_commitment: String,
    outcome: ModelOutcome,
    lifecycle: AttemptLifecycle,
    pre_dispatch_retries_used: u8,
    pre_dispatch_retry_exhausted: bool,
    provider_request_id: Option<String>,
    output_digest: Option<String>,
    usage: Option<ModelUsage>,
    budget_receipt_id: Option<String>,
    budget_settlement: BudgetSettlement,
    evidence: String,
}
impl ModelReceipt {
    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
    pub fn provider(&self) -> ModelProvider {
        self.provider
    }
    pub fn capability_digest(&self) -> &str {
        &self.capability_digest
    }
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    pub fn input_commitment(&self) -> &str {
        &self.input_commitment
    }
    pub fn outcome(&self) -> &ModelOutcome {
        &self.outcome
    }
    pub fn lifecycle(&self) -> AttemptLifecycle {
        self.lifecycle
    }
    pub fn pre_dispatch_retries_used(&self) -> u8 {
        self.pre_dispatch_retries_used
    }
    pub fn pre_dispatch_retry_exhausted(&self) -> bool {
        self.pre_dispatch_retry_exhausted
    }
    pub fn provider_request_id(&self) -> Option<&str> {
        self.provider_request_id.as_deref()
    }
    pub fn output_digest(&self) -> Option<&str> {
        self.output_digest.as_deref()
    }
    pub fn evidence(&self) -> &str {
        &self.evidence
    }
    pub fn usage(&self) -> Option<&ModelUsage> {
        self.usage.as_ref()
    }
    pub fn budget_receipt_id(&self) -> Option<&str> {
        self.budget_receipt_id.as_deref()
    }
    pub fn budget_settlement(&self) -> BudgetSettlement {
        self.budget_settlement
    }
}

impl fmt::Debug for ModelReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelReceipt")
            .field("scope", &self.scope)
            .field("attempt_id", &self.attempt_id)
            .field("provider", &self.provider)
            .field("capability_digest", &self.capability_digest)
            .field("policy_digest", &self.policy_digest)
            .field("input_commitment", &self.input_commitment)
            .field("outcome", &self.outcome)
            .field("lifecycle", &self.lifecycle)
            .field("pre_dispatch_retries_used", &self.pre_dispatch_retries_used)
            .field(
                "pre_dispatch_retry_exhausted",
                &self.pre_dispatch_retry_exhausted,
            )
            .field("provider_request_id", &"<redacted>")
            .field("output_digest", &self.output_digest)
            .field("usage", &self.usage)
            .field("budget_receipt_id", &self.budget_receipt_id)
            .field("budget_settlement", &self.budget_settlement)
            .field("evidence", &self.evidence)
            .finish()
    }
}

pub trait ModelPort {
    fn invoke(&mut self, invocation: ModelInvocation) -> Result<ModelReceipt, ModelProviderError>;
}

/// Trusted egress broker boundary. Only an implementation backed by the
/// active treatment/grant authority may mint `VerifiedProjection`; callers
/// cannot fabricate a string commitment at the model boundary.
pub trait ProjectionBrokerPort {
    fn authorize_projection(
        &mut self,
        scope: &CoreTaskScope,
        policy: &ModelPolicy,
        treated_input: String,
    ) -> Result<VerifiedProjection, ModelProviderError>;
}

/// Reference broker for local integration tests.  Production composition must
/// bind this port to the treatment/grant authority (which owns its key and
/// verifies source, policy, grant and authority before minting a projection).
/// The key is never accepted by the model adapter or written to receipts.
pub struct HmacProjectionBroker {
    key: Vec<u8>,
}

impl HmacProjectionBroker {
    #[cfg(feature = "test-support")]
    pub fn new_for_test(key: impl AsRef<[u8]>) -> Result<Self, ModelProviderError> {
        let key = key.as_ref().to_vec();
        if key.len() < 32 {
            return Err(ModelProviderError::InvalidProjectionAuthorization);
        }
        Ok(Self { key })
    }
}

impl ProjectionBrokerPort for HmacProjectionBroker {
    fn authorize_projection(
        &mut self,
        scope: &CoreTaskScope,
        policy: &ModelPolicy,
        treated_input: String,
    ) -> Result<VerifiedProjection, ModelProviderError> {
        let treated_input = redact(&treated_input, policy.redaction)?;
        if treated_input.is_empty() || treated_input.len() as u64 > policy.limits.max_input_units {
            return Err(ModelProviderError::InvalidInput);
        }
        let values = [
            scope.tenant_id(),
            scope.job_id(),
            scope.grant_id(),
            scope.authority_ref(),
            policy.digest(),
            &treated_input,
        ];
        let commitment = format!(
            "projection_hmac:{}:{}",
            scope.tenant_id(),
            hmac_sha256_hex(&self.key, &values)
        );
        Ok(VerifiedProjection::new(
            scope.clone(),
            policy.digest.clone(),
            treated_input,
            commitment,
        ))
    }
}

#[derive(Clone)]
pub struct VerifiedProjection {
    scope: CoreTaskScope,
    policy_digest: String,
    treated_input: String,
    commitment: String,
}
impl VerifiedProjection {
    fn new(
        scope: CoreTaskScope,
        policy_digest: String,
        treated_input: String,
        commitment: String,
    ) -> Self {
        Self {
            scope,
            policy_digest,
            treated_input,
            commitment,
        }
    }
}
impl fmt::Debug for VerifiedProjection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedProjection")
            .field("scope", &self.scope)
            .field("policy_digest", &self.policy_digest)
            .field("treated_input", &"<redacted>")
            .field("commitment", &self.commitment)
            .finish()
    }
}

/// U06 persistence seam. Production must persist this state beside the job
/// effect state with conditional writes; the in-memory adapter is test-only.
pub trait ModelAttemptRepository {
    fn load(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        attempt_id: &str,
    ) -> Result<Option<ModelAttemptState>, ModelProviderError>;
    /// Compare-and-swap is the concurrency boundary: a stale worker cannot
    /// overwrite an `Unknown`, terminal or exhausted retry state.
    fn compare_and_store(
        &mut self,
        expected_revision: Option<u64>,
        state: ModelAttemptState,
    ) -> Result<ModelAttemptState, ModelProviderError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelAttemptState {
    scope: CoreTaskScope,
    policy_digest: String,
    input_commitment: String,
    receipt: ModelReceipt,
    revision: u64,
}
impl ModelAttemptState {
    fn new(
        scope: CoreTaskScope,
        policy_digest: String,
        input_commitment: String,
        receipt: ModelReceipt,
        revision: u64,
    ) -> Self {
        Self {
            scope,
            policy_digest,
            input_commitment,
            receipt,
            revision,
        }
    }
    pub fn receipt(&self) -> &ModelReceipt {
        &self.receipt
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// PostgreSQL implementation boundary for the U06 control-plane ledger.  The
/// adapter is intentionally constructed with the already-configured U06
/// connection; it never opens or owns a separate database.
pub struct PostgresModelAttemptRepository {
    client: Client,
}

impl PostgresModelAttemptRepository {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl ModelAttemptRepository for PostgresModelAttemptRepository {
    fn load(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        attempt_id: &str,
    ) -> Result<Option<ModelAttemptState>, ModelProviderError> {
        let row = self.client.query_opt(
            "SELECT revision, state_json FROM pulso_model_attempts WHERE tenant_id = $1 AND job_id = $2 AND attempt_id = $3",
            &[&tenant_id, &job_id, &attempt_id],
        ).map_err(storage_model_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let revision = row.get::<_, i64>(0);
        if revision < 1 {
            return Err(ModelProviderError::Storage);
        }
        state_from_json(row.get(1), revision as u64)
    }

    fn compare_and_store(
        &mut self,
        expected_revision: Option<u64>,
        mut state: ModelAttemptState,
    ) -> Result<ModelAttemptState, ModelProviderError> {
        let expected = expected_revision
            .map(i64::try_from)
            .transpose()
            .map_err(|_| ModelProviderError::Storage)?;
        let payload = state_to_json(&state);
        let lifecycle = lifecycle_name(state.receipt.lifecycle);
        let retries = i32::from(state.receipt.pre_dispatch_retries_used);
        let exhausted = state.receipt.pre_dispatch_retry_exhausted;
        let reservation = state.receipt.budget_receipt_id.as_deref();
        let row = match expected {
            None => self.client.query_opt(
                "INSERT INTO pulso_model_attempts (tenant_id, job_id, attempt_id, policy_digest, input_commitment, lifecycle, retries_used, retry_exhausted, budget_reservation_id, state_json) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT (tenant_id, job_id, attempt_id) DO NOTHING RETURNING revision",
                &[&state.scope.tenant_id(), &state.scope.job_id(), &state.receipt.attempt_id, &state.policy_digest, &state.input_commitment, &lifecycle, &retries, &exhausted, &reservation, &payload],
            ),
            Some(expected) => self.client.query_opt(
                "UPDATE pulso_model_attempts SET lifecycle=$1, retries_used=$2, retry_exhausted=$3, budget_reservation_id=$4, state_json=$5, revision=revision+1 WHERE tenant_id=$6 AND job_id=$7 AND attempt_id=$8 AND revision=$9 RETURNING revision",
                &[&lifecycle, &retries, &exhausted, &reservation, &payload, &state.scope.tenant_id(), &state.scope.job_id(), &state.receipt.attempt_id, &expected],
            ),
        }.map_err(storage_model_error)?;
        let Some(row) = row else {
            return Err(ModelProviderError::AttemptStateConflict);
        };
        let revision = row.get::<_, i64>(0);
        if revision < 1 {
            return Err(ModelProviderError::Storage);
        }
        state.revision = revision as u64;
        Ok(state)
    }
}

/// Test/reference implementation only; unlike a U06-backed adapter this does
/// not survive restart and must never be selected for production composition.
#[derive(Default)]
pub struct InMemoryModelAttemptRepository {
    receipts: BTreeMap<(String, String, String), ModelAttemptState>,
}
impl ModelAttemptRepository for InMemoryModelAttemptRepository {
    fn load(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        attempt_id: &str,
    ) -> Result<Option<ModelAttemptState>, ModelProviderError> {
        Ok(self
            .receipts
            .get(&(tenant_id.into(), job_id.into(), attempt_id.into()))
            .cloned())
    }
    fn compare_and_store(
        &mut self,
        expected_revision: Option<u64>,
        mut state: ModelAttemptState,
    ) -> Result<ModelAttemptState, ModelProviderError> {
        let key = (
            state.scope.tenant_id().into(),
            state.scope.job_id().into(),
            state.receipt.attempt_id.clone(),
        );
        let actual = self.receipts.get(&key).map(ModelAttemptState::revision);
        if actual != expected_revision {
            return Err(ModelProviderError::AttemptStateConflict);
        }
        state.revision = actual
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(ModelProviderError::AttemptStateConflict)?;
        self.receipts.insert(key, state.clone());
        Ok(state)
    }
}

/// Restart-test adapter for the U06 repository contract.  Separate model
/// adapter instances share the same durable state; production uses the
/// PostgreSQL implementation instead.
type AttemptStateIndex = BTreeMap<(String, String, String), ModelAttemptState>;

#[derive(Clone, Default)]
pub struct SharedModelAttemptRepository {
    receipts: Arc<Mutex<AttemptStateIndex>>,
}
impl ModelAttemptRepository for SharedModelAttemptRepository {
    fn load(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        attempt_id: &str,
    ) -> Result<Option<ModelAttemptState>, ModelProviderError> {
        Ok(self
            .receipts
            .lock()
            .map_err(|_| ModelProviderError::Storage)?
            .get(&(tenant_id.into(), job_id.into(), attempt_id.into()))
            .cloned())
    }
    fn compare_and_store(
        &mut self,
        expected_revision: Option<u64>,
        mut state: ModelAttemptState,
    ) -> Result<ModelAttemptState, ModelProviderError> {
        let mut records = self
            .receipts
            .lock()
            .map_err(|_| ModelProviderError::Storage)?;
        let key = (
            state.scope.tenant_id().into(),
            state.scope.job_id().into(),
            state.receipt.attempt_id.clone(),
        );
        let actual = records.get(&key).map(ModelAttemptState::revision);
        if actual != expected_revision {
            return Err(ModelProviderError::AttemptStateConflict);
        }
        state.revision = actual
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(ModelProviderError::AttemptStateConflict)?;
        records.insert(key, state.clone());
        Ok(state)
    }
}

/// Consumer seam implemented by the U05/U06 quota/admission adapter. It must
/// reserve idempotently using the supplied attempt identity; this module never
/// mints a grant or decides a budget itself.
pub trait ModelBudgetPort {
    fn admit(&mut self, request: ModelBudgetRequest) -> BudgetAdmission;
    fn release(&mut self, _reservation_id: &str, _reason: BudgetReleaseReason) -> BudgetSettlement {
        BudgetSettlement::Released
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetReleaseReason {
    PreDispatchRetryExhausted,
    DurableClaimFailedBeforeDispatch,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetSettlement {
    NotApplicable,
    HeldForRetry,
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelBudgetRequest {
    scope: CoreTaskScope,
    attempt_id: String,
    policy_digest: String,
    capability_digest: String,
    input_commitment: String,
    max_input_units: u64,
    max_output_units: u64,
    max_response_bytes: u64,
    max_cost_micros: u64,
}
impl ModelBudgetRequest {
    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    pub fn capability_digest(&self) -> &str {
        &self.capability_digest
    }
    pub fn input_commitment(&self) -> &str {
        &self.input_commitment
    }
    pub fn max_input_units(&self) -> u64 {
        self.max_input_units
    }
    pub fn max_output_units(&self) -> u64 {
        self.max_output_units
    }
    pub fn max_response_bytes(&self) -> u64 {
        self.max_response_bytes
    }
    pub fn max_cost_micros(&self) -> u64 {
        self.max_cost_micros
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BudgetAdmission {
    Admitted { reservation_id: String },
    Denied { reason_code: String },
    DependencyUnavailable { reason_code: String },
}

#[derive(Default)]
pub struct PermissiveModelBudget;
impl ModelBudgetPort for PermissiveModelBudget {
    fn admit(&mut self, request: ModelBudgetRequest) -> BudgetAdmission {
        BudgetAdmission::Admitted {
            reservation_id: digest(
                "model-budget",
                &[
                    ("tenant", request.scope.tenant_id()),
                    ("job", request.scope.job_id()),
                    ("grant", request.scope.grant_id()),
                    ("authority", request.scope.authority_ref()),
                    ("attempt", &request.attempt_id),
                    ("policy", &request.policy_digest),
                    ("capability", &request.capability_digest),
                    ("input", &request.input_commitment),
                ],
            ),
        }
    }
}

/// Boundary a real OpenAI-compatible/OpenRouter HTTP adapter must implement.
/// `credential_ref` is intentionally passed as an opaque reference, so the
/// secret resolver stays outside the engine and logs cannot accidentally carry
/// secret bytes.
pub trait OpenAiCompatibleTransport {
    fn complete(&mut self, request: OpenAiCompatibleRequest) -> TransportResult;
}

#[derive(Clone, Eq, PartialEq)]
pub struct OpenAiCompatibleRequest {
    provider: ModelProvider,
    endpoint: String,
    model: String,
    credential_ref: String,
    purpose: String,
    timeout_ms: u32,
    max_output_units: u64,
    max_response_bytes: u64,
    max_cost_micros: u64,
    idempotency_key: String,
    input: String,
}
impl OpenAiCompatibleRequest {
    pub fn provider(&self) -> ModelProvider {
        self.provider
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn credential_ref(&self) -> &str {
        &self.credential_ref
    }
    pub fn purpose(&self) -> &str {
        &self.purpose
    }
    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
    pub fn max_output_units(&self) -> u64 {
        self.max_output_units
    }
    pub fn max_response_bytes(&self) -> u64 {
        self.max_response_bytes
    }
    pub fn max_cost_micros(&self) -> u64 {
        self.max_cost_micros
    }
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
    pub fn input(&self) -> &str {
        &self.input
    }
}
impl fmt::Debug for OpenAiCompatibleRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatibleRequest")
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("credential_ref", &"<redacted>")
            .field("purpose", &self.purpose)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_output_units", &self.max_output_units)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("max_cost_micros", &self.max_cost_micros)
            .field("idempotency_key", &self.idempotency_key)
            .field("input", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportResult {
    Succeeded {
        provider_request_id: String,
        output: String,
        usage: ModelUsage,
    },
    /// Nothing left the process; retry can be bounded by policy.
    PreDispatchTransient {
        class: String,
    },
    /// The destination may have received the request; never retry blindly.
    UnknownAfterDispatch {
        class: String,
    },
    DependencyUnavailable {
        class: String,
    },
}

/// The actual consumer adapter. It accepts a narrow transport rather than
/// hiding HTTP/client logic inside domain code, allowing an OpenRouter client,
/// Agent Core bridge, or test double to implement the same safety semantics.
pub struct GovernedModelAdapter<T, B = PermissiveModelBudget, R = InMemoryModelAttemptRepository> {
    policy: ModelPolicy,
    transport: T,
    budget: B,
    attempts: R,
    dispatch_count: u64,
}
impl<T> GovernedModelAdapter<T> {
    pub fn new(policy: ModelPolicy, transport: T) -> Self {
        Self {
            policy,
            transport,
            budget: PermissiveModelBudget,
            attempts: InMemoryModelAttemptRepository::default(),
            dispatch_count: 0,
        }
    }
}
impl<T, B> GovernedModelAdapter<T, B> {
    pub fn with_budget(policy: ModelPolicy, transport: T, budget: B) -> Self {
        Self {
            policy,
            transport,
            budget,
            attempts: InMemoryModelAttemptRepository::default(),
            dispatch_count: 0,
        }
    }
}
impl<T, B, R: ModelAttemptRepository> GovernedModelAdapter<T, B, R> {
    pub fn with_dependencies(policy: ModelPolicy, transport: T, budget: B, attempts: R) -> Self {
        Self {
            policy,
            transport,
            budget,
            attempts,
            dispatch_count: 0,
        }
    }
    fn store_attempt(
        &mut self,
        invocation: &ModelInvocation,
        old: Option<&ModelAttemptState>,
        receipt: ModelReceipt,
    ) -> Result<ModelAttemptState, ModelProviderError> {
        self.attempts.compare_and_store(
            old.map(ModelAttemptState::revision),
            ModelAttemptState::new(
                invocation.scope.clone(),
                invocation.policy.digest.clone(),
                invocation.input_commitment.clone(),
                receipt,
                0,
            ),
        )
    }
    pub fn dispatch_count(&self) -> u64 {
        self.dispatch_count
    }
    pub fn into_transport(self) -> T {
        self.transport
    }
}
impl<T: OpenAiCompatibleTransport, B: ModelBudgetPort, R: ModelAttemptRepository> ModelPort
    for GovernedModelAdapter<T, B, R>
{
    fn invoke(&mut self, invocation: ModelInvocation) -> Result<ModelReceipt, ModelProviderError> {
        if invocation.policy != self.policy {
            return Err(ModelProviderError::UnsupportedPolicy);
        }
        let existing = self.attempts.load(
            invocation.scope.tenant_id(),
            invocation.scope.job_id(),
            &invocation.attempt_id,
        )?;
        if let Some(old) = existing.as_ref() {
            if old.scope != invocation.scope {
                return Err(ModelProviderError::ScopeMismatch {
                    attempt_id: invocation.attempt_id,
                });
            }
            if old.policy_digest != invocation.policy.digest
                || old.input_commitment != invocation.input_commitment
            {
                return Err(ModelProviderError::AttemptConflict {
                    attempt_id: invocation.attempt_id,
                });
            }
            if old.receipt.lifecycle != AttemptLifecycle::RetryableBeforeDispatch {
                return Ok(old.receipt.clone());
            }
            if old.receipt.pre_dispatch_retry_exhausted {
                return Ok(old.receipt.clone());
            }
        }
        let mut retry = existing
            .as_ref()
            .map_or(0, |old| old.receipt.pre_dispatch_retries_used);
        let reservation_id = existing
            .as_ref()
            .and_then(|old| old.receipt.budget_receipt_id.clone());
        let reservation_id = if reservation_id.is_some() {
            reservation_id
        } else {
            match self.budget.admit(ModelBudgetRequest {
                scope: invocation.scope.clone(),
                attempt_id: invocation.attempt_id.clone(),
                policy_digest: invocation.policy.digest.clone(),
                capability_digest: invocation.policy.capability.digest.clone(),
                input_commitment: invocation.input_commitment.clone(),
                max_input_units: invocation.policy.limits.max_input_units,
                max_output_units: invocation.policy.limits.max_output_units,
                max_response_bytes: invocation.policy.limits.max_output_units.saturating_mul(4),
                max_cost_micros: invocation.policy.limits.max_cost_micros,
            }) {
                BudgetAdmission::Admitted { reservation_id } => Some(reservation_id),
                BudgetAdmission::Denied { reason_code } => {
                    let receipt = receipt_before_dispatch(
                        &invocation,
                        ModelOutcome::BudgetDenied,
                        AttemptLifecycle::Terminal,
                        reason_code,
                        None,
                    );
                    self.store_attempt(&invocation, existing.as_ref(), receipt.clone())?;
                    return Ok(receipt);
                }
                BudgetAdmission::DependencyUnavailable { reason_code } => {
                    let receipt = receipt_before_dispatch(
                        &invocation,
                        ModelOutcome::DependencyUnavailable,
                        AttemptLifecycle::RetryableBeforeDispatch,
                        reason_code,
                        None,
                    );
                    self.store_attempt(&invocation, existing.as_ref(), receipt.clone())?;
                    return Ok(receipt);
                }
            }
        };
        // Claim durable ownership *before* outbound I/O. A competing worker or
        // restart sees `Dispatching` and reconciles instead of sending again.
        let claimed = receipt_before_dispatch(
            &invocation,
            ModelOutcome::Unknown,
            AttemptLifecycle::Dispatching,
            "dispatch_claimed".into(),
            reservation_id.clone(),
        );
        let claimed = match self.store_attempt(&invocation, existing.as_ref(), claimed) {
            Ok(claimed) => claimed,
            Err(error) => {
                if let Some(reservation_id) = reservation_id.as_deref() {
                    let _ = self.budget.release(
                        reservation_id,
                        BudgetReleaseReason::DurableClaimFailedBeforeDispatch,
                    );
                }
                return Err(error);
            }
        };
        let request = OpenAiCompatibleRequest {
            provider: invocation.policy.capability.provider,
            endpoint: invocation.policy.capability.endpoint.clone(),
            model: invocation.policy.capability.model.clone(),
            credential_ref: invocation.policy.capability.credential_ref.clone(),
            purpose: invocation.policy.purpose.clone(),
            timeout_ms: invocation.policy.timeout_ms,
            max_output_units: invocation.policy.limits.max_output_units,
            max_response_bytes: invocation.policy.limits.max_output_units.saturating_mul(4),
            max_cost_micros: invocation.policy.limits.max_cost_micros,
            idempotency_key: provider_idempotency_key(&invocation),
            input: invocation.redacted_input.clone(),
        };
        let outcome = self.transport.complete(request);
        // Persist each retry increment separately.  This makes a process crash
        // between transient failures incapable of resetting the retry budget.
        let receipt_retry = if matches!(outcome, TransportResult::PreDispatchTransient { .. }) {
            retry = retry.saturating_add(1);
            retry
        } else {
            retry
        };
        let mut receipt =
            receipt_from_transport(&invocation, outcome, receipt_retry, reservation_id.clone());
        if receipt.pre_dispatch_retry_exhausted {
            if let Some(reservation_id) = receipt.budget_receipt_id.as_deref() {
                receipt.budget_settlement = self.budget.release(
                    reservation_id,
                    BudgetReleaseReason::PreDispatchRetryExhausted,
                );
            }
        }
        // Dependency lookup/pre-dispatch failures do not count as an external
        // model dispatch. A transport implementation must classify those
        // before it writes bytes to the provider. `Unknown` conservatively
        // counts: the remote side may have received the request.
        if !matches!(
            receipt.outcome,
            ModelOutcome::DependencyUnavailable | ModelOutcome::BudgetDenied
        ) {
            self.dispatch_count = self
                .dispatch_count
                .checked_add(1)
                .ok_or(ModelProviderError::DispatchCountExhausted)?;
        }
        self.attempts.compare_and_store(
            Some(claimed.revision()),
            ModelAttemptState::new(
                invocation.scope,
                invocation.policy.digest,
                invocation.input_commitment,
                receipt.clone(),
                0,
            ),
        )?;
        Ok(receipt)
    }
}

fn receipt_from_transport(
    invocation: &ModelInvocation,
    result: TransportResult,
    retries: u8,
    budget_receipt_id: Option<String>,
) -> ModelReceipt {
    let (outcome, lifecycle, provider_request_id, output_digest, usage, class) = match result {
        TransportResult::Succeeded {
            provider_request_id,
            output,
            usage,
        } => {
            let within_response_limit =
                output.len() as u64 <= invocation.policy.limits.max_output_units.saturating_mul(4);
            let output_digest = if within_response_limit {
                Some(digest("model-output", &[("output", &output)]))
            } else {
                None
            };
            let within_budget = usage.input_units <= invocation.policy.limits.max_input_units
                && usage.output_units <= invocation.policy.limits.max_output_units
                && usage.cost_micros <= invocation.policy.limits.max_cost_micros;
            (
                if !within_response_limit {
                    ModelOutcome::OutputLimitExceededAfterDispatch
                } else if within_budget {
                    ModelOutcome::Succeeded
                } else {
                    ModelOutcome::BudgetExceededAfterDispatch
                },
                AttemptLifecycle::Terminal,
                Some(provider_request_id),
                output_digest,
                Some(usage),
                "succeeded".to_owned(),
            )
        }
        TransportResult::PreDispatchTransient { class } => (
            ModelOutcome::DependencyUnavailable,
            AttemptLifecycle::RetryableBeforeDispatch,
            None,
            None,
            None,
            class,
        ),
        TransportResult::UnknownAfterDispatch { class } => (
            ModelOutcome::Unknown,
            AttemptLifecycle::ReconcileBeforeRetry,
            None,
            None,
            None,
            class,
        ),
        TransportResult::DependencyUnavailable { class } => (
            ModelOutcome::DependencyUnavailable,
            AttemptLifecycle::RetryableBeforeDispatch,
            None,
            None,
            None,
            class,
        ),
    };
    ModelReceipt {
        scope: invocation.scope.clone(),
        attempt_id: invocation.attempt_id.clone(),
        provider: invocation.policy.capability.provider,
        capability_digest: invocation.policy.capability.digest.clone(),
        policy_digest: invocation.policy.digest.clone(),
        input_commitment: invocation.input_commitment.clone(),
        outcome,
        lifecycle,
        pre_dispatch_retries_used: retries,
        pre_dispatch_retry_exhausted: lifecycle == AttemptLifecycle::RetryableBeforeDispatch
            && retries >= invocation.policy.max_pre_dispatch_retries,
        provider_request_id,
        output_digest,
        usage,
        budget_receipt_id,
        budget_settlement: if lifecycle == AttemptLifecycle::RetryableBeforeDispatch {
            BudgetSettlement::HeldForRetry
        } else {
            BudgetSettlement::NotApplicable
        },
        evidence: format!(
            "model_operation:{}:retries={retries}",
            sanitize_class(&class)
        ),
    }
}

fn receipt_before_dispatch(
    invocation: &ModelInvocation,
    outcome: ModelOutcome,
    lifecycle: AttemptLifecycle,
    class: String,
    budget_receipt_id: Option<String>,
) -> ModelReceipt {
    ModelReceipt {
        scope: invocation.scope.clone(),
        attempt_id: invocation.attempt_id.clone(),
        provider: invocation.policy.capability.provider,
        capability_digest: invocation.policy.capability.digest.clone(),
        policy_digest: invocation.policy.digest.clone(),
        input_commitment: invocation.input_commitment.clone(),
        outcome,
        lifecycle,
        pre_dispatch_retries_used: 0,
        pre_dispatch_retry_exhausted: false,
        provider_request_id: None,
        output_digest: None,
        usage: None,
        budget_receipt_id,
        budget_settlement: if lifecycle == AttemptLifecycle::RetryableBeforeDispatch {
            BudgetSettlement::HeldForRetry
        } else {
            BudgetSettlement::NotApplicable
        },
        evidence: format!("model_operation:{}:retries=0", sanitize_class(&class)),
    }
}

/// Deterministic fake used by local unit/integration tests. It deliberately
/// implements the transport seam rather than a separate, easier code path.
pub struct ModelProviderSimulator {
    inner: GovernedModelAdapter<ScriptedTransport>,
}
impl ModelProviderSimulator {
    pub fn new(policy: ModelPolicy) -> Self {
        Self {
            inner: GovernedModelAdapter::new(policy, ScriptedTransport::default()),
        }
    }
    pub fn script_success(&mut self, output: impl Into<String>, request_id: impl Into<String>) {
        self.script_success_with_usage(
            output,
            request_id,
            ModelUsage::new(1, 1, 1).expect("valid scripted usage"),
        );
    }
    pub fn script_success_with_usage(
        &mut self,
        output: impl Into<String>,
        request_id: impl Into<String>,
        usage: ModelUsage,
    ) {
        self.inner
            .transport
            .script
            .push(TransportResult::Succeeded {
                provider_request_id: request_id.into(),
                output: output.into(),
                usage,
            });
    }
    pub fn script_timeout_after_dispatch(&mut self) {
        self.inner
            .transport
            .script
            .push(TransportResult::UnknownAfterDispatch {
                class: "timeout_after_dispatch".into(),
            });
    }
    pub fn script_dependency_unavailable(&mut self, class: impl Into<String>) {
        self.inner
            .transport
            .script
            .push(TransportResult::DependencyUnavailable {
                class: class.into(),
            });
    }
    pub fn script_pre_dispatch_transient(&mut self, class: impl Into<String>) {
        self.inner
            .transport
            .script
            .push(TransportResult::PreDispatchTransient {
                class: class.into(),
            });
    }
    pub fn dispatch_count(&self) -> u64 {
        self.inner.dispatch_count()
    }
    pub fn transport_attempt_count(&self) -> u64 {
        self.inner.transport.attempt_count
    }
}
impl ModelPort for ModelProviderSimulator {
    fn invoke(&mut self, invocation: ModelInvocation) -> Result<ModelReceipt, ModelProviderError> {
        self.inner.invoke(invocation)
    }
}
#[derive(Default)]
struct ScriptedTransport {
    script: Vec<TransportResult>,
    attempt_count: u64,
}
impl OpenAiCompatibleTransport for ScriptedTransport {
    fn complete(&mut self, _request: OpenAiCompatibleRequest) -> TransportResult {
        self.attempt_count = self.attempt_count.saturating_add(1);
        if self.script.is_empty() {
            TransportResult::DependencyUnavailable {
                class: "unscripted".into(),
            }
        } else {
            self.script.remove(0)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelProviderError {
    InvalidCapability,
    UnapprovedEndpoint,
    InvalidSecretReference,
    InvalidPolicy,
    InvalidAttempt,
    InvalidInput,
    InvalidProjectionAuthorization,
    InvalidUsage,
    MarkedInputRejected,
    UnsupportedPolicy,
    ScopeMismatch { attempt_id: String },
    AttemptConflict { attempt_id: String },
    AttemptStateConflict,
    Storage,
    DispatchCountExhausted,
}
impl fmt::Display for ModelProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCapability => f.write_str("model capability is invalid"),
            Self::UnapprovedEndpoint => {
                f.write_str("model endpoint is not approved for its provider")
            }
            Self::InvalidSecretReference => {
                f.write_str("credential must be an opaque secret reference, never a literal")
            }
            Self::InvalidPolicy => f.write_str("model policy is invalid"),
            Self::InvalidAttempt => f.write_str("attempt id is invalid"),
            Self::InvalidInput => f.write_str("treated model input is invalid"),
            Self::InvalidProjectionAuthorization => {
                f.write_str("projection was not authorized for this scope and policy")
            }
            Self::InvalidUsage => f.write_str("provider usage is invalid"),
            Self::MarkedInputRejected => f.write_str("policy rejects marked sensitive input"),
            Self::UnsupportedPolicy => f.write_str("model policy is not approved by this adapter"),
            Self::ScopeMismatch { .. } => f.write_str("attempt scope mismatch"),
            Self::AttemptConflict { .. } => f.write_str("attempt payload conflict"),
            Self::AttemptStateConflict => {
                f.write_str("durable model-attempt state changed concurrently")
            }
            Self::Storage => f.write_str("durable model-attempt storage is unavailable or invalid"),
            Self::DispatchCountExhausted => f.write_str("dispatch count exhausted"),
        }
    }
}
impl std::error::Error for ModelProviderError {}

fn storage_model_error(_: postgres::Error) -> ModelProviderError {
    ModelProviderError::Storage
}
fn lifecycle_name(value: AttemptLifecycle) -> &'static str {
    match value {
        AttemptLifecycle::Terminal => "terminal",
        AttemptLifecycle::RetryableBeforeDispatch => "retryable_before_dispatch",
        AttemptLifecycle::Dispatching => "dispatching",
        AttemptLifecycle::ReconcileBeforeRetry => "reconcile_before_retry",
    }
}
fn lifecycle_from_name(value: &str) -> Result<AttemptLifecycle, ModelProviderError> {
    match value {
        "terminal" => Ok(AttemptLifecycle::Terminal),
        "retryable_before_dispatch" => Ok(AttemptLifecycle::RetryableBeforeDispatch),
        "dispatching" => Ok(AttemptLifecycle::Dispatching),
        "reconcile_before_retry" => Ok(AttemptLifecycle::ReconcileBeforeRetry),
        _ => Err(ModelProviderError::Storage),
    }
}
fn outcome_name(value: &ModelOutcome) -> &'static str {
    match value {
        ModelOutcome::Succeeded => "succeeded",
        ModelOutcome::BudgetDenied => "budget_denied",
        ModelOutcome::BudgetExceededAfterDispatch => "budget_exceeded_after_dispatch",
        ModelOutcome::OutputLimitExceededAfterDispatch => "output_limit_exceeded_after_dispatch",
        ModelOutcome::DependencyUnavailable => "dependency_unavailable",
        ModelOutcome::Unknown => "unknown",
    }
}
fn outcome_from_name(value: &str) -> Result<ModelOutcome, ModelProviderError> {
    match value {
        "succeeded" => Ok(ModelOutcome::Succeeded),
        "budget_denied" => Ok(ModelOutcome::BudgetDenied),
        "budget_exceeded_after_dispatch" => Ok(ModelOutcome::BudgetExceededAfterDispatch),
        "output_limit_exceeded_after_dispatch" => {
            Ok(ModelOutcome::OutputLimitExceededAfterDispatch)
        }
        "dependency_unavailable" => Ok(ModelOutcome::DependencyUnavailable),
        "unknown" => Ok(ModelOutcome::Unknown),
        _ => Err(ModelProviderError::Storage),
    }
}
fn settlement_name(value: BudgetSettlement) -> &'static str {
    match value {
        BudgetSettlement::NotApplicable => "not_applicable",
        BudgetSettlement::HeldForRetry => "held_for_retry",
        BudgetSettlement::Released => "released",
    }
}
fn settlement_from_name(value: &str) -> Result<BudgetSettlement, ModelProviderError> {
    match value {
        "not_applicable" => Ok(BudgetSettlement::NotApplicable),
        "held_for_retry" => Ok(BudgetSettlement::HeldForRetry),
        "released" => Ok(BudgetSettlement::Released),
        _ => Err(ModelProviderError::Storage),
    }
}
fn provider_from_name(value: &str) -> Result<ModelProvider, ModelProviderError> {
    match value {
        "openrouter" => Ok(ModelProvider::OpenRouter),
        "agent_core" => Ok(ModelProvider::AgentCore),
        _ => Err(ModelProviderError::Storage),
    }
}

fn state_to_json(state: &ModelAttemptState) -> serde_json::Value {
    let receipt = &state.receipt;
    serde_json::json!({
        "tenant_id": state.scope.tenant_id(), "job_id": state.scope.job_id(),
        "grant_id": state.scope.grant_id(), "authority_ref": state.scope.authority_ref(),
        "policy_digest": state.policy_digest, "input_commitment": state.input_commitment,
        "attempt_id": receipt.attempt_id, "provider": receipt.provider.as_str(),
        "capability_digest": receipt.capability_digest, "outcome": outcome_name(&receipt.outcome),
        "lifecycle": lifecycle_name(receipt.lifecycle), "retries": receipt.pre_dispatch_retries_used,
        "exhausted": receipt.pre_dispatch_retry_exhausted, "provider_request_id": receipt.provider_request_id,
        "output_digest": receipt.output_digest, "usage": receipt.usage.as_ref().map(|u| serde_json::json!({"input":u.input_units,"output":u.output_units,"cost":u.cost_micros})),
        "budget_receipt_id": receipt.budget_receipt_id, "budget_settlement": settlement_name(receipt.budget_settlement), "evidence": receipt.evidence,
    })
}
fn json_string(value: &serde_json::Value, key: &str) -> Result<String, ModelProviderError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(ModelProviderError::Storage)
}
fn json_u64(value: &serde_json::Value, key: &str) -> Result<u64, ModelProviderError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .ok_or(ModelProviderError::Storage)
}
fn state_from_json(
    value: serde_json::Value,
    revision: u64,
) -> Result<Option<ModelAttemptState>, ModelProviderError> {
    let scope = CoreTaskScope::new(
        json_string(&value, "tenant_id")?,
        json_string(&value, "job_id")?,
        json_string(&value, "grant_id")?,
        json_string(&value, "authority_ref")?,
    )
    .map_err(|_| ModelProviderError::Storage)?;
    let usage = match value.get("usage") {
        None | Some(serde_json::Value::Null) => None,
        Some(usage) => Some(
            ModelUsage::new(
                json_u64(usage, "input")?,
                json_u64(usage, "output")?,
                json_u64(usage, "cost")?,
            )
            .map_err(|_| ModelProviderError::Storage)?,
        ),
    };
    let receipt = ModelReceipt {
        scope: scope.clone(),
        attempt_id: json_string(&value, "attempt_id")?,
        provider: provider_from_name(&json_string(&value, "provider")?)?,
        capability_digest: json_string(&value, "capability_digest")?,
        policy_digest: json_string(&value, "policy_digest")?,
        input_commitment: json_string(&value, "input_commitment")?,
        outcome: outcome_from_name(&json_string(&value, "outcome")?)?,
        lifecycle: lifecycle_from_name(&json_string(&value, "lifecycle")?)?,
        pre_dispatch_retries_used: u8::try_from(json_u64(&value, "retries")?)
            .map_err(|_| ModelProviderError::Storage)?,
        pre_dispatch_retry_exhausted: value
            .get("exhausted")
            .and_then(serde_json::Value::as_bool)
            .ok_or(ModelProviderError::Storage)?,
        provider_request_id: value
            .get("provider_request_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        output_digest: value
            .get("output_digest")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        usage,
        budget_receipt_id: value
            .get("budget_receipt_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        budget_settlement: settlement_from_name(&json_string(&value, "budget_settlement")?)?,
        evidence: json_string(&value, "evidence")?,
    };
    Ok(Some(ModelAttemptState::new(
        scope,
        receipt.policy_digest.clone(),
        receipt.input_commitment.clone(),
        receipt,
        revision,
    )))
}

fn redact(input: &str, policy: RedactionPolicy) -> Result<String, ModelProviderError> {
    let mut result = String::with_capacity(input.len());
    let mut remaining = input;
    while let Some(start) = remaining.find("[PII:") {
        result.push_str(&remaining[..start]);
        let tail = &remaining[start + 5..];
        let Some(end) = tail.find(']') else {
            return Err(ModelProviderError::InvalidInput);
        };
        if policy == RedactionPolicy::RejectMarkedInput {
            return Err(ModelProviderError::MarkedInputRejected);
        }
        result.push_str("[REDACTED]");
        remaining = &tail[end + 1..];
    }
    result.push_str(remaining);
    Ok(result)
}
fn provider_idempotency_key(invocation: &ModelInvocation) -> String {
    digest(
        "provider-attempt",
        &[
            ("tenant", invocation.scope.tenant_id()),
            ("job", invocation.scope.job_id()),
            ("grant", invocation.scope.grant_id()),
            ("authority", invocation.scope.authority_ref()),
            ("attempt", &invocation.attempt_id),
            ("policy", invocation.policy.digest()),
            ("capability", invocation.policy.capability.digest()),
            ("input", &invocation.input_commitment),
        ],
    )
}
fn sanitize_class(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
        .take(80)
        .collect()
}
fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}
fn is_model_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && value.bytes().all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'.' | b':' | b'_' | b'-'))
}
fn is_secret_reference(value: &str) -> bool {
    value.starts_with("secret://")
        && value.len() > 9
        && value.len() <= 255
        && value.bytes().all(
            |b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b':' | b'/' | b'_' | b'-'),
        )
}
/// Minimal SHA-256 HMAC implementation kept local to avoid exposing a key to
/// the adapter. It is used only inside the trusted projection-broker boundary.
fn hmac_sha256_hex(key: &[u8], values: &[&str]) -> String {
    let mut block = [0_u8; 64];
    if key.len() > block.len() {
        let digest = Sha256::digest(key);
        block[..digest.len()].copy_from_slice(&digest);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    inner.update(b"pulso:projection:v1\n");
    for value in values {
        inner.update((value.len() as u64).to_be_bytes());
        inner.update(value.as_bytes());
    }
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5c));
    outer.update(inner);
    format!("{:x}", outer.finalize())
}
fn digest(kind: &str, parts: &[(&str, &str)]) -> String {
    let mut canonical = String::new();
    for (k, v) in parts {
        canonical.push_str(k);
        canonical.push(':');
        canonical.push_str(&v.len().to_string());
        canonical.push(':');
        canonical.push_str(v);
        canonical.push('\n');
    }
    format!("{kind}:sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}
