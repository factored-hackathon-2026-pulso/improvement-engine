//! Deterministic, in-process sandbox for evaluation fixtures.
//!
//! This is not a banking service, a platform runtime, or an authorization
//! system. It only models state owned by a synthetic evaluation fixture. The
//! caller receives an opaque arm reference and every read is scoped to the
//! fixture tenant and namespace. A fresh arm clones the fixture's seed state;
//! no state can cross arms.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

/// Synthetic state that can be cloned into isolated evaluation arms.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxFixture {
    tenant_id: String,
    namespace: String,
    fixture_id: String,
    initial_state: BTreeMap<String, String>,
    allowed_actions: BTreeSet<String>,
    identity_policy: Option<SandboxIdentityPolicy>,
}

impl SandboxFixture {
    #[must_use]
    pub fn new(
        tenant_id: impl Into<String>,
        namespace: impl Into<String>,
        fixture_id: impl Into<String>,
        initial_state: BTreeMap<String, String>,
        allowed_actions: BTreeSet<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            namespace: namespace.into(),
            fixture_id: fixture_id.into(),
            initial_state,
            allowed_actions,
            identity_policy: None,
        }
    }

    /// Creates a fixture whose sensitive operations require current identity
    /// evidence. Evidence carries only identifiers and commitments; it never
    /// carries KBA answers or customer secrets.
    #[must_use]
    pub fn with_identity_policy(
        tenant_id: impl Into<String>,
        namespace: impl Into<String>,
        fixture_id: impl Into<String>,
        initial_state: BTreeMap<String, String>,
        allowed_actions: BTreeSet<String>,
        identity_policy: SandboxIdentityPolicy,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            namespace: namespace.into(),
            fixture_id: fixture_id.into(),
            initial_state,
            allowed_actions,
            identity_policy: Some(identity_policy),
        }
    }
}

/// Sealed identity rules for one sandbox fixture. This is a test/sandbox
/// policy, not a bank authentication implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxIdentityPolicy {
    case_id: String,
    channel: String,
    policy_digest: String,
    questions_digest: String,
    permitted_principals: BTreeSet<String>,
    valid_until: u64,
}

impl SandboxIdentityPolicy {
    #[must_use]
    pub fn new(
        case_id: impl Into<String>,
        channel: impl Into<String>,
        policy_digest: impl Into<String>,
        questions_digest: impl Into<String>,
        permitted_principals: BTreeSet<String>,
        valid_until: u64,
    ) -> Self {
        Self {
            case_id: case_id.into(),
            channel: channel.into(),
            policy_digest: policy_digest.into(),
            questions_digest: questions_digest.into(),
            permitted_principals,
            valid_until,
        }
    }
}

/// Attestation supplied to the fixture boundary after identity verification.
/// The attestation intentionally holds only commitments to the policy and
/// question set, never challenge answers or a model response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityEvidence {
    principal_id: String,
    tenant_id: String,
    case_id: String,
    channel: String,
    policy_digest: String,
    questions_digest: String,
    valid_until: u64,
}

impl IdentityEvidence {
    #[must_use]
    pub fn new(
        principal_id: impl Into<String>,
        tenant_id: impl Into<String>,
        case_id: impl Into<String>,
        channel: impl Into<String>,
        policy_digest: impl Into<String>,
        questions_digest: impl Into<String>,
        valid_until: u64,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            tenant_id: tenant_id.into(),
            case_id: case_id.into(),
            channel: channel.into(),
            policy_digest: policy_digest.into(),
            questions_digest: questions_digest.into(),
            valid_until,
        }
    }

    fn binding_digest(&self) -> String {
        let mut digest = Sha256::new();
        for part in [
            &self.principal_id,
            &self.tenant_id,
            &self.case_id,
            &self.channel,
            &self.policy_digest,
            &self.questions_digest,
        ] {
            digest.update(part.len().to_be_bytes());
            digest.update(part.as_bytes());
        }
        format!("sha256:{:x}", digest.finalize())
    }
}

/// Opaque handle for exactly one isolated fixture copy.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SandboxArmRef {
    evaluation_id: String,
    arm_id: String,
}

/// Scope asserted by the evaluator for an isolated fixture operation.
///
/// The deterministic adapter validates this assertion against the arm. It is
/// not a substitute for U36 identity and policy enforcement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxScope {
    tenant_id: String,
    namespace: String,
    observed_at: u64,
    identity: Option<IdentityEvidence>,
}

