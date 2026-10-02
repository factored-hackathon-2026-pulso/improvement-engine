//! U17 compiles one explicitly authorized change into immutable Core drafts.
//!
//! This boundary deliberately does not write a registry, execute Agent Core,
//! evaluate a candidate, or release anything. Those effects belong to U18+
//! after their own gates.

use crate::ArtifactReference;
use crate::core_task::CoreTaskScope;
use crate::final_eligibility::{FinalEligibility, FinalEligibilityDecision};
use crate::workflow_bridge::{LinkGrade, WorkflowBridgeContract};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreEntityKind {
    Flow,
    Agent,
    DecisionModel,
    Prompt,
    Template,
    Tool,
}

impl CoreEntityKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flow => "flow",
            Self::Agent => "agent",
            Self::DecisionModel => "decision_model",
            Self::Prompt => "prompt",
            Self::Template => "template",
            Self::Tool => "tool",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeOperationKind {
    Add,
    Replace,
    Disable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangeOperation {
    operation: ChangeOperationKind,
    target_kind: CoreEntityKind,
    content: Value,
    precondition_digest: String,
}

impl ChangeOperation {
    #[must_use]
    pub fn new(
        operation: ChangeOperationKind,
        target_kind: CoreEntityKind,
        content: Value,
        precondition_digest: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            target_kind,
            content,
            precondition_digest: precondition_digest.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UntrustedChangeSpec {
    claimed_scope: CoreTaskScope,
    claimed_source_snapshot: ArtifactReference,
    candidate_route: String,
    expected_mechanism: String,
    operations: Vec<ChangeOperation>,
}

impl UntrustedChangeSpec {
    pub fn new(
        claimed_scope: CoreTaskScope,
        claimed_source_snapshot: ArtifactReference,
        candidate_route: impl Into<String>,
        expected_mechanism: impl Into<String>,
        operations: Vec<ChangeOperation>,
    ) -> Result<Self, CompilerError> {
        let value = Self {
            claimed_scope,
            claimed_source_snapshot,
            candidate_route: candidate_route.into(),
            expected_mechanism: expected_mechanism.into(),
            operations,
        };
        if value.candidate_route.is_empty()
            || value.expected_mechanism.is_empty()
            || value.operations.is_empty()
        {
            return Err(CompilerError::InvalidChangeSpec);
        }
        Ok(value)
    }
}

/// The atomic registry condition that U18 must evaluate at the same write
/// boundary as the enclosing [`CompilationAuthorization`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutablePredicate {
    /// An `add` may proceed only if the exact Core entity identity has no head.
    EntityAbsent {
        kind: CoreEntityKind,
        entity_id: String,
    },
    /// A future replacement must compare the exact known entity revision/body.
    RevisionEquals {
        kind: CoreEntityKind,
        entity_id: String,
        entity_version: String,
        content_digest: String,
    },
}

impl ExecutablePredicate {
    fn canonical_form(&self) -> String {
        match self {
            Self::EntityAbsent { kind, entity_id } => {
                format!("entity_absent|{}|{entity_id}", kind.as_str())
            }
            Self::RevisionEquals {
                kind,
                entity_id,
                entity_version,
                content_digest,
            } => format!(
                "revision_equals|{}|{entity_id}|{entity_version}|{content_digest}",
                kind.as_str()
            ),
        }
    }
}

/// A typed executable fence plus its canonical digest. The digest is evidence
/// of the predicate bytes; it is never itself the condition U18 will execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutablePrecondition {
    predicate: ExecutablePredicate,
    commitment: String,
}

impl ExecutablePrecondition {
    fn entity_absent(kind: CoreEntityKind, entity_id: String) -> Self {
        let predicate = ExecutablePredicate::EntityAbsent { kind, entity_id };
        let commitment = digest(&[&predicate.canonical_form()]);
        Self {
            predicate,
            commitment,
        }
    }

    #[must_use]
    pub fn predicate(&self) -> &ExecutablePredicate {
        &self.predicate
    }
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }
}

/// Sealed authorization retained by U18 so a later write boundary can
/// atomically revalidate the exact tenant/scope/snapshot and draft identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompilationAuthorization {
    scope: CoreTaskScope,
    source_snapshot: ArtifactReference,
    workflow_bridge_commitment: String,
    evaluation_plan_commitment: String,
    candidate_route: String,
    mechanism: String,
    operation: ChangeOperationKind,
    kind: CoreEntityKind,
    entity_id: String,
    entity_version: String,
    content_digest: String,
    precondition: ExecutablePrecondition,
    commitment: String,
}

