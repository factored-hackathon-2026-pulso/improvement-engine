//! U17 compiles one explicitly authorized change into immutable Core drafts.
//!
//! This boundary deliberately does not write a registry, execute Agent Core,
//! evaluate a candidate, or release anything. Those effects belong to U18+
//! after their own gates.

use crate::final_eligibility::{FinalEligibility, FinalEligibilityDecision};
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
pub struct ChangeSpec {
    workflow_bridge_commitment: String,
    evaluation_plan_commitment: String,
    expected_mechanism: String,
    operations: Vec<ChangeOperation>,
}

impl ChangeSpec {
    pub fn new(
        workflow_bridge_commitment: impl Into<String>,
        evaluation_plan_commitment: impl Into<String>,
        expected_mechanism: impl Into<String>,
        operations: Vec<ChangeOperation>,
    ) -> Result<Self, CompilerError> {
        let value = Self {
            workflow_bridge_commitment: workflow_bridge_commitment.into(),
            evaluation_plan_commitment: evaluation_plan_commitment.into(),
            expected_mechanism: expected_mechanism.into(),
            operations,
        };
        if !is_digest(&value.workflow_bridge_commitment)
            || !is_digest(&value.evaluation_plan_commitment)
            || value.expected_mechanism.is_empty()
            || value.operations.is_empty()
        {
            return Err(CompilerError::InvalidChangeSpec);
        }
        Ok(value)
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
    commitment: String,
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
    pub fn authorizes_registry_write(&self) -> bool {
        false
    }
    #[must_use]
    pub fn authorizes_execution_or_release(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompilerError {
    FinalEligibilityRequired,
    ReadinessCommitmentMismatch,
    InvalidChangeSpec,
    UnsupportedOperation,
    UnsupportedEntityKind,
    InvalidEntityIdentity,
    DuplicateEntity,
}

pub struct ChangeCompiler;

impl ChangeCompiler {
    pub fn compile(
        readiness: &FinalEligibilityDecision,
        spec: ChangeSpec,
    ) -> Result<CompiledChange, CompilerError> {
        if !readiness.eligible_for_proposal() {
            return Err(CompilerError::FinalEligibilityRequired);
        }
        let FinalEligibilityDecision::Eligible(readiness) = readiness else {
            return Err(CompilerError::FinalEligibilityRequired);
        };
        Self::compile_eligible(readiness, spec)
    }

    fn compile_eligible(
        readiness: &FinalEligibility,
        spec: ChangeSpec,
    ) -> Result<CompiledChange, CompilerError> {
        if spec.workflow_bridge_commitment != readiness.bridge_commitment()
            || spec.evaluation_plan_commitment != readiness.plan_commitment()
        {
            return Err(CompilerError::ReadinessCommitmentMismatch);
        }
        let mut drafts = Vec::with_capacity(spec.operations.len());
        for operation in &spec.operations {
            if operation.operation != ChangeOperationKind::Add {
                return Err(CompilerError::UnsupportedOperation);
            }
            if operation.target_kind != CoreEntityKind::Flow {
                return Err(CompilerError::UnsupportedEntityKind);
            }
            if !is_digest(&operation.precondition_digest) {
                return Err(CompilerError::InvalidChangeSpec);
            }
            let (id, version) =
                entity_identity(&operation.content).ok_or(CompilerError::InvalidEntityIdentity)?;
            if !is_minimal_core_flow(&operation.content) {
                return Err(CompilerError::InvalidEntityIdentity);
            }
            if drafts
                .iter()
                .any(|draft: &EntityDraft| draft.kind == operation.target_kind && draft.id == id)
            {
                return Err(CompilerError::DuplicateEntity);
            }
            let digest = digest(&[
                operation.target_kind.as_str(),
                &id,
                &version,
                &operation.precondition_digest,
                &canonical_json(&operation.content),
            ]);
            drafts.push(EntityDraft {
                kind: operation.target_kind,
                id,
                version,
                content: operation.content.clone(),
                digest,
            });
        }
        let mut pieces = vec![
            spec.workflow_bridge_commitment.as_str(),
            spec.evaluation_plan_commitment.as_str(),
            spec.expected_mechanism.as_str(),
        ];
        pieces.extend(drafts.iter().map(EntityDraft::digest));
        Ok(CompiledChange {
            commitment: digest(&pieces),
            drafts,
        })
    }
}

fn entity_identity(content: &Value) -> Option<(String, String)> {
    let id = content.get("id")?.as_str()?.to_owned();
    let version = content.get("version")?.as_str()?.to_owned();
    (is_identifier(&id) && is_semver(&version)).then_some((id, version))
}

fn canonical_json(value: &Value) -> String {
    serde_json::to_string(value).expect("JSON value serializes")
}

fn digest(pieces: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for piece in pieces {
        hasher.update(piece.len().to_be_bytes());
        hasher.update(piece.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
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
            if [major, minor, patch].iter().all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    )
}

/// The first U17 vertical supports only the smallest executable Core Flow
/// shape. Broader entity/schema coverage must land with its own compatibility
/// slice instead of being accepted as opaque JSON.
fn is_minimal_core_flow(content: &Value) -> bool {
    let Some(priority) = content.get("priority").and_then(Value::as_i64) else {
        return false;
    };
    let Some(nodes) = content.get("nodes").and_then(Value::as_array) else {
        return false;
    };
    priority >= 0
        && !nodes.is_empty()
        && nodes.iter().all(|node| {
            node.get("id")
                .and_then(Value::as_str)
                .is_some_and(is_identifier)
                && node.get("type").and_then(Value::as_str) == Some("end")
                && node
                    .get("config")
                    .and_then(|config| config.get("outcome"))
                    .and_then(Value::as_str)
                    .is_some_and(is_core_outcome)
        })
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

    fn eligible_readiness() -> FinalEligibilityDecision {
        let mut repo = InMemoryArtifactRepository::default();
        let snapshot = append(
            &mut repo,
            "018f0f4e-7bbd-7000-8000-000000000600",
            ArtifactKind::SourceSnapshot,
            json!({}),
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
        FinalEligibilityGate::decide(&report, &bridge, &plan)
    }

    fn spec(readiness: &FinalEligibilityDecision) -> ChangeSpec {
        let FinalEligibilityDecision::Eligible(eligibility) = readiness else {
            panic!("test fixture must create eligible readiness");
        };
        ChangeSpec::new(
            eligibility.bridge_commitment(),
            eligibility.plan_commitment(),
            "transaction_status_lookup",
            vec![ChangeOperation::new(
                ChangeOperationKind::Add,
                CoreEntityKind::Flow,
                json!({"id":"payment_status_resolution","version":"1.0.0","priority":1,"nodes":[{"id":"complete","type":"end","config":{"outcome":"completed"}}]}),
                format!("sha256:{}", "e".repeat(64)),
            )],
        )
        .expect("fixed spec is valid")
    }

    #[test]
    fn eligible_readiness_compiles_one_immutable_draft_without_registry_or_release_effect() {
        let readiness = eligible_readiness();
        let compiled = ChangeCompiler::compile(&readiness, spec(&readiness))
            .expect("eligible readiness compiles");
        assert_eq!(compiled.drafts().len(), 1);
        assert_eq!(compiled.drafts()[0].kind(), CoreEntityKind::Flow);
        assert_eq!(compiled.drafts()[0].id(), "payment_status_resolution");
        assert!(!compiled.authorizes_registry_write());
        assert!(!compiled.authorizes_execution_or_release());
    }

    #[test]
    fn altered_readiness_commitment_cannot_authorise_a_change_spec() {
        let readiness = eligible_readiness();
        let FinalEligibilityDecision::Eligible(eligibility) = &readiness else {
            panic!("fixture must be eligible")
        };
        let spec = ChangeSpec::new(
            format!("sha256:{}", "f".repeat(64)),
            eligibility.plan_commitment(),
            "transaction_status_lookup",
            vec![ChangeOperation::new(
                ChangeOperationKind::Add,
                CoreEntityKind::Flow,
                json!({"id":"payment_status_resolution","version":"1.0.0"}),
                format!("sha256:{}", "e".repeat(64)),
            )],
        )
        .unwrap();
        assert_eq!(
            ChangeCompiler::compile(&readiness, spec),
            Err(CompilerError::ReadinessCommitmentMismatch)
        );
    }
}