impl SandboxScope {
    #[must_use]
    pub fn new(tenant_id: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            namespace: namespace.into(),
            observed_at: 0,
            identity: None,
        }
    }

    #[must_use]
    pub fn with_identity(
        tenant_id: impl Into<String>,
        namespace: impl Into<String>,
        observed_at: u64,
        identity: IdentityEvidence,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            namespace: namespace.into(),
            observed_at,
            identity: Some(identity),
        }
    }
}

/// A versioned synthetic mutation. It cannot invoke an external tool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Action {
    action_id: String,
    expected_revision: u64,
    action: String,
    resource: String,
    replacement: String,
}

/// An action plus the authorization scope that the executor revalidates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequest {
    scope: SandboxScope,
    action: Action,
    observed_at: u64,
    identity: Option<IdentityEvidence>,
}

impl ActionRequest {
    #[must_use]
    pub fn new(tenant_id: impl Into<String>, namespace: impl Into<String>, action: Action) -> Self {
        Self {
            scope: SandboxScope::new(tenant_id, namespace),
            action,
            observed_at: 0,
            identity: None,
        }
    }

    #[must_use]
    pub fn with_identity(
        tenant_id: impl Into<String>,
        namespace: impl Into<String>,
        observed_at: u64,
        identity: IdentityEvidence,
        action: Action,
    ) -> Self {
        Self {
            scope: SandboxScope::new(tenant_id, namespace),
            action,
            observed_at,
            identity: Some(identity),
        }
    }
}

impl Action {
    #[must_use]
    pub fn replace(
        action_id: impl Into<String>,
        expected_revision: u64,
        action: impl Into<String>,
        resource: impl Into<String>,
        replacement: impl Into<String>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            expected_revision,
            action: action.into(),
            resource: resource.into(),
            replacement: replacement.into(),
        }
    }
}

/// Proof that a synthetic action was applied to one specific arm.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionReceipt {
    pub action_id: String,
    pub evaluation_id: String,
    pub arm_id: String,
    pub fixture_id: String,
    pub state_revision: u64,
}

/// Caller-scoped request for a value in an arm's synthetic state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadRequest {
    scope: SandboxScope,
    resource: String,
    observed_at: u64,
    identity: Option<IdentityEvidence>,
}

impl ReadRequest {
    #[must_use]
    pub fn new(
        tenant_id: impl Into<String>,
        namespace: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        Self {
            scope: SandboxScope::new(tenant_id, namespace),
            resource: resource.into(),
            observed_at: 0,
            identity: None,
        }
    }

    #[must_use]
    pub fn with_identity(
        tenant_id: impl Into<String>,
        namespace: impl Into<String>,
        observed_at: u64,
        identity: IdentityEvidence,
        resource: impl Into<String>,
    ) -> Self {
        Self {
            scope: SandboxScope::new(tenant_id, namespace),
            resource: resource.into(),
            observed_at,
            identity: Some(identity),
        }
    }
}

/// Authorized state observed after an action or reset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Readback {
    pub value: String,
    pub evaluation_id: String,
    pub arm_id: String,
    pub fixture_id: String,
    pub state_revision: u64,
}

/// Immutable proof that one authorized arm was restored to its sealed seed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResetReceipt {
    pub evaluation_id: String,
    pub arm_id: String,
    pub fixture_id: String,
    pub before_revision: u64,
    pub state_revision: u64,
    pub state_digest: String,
}

/// Errors are explicit so an evaluator cannot turn a missing effect into a
/// successful or neutral outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxError {
    ArmAlreadyExists,
    ArmUnknown,
    FixtureInvalid,
    ActionInvalid,
    FixtureMismatch,
    TenantDenied,
    NamespaceDenied,
    ResourceUnknown {
        resource: String,
    },
    ActionForbidden {
        action: String,
    },
    StaleAction {
        expected_revision: u64,
        actual_revision: u64,
    },
    ActionIdConflict {
        action_id: String,
    },
    IdentityEvidenceMissing,
    IdentityEvidenceMismatch,
    IdentityEvidenceExpired,
    IdentityRevoked,
}

/// Boundary used by an evaluator. Production execution must replace this
/// adapter; this port has no filesystem, network, provider or bank access.
pub trait SandboxPort {
    fn start_arm(
        &mut self,
        evaluation_id: &str,
        arm_id: &str,
        fixture: SandboxFixture,
    ) -> Result<SandboxArmRef, SandboxError>;

    fn execute(
        &mut self,
        arm: &SandboxArmRef,
        request: ActionRequest,
    ) -> Result<ActionReceipt, SandboxError>;

    fn read(&self, arm: &SandboxArmRef, request: ReadRequest) -> Result<Readback, SandboxError>;

    fn reset_arm(
        &mut self,
        arm: &SandboxArmRef,
        scope: SandboxScope,
    ) -> Result<ResetReceipt, SandboxError>;
}