impl CompilationAuthorization {
    #[must_use]
    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }
    #[must_use]
    pub fn source_snapshot(&self) -> &ArtifactReference {
        &self.source_snapshot
    }
    #[must_use]
    pub fn workflow_bridge_commitment(&self) -> &str {
        &self.workflow_bridge_commitment
    }
    #[must_use]
    pub fn evaluation_plan_commitment(&self) -> &str {
        &self.evaluation_plan_commitment
    }
    #[must_use]
    pub fn candidate_route(&self) -> &str {
        &self.candidate_route
    }
    #[must_use]
    pub fn mechanism(&self) -> &str {
        &self.mechanism
    }
    #[must_use]
    pub fn operation(&self) -> ChangeOperationKind {
        self.operation
    }
    #[must_use]
    pub fn kind(&self) -> CoreEntityKind {
        self.kind
    }
    #[must_use]
    pub fn entity_id(&self) -> &str {
        &self.entity_id
    }
    #[must_use]
    pub fn entity_version(&self) -> &str {
        &self.entity_version
    }
    #[must_use]
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }
    #[must_use]
    pub fn precondition(&self) -> &ExecutablePrecondition {
        &self.precondition
    }
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }
}

/// Opaque input emitted only by trusted in-crate composition after it has
/// checked U35 and U16. Public callers may author an [`UntrustedChangeSpec`]
/// but cannot construct this capability.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthorizedChangeSpec {
    untrusted: UntrustedChangeSpec,
    authorization: CompilationAuthorization,
}

/// ```compile_fail
/// use improvement_engine_core::change_compiler::AuthorizedChangeSpec;
/// let _ = AuthorizedChangeSpec::new();
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::change_compiler::AuthorizedChangeSpec;
/// let _ = AuthorizedChangeSpec { untrusted: todo!(), authorization: todo!() };
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::change_compiler::TrustedChangeAuthorizer;
/// let _ = TrustedChangeAuthorizer;
/// ```
///
/// Authoring a change does not manufacture the authority required to compile
/// it. Only trusted in-crate composition can emit an `AuthorizedChangeSpec`.
const _NO_PUBLIC_AUTHORIZED_CHANGE_CONSTRUCTOR: () = ();

/// Trusted composition boundary, intentionally crate-private until a
/// separately governed policy/authority adapter is available.
#[allow(dead_code)] // Invoked by trusted service composition after U18 wiring.
pub(crate) struct TrustedChangeAuthorizer;

impl TrustedChangeAuthorizer {
    #[allow(dead_code)] // Exercised in crate tests until the U18 composition exists.
    pub(crate) fn authorize(
        readiness: &FinalEligibilityDecision,
        bridge: &WorkflowBridgeContract,
        untrusted: UntrustedChangeSpec,
    ) -> Result<AuthorizedChangeSpec, CompilerError> {
        let FinalEligibilityDecision::Eligible(readiness) = readiness else {
            return Err(CompilerError::FinalEligibilityRequired);
        };
        if bridge.link_grade() != LinkGrade::MechanismProxy
            || !bridge.alternatives().includes_candidate_route()
        {
            return Err(CompilerError::BridgeNotAuthorizable);
        }
        if readiness.bridge_commitment() != bridge.commitment()
            || untrusted.claimed_scope != *bridge.scope()
            || untrusted.claimed_source_snapshot != *bridge.source_snapshot_ref()
        {
            return Err(CompilerError::AuthorizationBindingMismatch);
        }
        if untrusted.candidate_route != bridge.input().candidate_route()
            || untrusted.expected_mechanism != bridge.input().mechanism()
        {
            return Err(CompilerError::RouteOrMechanismMismatch);
        }
        if untrusted.operations.len() != 1 {
            return Err(CompilerError::UnsupportedOperation);
        }
        let operation = &untrusted.operations[0];
        if operation.operation != ChangeOperationKind::Add {
            return Err(CompilerError::UnsupportedOperation);
        }
        if operation.target_kind != CoreEntityKind::Flow {
            return Err(CompilerError::UnsupportedEntityKind);
        }
        let (entity_id, entity_version) =
            entity_identity(&operation.content).ok_or(CompilerError::InvalidEntityIdentity)?;
        if !is_minimal_core_flow(&operation.content) {
            return Err(CompilerError::InvalidEntityIdentity);
        }
        let expected = expected_precondition(operation.target_kind, &entity_id);
        if operation.precondition_digest != expected.commitment() {
            return Err(CompilerError::PreconditionMismatch);
        }
        let content_digest = digest(&[&canonical_json(&operation.content)]);
        let authorization = CompilationAuthorization::new(
            bridge,
            readiness,
            operation,
            entity_id,
            entity_version,
            content_digest,
            expected,
        );
        Ok(AuthorizedChangeSpec {
            untrusted,
            authorization,
        })
    }
}

impl CompilationAuthorization {
    #[allow(dead_code)] // Reached through the deferred trusted composition.
    fn new(
        bridge: &WorkflowBridgeContract,
        readiness: &FinalEligibility,
        operation: &ChangeOperation,
        entity_id: String,
        entity_version: String,
        content_digest: String,
        precondition: ExecutablePrecondition,
    ) -> Self {
        let scope = bridge.scope().clone();
        let source_snapshot = bridge.source_snapshot_ref().clone();
        let workflow_bridge_commitment = bridge.commitment().to_owned();
        let evaluation_plan_commitment = readiness.plan_commitment().to_owned();
        let candidate_route = bridge.input().candidate_route().to_owned();
        let mechanism = bridge.input().mechanism().to_owned();
        let commitment = compilation_authorization_commitment(
            &scope,
            &source_snapshot,
            &workflow_bridge_commitment,
            &evaluation_plan_commitment,
            &candidate_route,
            &mechanism,
            operation.operation,
            operation.target_kind,
            &entity_id,
            &entity_version,
            &content_digest,
            &precondition,
        );
        Self {
            scope,
            source_snapshot,
            workflow_bridge_commitment,
            evaluation_plan_commitment,
            candidate_route,
            mechanism,
            operation: operation.operation,
            kind: operation.target_kind,
            entity_id,
            entity_version,
            content_digest,
            precondition,
            commitment,
        }
    }
}

/// Immutable Core-wire candidate draft. It has no registry identity or release
/// status, so possessing it cannot cause an external mutation.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityDraft {
    kind: CoreEntityKind,
    id: String,
    version: String,
    content: Value,
    digest: String,
}

impl EntityDraft {
    #[must_use]
    pub fn kind(&self) -> CoreEntityKind {
        self.kind
    }
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
    #[must_use]
    pub fn content(&self) -> &Value {
        &self.content
    }
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledChange {
    drafts: Vec<EntityDraft>,
    authorization: CompilationAuthorization,
    commitment: String,
}

/// Verified, immutable material that may cross only into U18's crate-private
/// registry write boundary. It is reconstructed from the compiled draft and
/// sealed authorization immediately before the writer evaluates its predicate.
#[allow(dead_code)] // Consumed by the crate-private U18 registry writer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegistryWriteMaterial {
    pub(crate) scope: CoreTaskScope,
    pub(crate) source_snapshot: ArtifactReference,
    pub(crate) authorization_commitment: String,
    pub(crate) compiled_commitment: String,
    pub(crate) predicate: ExecutablePredicate,
    pub(crate) kind: CoreEntityKind,
    pub(crate) entity_id: String,
    pub(crate) entity_version: String,
    pub(crate) content: Value,
    pub(crate) content_digest: String,
    pub(crate) draft_digest: String,
}