#[derive(Clone, Debug)]
struct ArmState {
    fixture: SandboxFixture,
    state: BTreeMap<String, String>,
    revision: u64,
    actions: BTreeMap<String, (Action, ActionReceipt)>,
    revoked_identity_bindings: BTreeSet<String>,
}

/// A deterministic fixture simulator. It is deliberately in-memory: no
/// source-system data, bank credentials, or effects are accepted or retained.
#[derive(Default)]
pub struct StatefulSandbox {
    arms: BTreeMap<SandboxArmRef, ArmState>,
    evaluation_fixtures: BTreeMap<String, SandboxFixture>,
}

impl SandboxPort for StatefulSandbox {
    fn start_arm(
        &mut self,
        evaluation_id: &str,
        arm_id: &str,
        fixture: SandboxFixture,
    ) -> Result<SandboxArmRef, SandboxError> {
        if evaluation_id.is_empty() || arm_id.is_empty() || !fixture_is_valid(&fixture) {
            return Err(SandboxError::FixtureInvalid);
        }

        if let Some(existing_fixture) = self.evaluation_fixtures.get(evaluation_id) {
            if existing_fixture != &fixture {
                return Err(SandboxError::FixtureMismatch);
            }
        }

        let arm = SandboxArmRef {
            evaluation_id: evaluation_id.to_owned(),
            arm_id: arm_id.to_owned(),
        };
        if self.arms.contains_key(&arm) {
            return Err(SandboxError::ArmAlreadyExists);
        }

        self.arms.insert(
            arm.clone(),
            ArmState {
                state: fixture.initial_state.clone(),
                fixture: fixture.clone(),
                revision: 0,
                actions: BTreeMap::new(),
                revoked_identity_bindings: BTreeSet::new(),
            },
        );
        self.evaluation_fixtures
            .entry(evaluation_id.to_owned())
            .or_insert(fixture);
        Ok(arm)
    }

    fn execute(
        &mut self,
        arm: &SandboxArmRef,
        request: ActionRequest,
    ) -> Result<ActionReceipt, SandboxError> {
        let state = self.arms.get_mut(arm).ok_or(SandboxError::ArmUnknown)?;
        validate_scope(&state.fixture, &request.scope)?;
        validate_identity(
            &state.fixture,
            request.identity.as_ref(),
            request.observed_at,
            &state.revoked_identity_bindings,
        )?;
        let action = request.action;

        if action.action_id.is_empty() || action.action.is_empty() || action.resource.is_empty() {
            return Err(SandboxError::ActionInvalid);
        }

        if let Some((prior_action, prior_receipt)) = state.actions.get(&action.action_id) {
            return if prior_action == &action {
                Ok(prior_receipt.clone())
            } else {
                Err(SandboxError::ActionIdConflict {
                    action_id: action.action_id,
                })
            };
        }
        if action.expected_revision != state.revision {
            return Err(SandboxError::StaleAction {
                expected_revision: action.expected_revision,
                actual_revision: state.revision,
            });
        }
        if !state.fixture.allowed_actions.contains(&action.action) {
            return Err(SandboxError::ActionForbidden {
                action: action.action,
            });
        }
        if !state.state.contains_key(&action.resource) {
            return Err(SandboxError::ResourceUnknown {
                resource: action.resource,
            });
        }

        state
            .state
            .insert(action.resource.clone(), action.replacement.clone());
        state.revision += 1;
        let receipt = ActionReceipt {
            action_id: action.action_id.clone(),
            evaluation_id: arm.evaluation_id.clone(),
            arm_id: arm.arm_id.clone(),
            fixture_id: state.fixture.fixture_id.clone(),
            state_revision: state.revision,
        };
        state
            .actions
            .insert(action.action_id.clone(), (action, receipt.clone()));
        Ok(receipt)
    }

    fn read(&self, arm: &SandboxArmRef, request: ReadRequest) -> Result<Readback, SandboxError> {
        let state = self.arms.get(arm).ok_or(SandboxError::ArmUnknown)?;
        validate_scope(&state.fixture, &request.scope)?;
        validate_identity(
            &state.fixture,
            request.identity.as_ref(),
            request.observed_at,
            &state.revoked_identity_bindings,
        )?;
        let value =
            state
                .state
                .get(&request.resource)
                .cloned()
                .ok_or(SandboxError::ResourceUnknown {
                    resource: request.resource,
                })?;
        Ok(Readback {
            value,
            evaluation_id: arm.evaluation_id.clone(),
            arm_id: arm.arm_id.clone(),
            fixture_id: state.fixture.fixture_id.clone(),
            state_revision: state.revision,
        })
    }

    fn reset_arm(
        &mut self,
        arm: &SandboxArmRef,
        scope: SandboxScope,
    ) -> Result<ResetReceipt, SandboxError> {
        let state = self.arms.get_mut(arm).ok_or(SandboxError::ArmUnknown)?;
        validate_scope(&state.fixture, &scope)?;
        validate_identity(
            &state.fixture,
            scope.identity.as_ref(),
            scope.observed_at,
            &state.revoked_identity_bindings,
        )?;
        let before_revision = state.revision;
        state.state = state.fixture.initial_state.clone();
        state.revision += 1;
        state.actions.clear();
        Ok(ResetReceipt {
            evaluation_id: arm.evaluation_id.clone(),
            arm_id: arm.arm_id.clone(),
            fixture_id: state.fixture.fixture_id.clone(),
            before_revision,
            state_revision: state.revision,
            state_digest: state_digest(&state.state),
        })
    }
}

impl StatefulSandbox {
    /// Revokes matching identity evidence for one arm. The effect is local to
    /// that arm so candidate and baseline never share mutable authorization.
    pub fn revoke_identity(
        &mut self,
        arm: &SandboxArmRef,
        scope: SandboxScope,
        identity: IdentityEvidence,
    ) -> Result<(), SandboxError> {
        let state = self.arms.get_mut(arm).ok_or(SandboxError::ArmUnknown)?;
        validate_scope(&state.fixture, &scope)?;
        let Some(policy) = &state.fixture.identity_policy else {
            return Err(SandboxError::IdentityEvidenceMismatch);
        };
        if !identity_matches(&state.fixture.tenant_id, policy, &identity) {
            return Err(SandboxError::IdentityEvidenceMismatch);
        }
        state
            .revoked_identity_bindings
            .insert(identity.binding_digest());
        Ok(())
    }
}

fn fixture_is_valid(fixture: &SandboxFixture) -> bool {
    !fixture.tenant_id.is_empty()
        && !fixture.namespace.is_empty()
        && !fixture.fixture_id.is_empty()
        && fixture
            .initial_state
            .keys()
            .all(|resource| !resource.is_empty())
        && fixture
            .allowed_actions
            .iter()
            .all(|action| !action.is_empty())
        && fixture
            .identity_policy
            .as_ref()
            .is_none_or(identity_policy_is_valid)
}

fn identity_policy_is_valid(policy: &SandboxIdentityPolicy) -> bool {
    !policy.case_id.is_empty()
        && !policy.channel.is_empty()
        && !policy.policy_digest.is_empty()
        && !policy.questions_digest.is_empty()
        && !policy.permitted_principals.is_empty()
        && policy.valid_until > 0
        && policy
            .permitted_principals
            .iter()
            .all(|principal| !principal.is_empty())
}

fn validate_identity(
    fixture: &SandboxFixture,
    identity: Option<&IdentityEvidence>,
    observed_at: u64,
    revoked_identity_bindings: &BTreeSet<String>,
) -> Result<(), SandboxError> {
    let Some(policy) = &fixture.identity_policy else {
        return Ok(());
    };
    let identity = identity.ok_or(SandboxError::IdentityEvidenceMissing)?;
    if identity.valid_until <= observed_at || policy.valid_until <= observed_at {
        return Err(SandboxError::IdentityEvidenceExpired);
    }
    if !identity_matches(&fixture.tenant_id, policy, identity) {
        return Err(SandboxError::IdentityEvidenceMismatch);
    }
    if revoked_identity_bindings.contains(&identity.binding_digest()) {
        return Err(SandboxError::IdentityRevoked);
    }
    Ok(())
}

fn identity_matches(
    tenant_id: &str,
    policy: &SandboxIdentityPolicy,
    identity: &IdentityEvidence,
) -> bool {
    identity.tenant_id == tenant_id
        && identity.case_id == policy.case_id
        && identity.channel == policy.channel
        && identity.policy_digest == policy.policy_digest
        && identity.questions_digest == policy.questions_digest
        && policy.permitted_principals.contains(&identity.principal_id)
}

fn validate_scope(fixture: &SandboxFixture, scope: &SandboxScope) -> Result<(), SandboxError> {
    if scope.tenant_id != fixture.tenant_id {
        return Err(SandboxError::TenantDenied);
    }
    if scope.namespace != fixture.namespace {
        return Err(SandboxError::NamespaceDenied);
    }
    Ok(())
}

fn state_digest(state: &BTreeMap<String, String>) -> String {
    let mut digest = Sha256::new();
    for (resource, value) in state {
        digest.update(resource.len().to_be_bytes());
        digest.update(resource.as_bytes());
        digest.update(value.len().to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}