impl CompiledChange {
    #[must_use]
    pub fn drafts(&self) -> &[EntityDraft] {
        &self.drafts
    }
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }
    #[must_use]
    pub fn authorization(&self) -> &CompilationAuthorization {
        &self.authorization
    }
    #[must_use]
    pub fn authorizes_registry_write(&self) -> bool {
        false
    }
    #[must_use]
    pub fn authorizes_execution_or_release(&self) -> bool {
        false
    }

    #[allow(dead_code)] // Consumed by the crate-private U18 registry writer.
    pub(crate) fn registry_write_material(&self) -> Result<RegistryWriteMaterial, CompilerError> {
        if self.drafts.len() != 1 || self.authorization.operation != ChangeOperationKind::Add {
            return Err(CompilerError::AuthorizationBindingMismatch);
        }
        let draft = &self.drafts[0];
        let authorization = &self.authorization;
        let expected_content_digest = digest(&[&canonical_json(&draft.content)]);
        let expected_draft_digest = digest(&[
            draft.kind.as_str(),
            &draft.id,
            &draft.version,
            authorization.precondition.commitment(),
            &canonical_json(&draft.content),
        ]);
        let expected_compiled_commitment = digest(&[authorization.commitment(), &draft.digest]);
        let expected_authorization_commitment = compilation_authorization_commitment(
            &authorization.scope,
            &authorization.source_snapshot,
            &authorization.workflow_bridge_commitment,
            &authorization.evaluation_plan_commitment,
            &authorization.candidate_route,
            &authorization.mechanism,
            authorization.operation,
            authorization.kind,
            &authorization.entity_id,
            &authorization.entity_version,
            &authorization.content_digest,
            &authorization.precondition,
        );
        if draft.kind != authorization.kind
            || draft.id != authorization.entity_id
            || draft.version != authorization.entity_version
            || draft.digest != expected_draft_digest
            || authorization.content_digest != expected_content_digest
            || authorization.commitment != expected_authorization_commitment
            || self.commitment != expected_compiled_commitment
            || !matches!(
                &authorization.precondition.predicate,
                ExecutablePredicate::EntityAbsent { kind, entity_id }
                    if *kind == draft.kind && entity_id == &draft.id
            )
            || !is_minimal_core_flow(&draft.content)
            || authorization.scope.tenant_id() != authorization.source_snapshot.tenant_id
        {
            return Err(CompilerError::AuthorizationBindingMismatch);
        }
        Ok(RegistryWriteMaterial {
            scope: authorization.scope.clone(),
            source_snapshot: authorization.source_snapshot.clone(),
            authorization_commitment: authorization.commitment.clone(),
            compiled_commitment: self.commitment.clone(),
            predicate: authorization.precondition.predicate.clone(),
            kind: draft.kind,
            entity_id: draft.id.clone(),
            entity_version: draft.version.clone(),
            content: draft.content.clone(),
            content_digest: authorization.content_digest.clone(),
            draft_digest: draft.digest.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompilerError {
    FinalEligibilityRequired,
    ReadinessCommitmentMismatch,
    AuthorizationBindingMismatch,
    BridgeNotAuthorizable,
    RouteOrMechanismMismatch,
    PreconditionMismatch,
    InvalidChangeSpec,
    UnsupportedOperation,
    UnsupportedEntityKind,
    InvalidEntityIdentity,
    DuplicateEntity,
}

pub struct ChangeCompiler;

impl ChangeCompiler {
    pub fn compile(spec: AuthorizedChangeSpec) -> Result<CompiledChange, CompilerError> {
        let operation = &spec.untrusted.operations[0];
        let (id, version) =
            entity_identity(&operation.content).ok_or(CompilerError::InvalidEntityIdentity)?;
        if operation.operation != spec.authorization.operation
            || operation.target_kind != spec.authorization.kind
            || id != spec.authorization.entity_id
            || version != spec.authorization.entity_version
            || operation.precondition_digest != spec.authorization.precondition.commitment
            || digest(&[&canonical_json(&operation.content)]) != spec.authorization.content_digest
            || !is_minimal_core_flow(&operation.content)
        {
            return Err(CompilerError::AuthorizationBindingMismatch);
        }
        let draft_digest = digest(&[
            operation.target_kind.as_str(),
            &id,
            &version,
            operation.precondition_digest.as_str(),
            &canonical_json(&operation.content),
        ]);
        let draft = EntityDraft {
            kind: operation.target_kind,
            id,
            version,
            content: operation.content.clone(),
            digest: draft_digest,
        };
        let commitment = digest(&[spec.authorization.commitment(), draft.digest()]);
        Ok(CompiledChange {
            drafts: vec![draft],
            authorization: spec.authorization,
            commitment,
        })
    }
}

/// Trusted in-crate fixture for U18's writer regressions. It still reaches the
/// real compiler from an opaque authorized capability; it is unavailable from
/// non-test consumers and must never become a service composition path.
#[cfg(test)]
pub(crate) fn compiled_for_governed_registry_test(tenant_id: &str) -> CompiledChange {
    let scope =
        CoreTaskScope::new(tenant_id, "job_a", "grant_a", "authority_a").expect("fixed scope");
    let source_snapshot = ArtifactReference {
        tenant_id: tenant_id.to_owned(),
        id: "018f0f4e-7bbd-7000-8000-000000000600".to_owned(),
        revision: 1,
        digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_owned(),
    };
    let content = serde_json::json!({
        "id": "payment_status_resolution",
        "version": "1.0.0",
        "priority": 1,
        "nodes": [{"id":"complete","type":"end","config":{"outcome":"completed"}}]
    });
    let precondition = expected_precondition(CoreEntityKind::Flow, "payment_status_resolution");
    let content_digest = digest(&[&canonical_json(&content)]);
    let candidate_route = "flow/payment-status".to_owned();
    let mechanism = "transaction_status_lookup".to_owned();
    let commitment = compilation_authorization_commitment(
        &scope,
        &source_snapshot,
        "sha256:bridge",
        "sha256:plan",
        &candidate_route,
        &mechanism,
        ChangeOperationKind::Add,
        CoreEntityKind::Flow,
        "payment_status_resolution",
        "1.0.0",
        &content_digest,
        &precondition,
    );
    let authorization = CompilationAuthorization {
        scope: scope.clone(),
        source_snapshot: source_snapshot.clone(),
        workflow_bridge_commitment: "sha256:bridge".to_owned(),
        evaluation_plan_commitment: "sha256:plan".to_owned(),
        candidate_route: candidate_route.clone(),
        mechanism: mechanism.clone(),
        operation: ChangeOperationKind::Add,
        kind: CoreEntityKind::Flow,
        entity_id: "payment_status_resolution".to_owned(),
        entity_version: "1.0.0".to_owned(),
        content_digest,
        precondition: precondition.clone(),
        commitment,
    };
    ChangeCompiler::compile(AuthorizedChangeSpec {
        untrusted: UntrustedChangeSpec::new(
            scope,
            source_snapshot,
            candidate_route,
            mechanism,
            vec![ChangeOperation::new(
                ChangeOperationKind::Add,
                CoreEntityKind::Flow,
                content,
                precondition.commitment(),
            )],
        )
        .expect("fixed untrusted spec"),
        authorization,
    })
    .expect("fixed authorized fixture compiles")
}

/// Deliberately corrupts sealed data only for U18's anti-tamper regression.
/// Production consumers cannot access either the compiled fields or this seam.
#[cfg(test)]
pub(crate) fn corrupt_registry_snapshot_tenant_for_test(
    mut compiled: CompiledChange,
    tenant_id: &str,
) -> CompiledChange {
    compiled.authorization.source_snapshot.tenant_id = tenant_id.to_owned();
    compiled
}

#[allow(dead_code)] // Reached through the deferred trusted composition.
fn expected_precondition(kind: CoreEntityKind, entity_id: &str) -> ExecutablePrecondition {
    ExecutablePrecondition::entity_absent(kind, entity_id.to_owned())
}

/// Canonical authorization identity. U17 emits this at trusted composition;
/// U18 recomputes it before its conditional write so a coherent-looking but
/// altered authorization object cannot be accepted as authority.
#[allow(clippy::too_many_arguments)] // Exact sealed fields are intentionally explicit.
fn compilation_authorization_commitment(
    scope: &CoreTaskScope,
    source_snapshot: &ArtifactReference,
    workflow_bridge_commitment: &str,
    evaluation_plan_commitment: &str,
    candidate_route: &str,
    mechanism: &str,
    operation: ChangeOperationKind,
    kind: CoreEntityKind,
    entity_id: &str,
    entity_version: &str,
    content_digest: &str,
    precondition: &ExecutablePrecondition,
) -> String {
    digest(&[
        scope.tenant_id(),
        scope.job_id(),
        scope.grant_id(),
        scope.authority_ref(),
        &source_snapshot.tenant_id,
        &source_snapshot.id,
        &source_snapshot.revision.to_string(),
        &source_snapshot.digest,
        workflow_bridge_commitment,
        evaluation_plan_commitment,
        candidate_route,
        mechanism,
        kind.as_str(),
        operation_name(operation),
        entity_id,
        entity_version,
        content_digest,
        precondition.commitment(),
    ])
}

#[allow(dead_code)] // Used by the trusted authorizer.
fn operation_name(operation: ChangeOperationKind) -> &'static str {
    match operation {
        ChangeOperationKind::Add => "add",
        ChangeOperationKind::Replace => "replace",
        ChangeOperationKind::Disable => "disable",
    }
}

fn entity_identity(content: &Value) -> Option<(String, String)> {
    let id = content.get("id")?.as_str()?.to_owned();
    let version = content.get("version")?.as_str()?.to_owned();
    (is_identifier(&id) && is_semver(&version)).then_some((id, version))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(string) => serde_json::to_string(string).expect("JSON string serializes"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, nested)| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("JSON object key serializes"),
                        canonical_json(nested)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn digest(pieces: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for piece in pieces {
        hasher.update(piece.len().to_be_bytes());
        hasher.update(piece.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

fn is_semver(value: &str) -> bool {
    let mut parts = value.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(major), Some(minor), Some(patch), None)
            if [major, minor, patch].iter().all(is_canonical_semver_part)
    )
}

fn is_canonical_semver_part(part: &&str) -> bool {
    !part.is_empty()
        && part.len() <= 9
        && part.bytes().all(|byte| byte.is_ascii_digit())
        && (part == &"0" || !part.starts_with('0'))
        && part.parse::<u32>().is_ok()
}

/// The first U17 vertical supports only the smallest executable Core Flow
/// shape. Broader entity/schema coverage must land with its own compatibility
/// slice instead of being accepted as opaque JSON.
fn is_minimal_core_flow(content: &Value) -> bool {
    let Some(flow) = content.as_object() else {
        return false;
    };
    if flow.len() != 4
        || !["id", "version", "priority", "nodes"]
            .iter()
            .all(|key| flow.contains_key(*key))
    {
        return false;
    }
    let Some(priority) = content.get("priority").and_then(Value::as_i64) else {
        return false;
    };
    let Some(nodes) = content.get("nodes").and_then(Value::as_array) else {
        return false;
    };
    priority >= 0 && !nodes.is_empty() && {
        let mut ids = std::collections::BTreeSet::new();
        nodes.iter().all(|node| {
            let Some(node_object) = node.as_object() else {
                return false;
            };
            if node_object.len() != 3
                || !["id", "type", "config"]
                    .iter()
                    .all(|key| node_object.contains_key(*key))
            {
                return false;
            }
            let Some(node_id) = node.get("id").and_then(Value::as_str) else {
                return false;
            };
            ids.insert(node_id.to_owned())
                && node
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(is_identifier)
                && node.get("type").and_then(Value::as_str) == Some("end")
                && node
                    .get("config")
                    .and_then(Value::as_object)
                    .filter(|config| config.len() == 1 && config.contains_key("outcome"))
                    .and_then(|config| config.get("outcome"))
                    .and_then(Value::as_str)
                    .is_some_and(is_core_outcome)
        })
    }
}

fn is_core_outcome(value: &str) -> bool {
    matches!(
        value,
        "resolved"
            | "abstained"
            | "cancelled"
            | "clarify_exhausted"
            | "completed"
            | "failed"
            | "abandoned"
            | "escalated"
            | "transferred"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation_plan::{
        EvaluationArtifactGrant, EvaluationArtifactRef, EvaluationInputs,
        InMemoryEvaluationArtifactAuthority, TrustedEvaluationComposer,
    };
    use crate::final_eligibility::FinalEligibilityGate;
    use crate::independent_verifier::VerificationStatus;
    use crate::workflow_bridge::report_and_bridge_for_final_eligibility_test;
    use crate::{
        ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository,
        InMemoryArtifactRepository,
    };
    use serde_json::json;

    fn append(
        repo: &mut InMemoryArtifactRepository,
        id: &str,
        kind: ArtifactKind,
        body: Value,
        snapshot: Option<ArtifactReference>,
    ) -> ArtifactReference {
        repo.append(
            None,
            ArtifactDraft::new("tenant_a", id, 1, kind, body, snapshot),
        )
        .expect("fixed test artifact appends")
        .reference()
    }

    fn evaluation_input(
        repo: &mut InMemoryArtifactRepository,
        id: &str,
        role: &str,
        partition: &str,
        snapshot: ArtifactReference,
    ) -> ArtifactReference {
        append(
            repo,
            id,
            ArtifactKind::ScenarioSet,
            json!({"evaluation_contract": {
                "artifact_type": role,
                "partition": partition,
                "target_outcome": "reduce_repeat_payment_contacts",
                "unit_of_analysis": "customer_episode",
                "oracle_measure": "scenario_oracle/payment_status_resolution_v1",
                "scope": {"tenant_id":"tenant_a","job_id":"job_a","grant_id":"grant_a","authority_ref":"authority_a"}
            }}),
            Some(snapshot),
        )
    }

    fn eligible_readiness() -> (FinalEligibilityDecision, WorkflowBridgeContract) {
        let mut repo = InMemoryArtifactRepository::default();
        let snapshot = append(
            &mut repo,
            "018f0f4e-7bbd-7000-8000-000000000600",
            ArtifactKind::SourceSnapshot,
            crate::evaluation_plan::tests::source_snapshot_payload("tenant_a"),
            None,
        );
        let (report, bridge) = report_and_bridge_for_final_eligibility_test(
            snapshot.clone(),
            VerificationStatus::Supported,
        );
        let baseline = evaluation_input(
            &mut repo,
            "018f0f4e-7bbd-7000-8000-000000000601",
            "baseline",
            "shared",
            snapshot.clone(),
        );
        let oracle = evaluation_input(
            &mut repo,
            "018f0f4e-7bbd-7000-8000-000000000602",
            "oracle",
            "shared",
            snapshot.clone(),
        );
        let development = evaluation_input(
            &mut repo,
            "018f0f4e-7bbd-7000-8000-000000000603",
            "development_suite",
            "development",
            snapshot.clone(),
        );
        let final_suite = evaluation_input(
            &mut repo,
            "018f0f4e-7bbd-7000-8000-000000000604",
            "final_suite",
            "final",
            snapshot,
        );
        let inputs = EvaluationInputs::new(
            EvaluationArtifactRef::baseline(baseline.clone()),
            EvaluationArtifactRef::oracle(oracle.clone()),
            EvaluationArtifactRef::development_suite(development.clone()),
            EvaluationArtifactRef::final_suite(final_suite.clone()),
        );
        let mut authority = InMemoryEvaluationArtifactAuthority::default();
        for reference in [&baseline, &oracle, &development, &final_suite] {
            authority.issue(EvaluationArtifactGrant::for_scope(
                bridge.scope().clone(),
                reference.clone(),
            ));
        }
        let plan = TrustedEvaluationComposer::from_policy(authority)
            .seal_from_bridge(&bridge, inputs, &mut repo)
            .expect("matching attested plan seals");
        (
            FinalEligibilityGate::decide(&report, &bridge, &plan),
            bridge,
        )
    }

    fn spec(bridge: &WorkflowBridgeContract) -> UntrustedChangeSpec {
        let content = json!({"id":"payment_status_resolution","version":"1.0.0","priority":1,"nodes":[{"id":"complete","type":"end","config":{"outcome":"completed"}}]});
        let precondition = expected_precondition(CoreEntityKind::Flow, "payment_status_resolution");
        UntrustedChangeSpec::new(
            bridge.scope().clone(),
            bridge.source_snapshot_ref().clone(),
            bridge.input().candidate_route(),
            bridge.input().mechanism(),
            vec![ChangeOperation::new(
                ChangeOperationKind::Add,
                CoreEntityKind::Flow,
                content,
                precondition.commitment(),
            )],
        )
        .expect("fixed spec is valid")
    }

    #[test]
    fn eligible_readiness_compiles_one_immutable_draft_without_registry_or_release_effect() {
        let (readiness, bridge) = eligible_readiness();
        let authorized = TrustedChangeAuthorizer::authorize(&readiness, &bridge, spec(&bridge))
            .expect("trusted composition authorizes matching U35/U16/spec");
        let compiled = ChangeCompiler::compile(authorized).expect("eligible readiness compiles");
        let expected_plan_commitment = match &readiness {
            FinalEligibilityDecision::Eligible(value) => value.plan_commitment(),
            FinalEligibilityDecision::Ineligible(_) => panic!("fixture must be eligible"),
        };
        assert_eq!(compiled.drafts().len(), 1);
        assert_eq!(compiled.drafts()[0].kind(), CoreEntityKind::Flow);
        assert_eq!(compiled.drafts()[0].id(), "payment_status_resolution");
        assert!(!compiled.authorizes_registry_write());
        assert!(!compiled.authorizes_execution_or_release());
        assert_eq!(compiled.authorization().scope(), bridge.scope());
        assert_eq!(
            compiled.authorization().workflow_bridge_commitment(),
            bridge.commitment()
        );
        assert_eq!(
            compiled.authorization().evaluation_plan_commitment(),
            expected_plan_commitment
        );
        assert_eq!(
            compiled.authorization().source_snapshot(),
            bridge.source_snapshot_ref()
        );
    }

    #[test]
    fn cross_tenant_scope_cannot_authorise_a_change_spec() {
        let (readiness, bridge) = eligible_readiness();
        let mut spec = spec(&bridge);
        spec.claimed_scope =
            CoreTaskScope::new("tenant_b", "job_a", "grant_a", "authority_a").unwrap();
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, spec),
            Err(CompilerError::AuthorizationBindingMismatch)
        );
    }

    #[test]
    fn cross_snapshot_cannot_authorise_a_change_spec() {
        let (readiness, bridge) = eligible_readiness();
        let mut untrusted = spec(&bridge);
        untrusted.claimed_source_snapshot.id = "018f0f4e-7bbd-7000-8000-000000000699".to_owned();
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, untrusted),
            Err(CompilerError::AuthorizationBindingMismatch)
        );
    }

    #[test]
    fn route_or_mechanism_drift_cannot_authorise_a_change_spec() {
        let (readiness, bridge) = eligible_readiness();
        let mut wrong_route = spec(&bridge);
        wrong_route.candidate_route = "flow/other".to_owned();
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, wrong_route),
            Err(CompilerError::RouteOrMechanismMismatch)
        );
        let mut wrong_mechanism = spec(&bridge);
        wrong_mechanism.expected_mechanism = "other_mechanism".to_owned();
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, wrong_mechanism),
            Err(CompilerError::RouteOrMechanismMismatch)
        );
    }

    #[test]
    fn non_flow_kind_or_non_add_operation_cannot_authorise_a_change_spec() {
        let (readiness, bridge) = eligible_readiness();
        let mut wrong_kind = spec(&bridge);
        wrong_kind.operations[0].target_kind = CoreEntityKind::Agent;
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, wrong_kind),
            Err(CompilerError::UnsupportedEntityKind)
        );
        let mut wrong_operation = spec(&bridge);
        wrong_operation.operations[0].operation = ChangeOperationKind::Replace;
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, wrong_operation),
            Err(CompilerError::UnsupportedOperation)
        );
    }

    #[test]
    fn entity_identity_or_precondition_drift_cannot_authorise_a_change_spec() {
        let (readiness, bridge) = eligible_readiness();
        let mut wrong_id = spec(&bridge);
        wrong_id.operations[0].content["id"] = json!("other_flow");
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, wrong_id),
            Err(CompilerError::PreconditionMismatch)
        );
        let mut wrong_precondition = spec(&bridge);
        wrong_precondition.operations[0].precondition_digest = format!("sha256:{}", "f".repeat(64));
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, wrong_precondition),
            Err(CompilerError::PreconditionMismatch)
        );
    }

    #[test]
    fn duplicate_nodes_noncanonical_semver_and_contradictory_flow_shape_are_rejected() {
        let (readiness, bridge) = eligible_readiness();
        let mut duplicate = spec(&bridge);
        duplicate.operations[0].content["nodes"] = json!([
            {"id":"complete","type":"end","config":{"outcome":"completed"}},
            {"id":"complete","type":"end","config":{"outcome":"completed"}}
        ]);
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, duplicate),
            Err(CompilerError::InvalidEntityIdentity)
        );
        let mut noncanonical = spec(&bridge);
        noncanonical.operations[0].content["version"] = json!("01.0.0");
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, noncanonical),
            Err(CompilerError::InvalidEntityIdentity)
        );
        let mut contradictory = spec(&bridge);
        contradictory.operations[0].content["nodes"][0]["next"] = json!({"always":"complete"});
        assert_eq!(
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, contradictory),
            Err(CompilerError::InvalidEntityIdentity)
        );
    }

    #[test]
    fn compiler_revalidates_the_opaque_authorization_before_emitting_a_draft() {
        let (readiness, bridge) = eligible_readiness();
        let mut authorized =
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, spec(&bridge)).unwrap();

        // Simulates storage corruption after trusted composition. External consumers cannot
        // reach either private field; the compiler nevertheless rechecks the sealed fence.
        authorized.untrusted.operations[0].content["version"] = json!("1.0.1");
        assert_eq!(
            ChangeCompiler::compile(authorized),
            Err(CompilerError::AuthorizationBindingMismatch)
        );
    }

    #[test]
    fn authorization_preserves_a_typed_entity_absent_fence_and_seals_full_flow_content() {
        let (readiness, bridge) = eligible_readiness();
        let authorized =
            TrustedChangeAuthorizer::authorize(&readiness, &bridge, spec(&bridge)).unwrap();
        assert_eq!(
            authorized.authorization.precondition().predicate(),
            &ExecutablePredicate::EntityAbsent {
                kind: CoreEntityKind::Flow,
                entity_id: "payment_status_resolution".to_owned(),
            }
        );

        let mut corrupted = authorized;
        // The same id/version and a valid Flow schema must still not reuse the seal.
        corrupted.untrusted.operations[0].content["priority"] = json!(99);
        assert_eq!(
            ChangeCompiler::compile(corrupted),
            Err(CompilerError::AuthorizationBindingMismatch)
        );
    }

    #[test]
    fn canonical_flow_body_digest_is_invariant_to_object_key_order_at_every_depth() {
        let first: Value = serde_json::from_str(
            r#"{"version":"1.0.0","nodes":[{"type":"end","config":{"outcome":"completed"},"id":"complete"}],"id":"payment_status_resolution","priority":1}"#,
        )
        .unwrap();
        let second: Value = serde_json::from_str(
            r#"{"priority":1,"id":"payment_status_resolution","nodes":[{"id":"complete","config":{"outcome":"completed"},"type":"end"}],"version":"1.0.0"}"#,
        )
        .unwrap();

        assert_eq!(canonical_json(&first), canonical_json(&second));
        assert_eq!(
            digest(&[&canonical_json(&first)]),
            digest(&[&canonical_json(&second)])
        );
    }
}
