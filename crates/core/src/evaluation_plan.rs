//! U20 seals repository-attested evaluation inputs before any execution.

use crate::workflow_bridge::{LinkGrade, WorkflowBridgeContract};
use crate::{ArtifactKind, ArtifactReference, ArtifactRepository};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationPlan {
    bridge_commitment: String,
    source_snapshot_ref: ArtifactReference,
    baseline_ref: ArtifactReference,
    oracle_ref: ArtifactReference,
    suite_ref: ArtifactReference,
    target_outcome: String,
    unit_of_analysis: String,
    oracle_measure: String,
    commitment: String,
}

impl EvaluationPlan {
    /// Verifies exact immutable artifact revisions before freezing a plan. A
    /// content hash alone is never accepted as identity or authority.
    pub fn seal_from_bridge<R: ArtifactRepository>(
        bridge: &WorkflowBridgeContract,
        baseline_ref: ArtifactReference,
        oracle_ref: ArtifactReference,
        suite_ref: ArtifactReference,
        artifacts: &mut R,
    ) -> Result<Self, EvaluationPlanError> {
        if bridge.link_grade() != LinkGrade::MechanismProxy
            || !bridge.alternatives().includes_candidate_route()
        {
            return Err(EvaluationPlanError::BridgeNotEvaluable);
        }
        let scope = bridge.scope();
        let snapshot = bridge.source_snapshot_ref();
        let _baseline =
            verified_input(artifacts, scope, snapshot, &baseline_ref, "baseline", false)?;
        let oracle = verified_input(artifacts, scope, snapshot, &oracle_ref, "oracle", true)?;
        let _suite = verified_input(artifacts, scope, snapshot, &suite_ref, "suite", false)?;
        if baseline_ref == oracle_ref || baseline_ref == suite_ref || oracle_ref == suite_ref {
            return Err(EvaluationPlanError::DuplicateInputReference);
        }
        let outcome = field(&oracle, "target_outcome")?;
        let unit = field(&oracle, "unit_of_analysis")?;
        let measure = field(&oracle, "oracle_measure")?;
        if outcome != bridge.input().target_outcome() || unit != bridge.input().unit_of_analysis() {
            return Err(EvaluationPlanError::OracleSemanticMismatch);
        }
        let values = [
            bridge.commitment(),
            &snapshot.tenant_id,
            &snapshot.id,
            &snapshot.revision.to_string(),
            &snapshot.digest,
            &baseline_ref.tenant_id,
            &baseline_ref.id,
            &baseline_ref.revision.to_string(),
            &baseline_ref.digest,
            &oracle_ref.tenant_id,
            &oracle_ref.id,
            &oracle_ref.revision.to_string(),
            &oracle_ref.digest,
            &suite_ref.tenant_id,
            &suite_ref.id,
            &suite_ref.revision.to_string(),
            &suite_ref.digest,
            &outcome,
            &unit,
            &measure,
        ];
        let commitment = digest(&values);
        Ok(Self {
            bridge_commitment: bridge.commitment().to_owned(),
            source_snapshot_ref: snapshot.clone(),
            baseline_ref,
            oracle_ref,
            suite_ref,
            target_outcome: outcome,
            unit_of_analysis: unit,
            oracle_measure: measure,
            commitment,
        })
    }
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }
    #[must_use]
    pub fn allows_same_outcome_claim(&self) -> bool {
        false
    }
    #[must_use]
    pub fn eligible_for_proposal(&self) -> bool {
        false
    }
}

fn verified_input<R: ArtifactRepository>(
    artifacts: &mut R,
    scope: &crate::core_task::CoreTaskScope,
    snapshot: &ArtifactReference,
    reference: &ArtifactReference,
    role: &str,
    oracle_authority: bool,
) -> Result<Value, EvaluationPlanError> {
    if reference.tenant_id != scope.tenant_id() {
        return Err(EvaluationPlanError::CrossTenantReference);
    }
    let artifact = artifacts
        .get(&reference.tenant_id, &reference.id, reference.revision)
        .map_err(|_| EvaluationPlanError::ReferenceUnavailable)?
        .ok_or(EvaluationPlanError::ReferenceUnavailable)?;
    if artifact.reference() != *reference
        || artifact.kind != ArtifactKind::ScenarioSet
        || artifact.source_snapshot_ref.as_ref() != Some(snapshot)
    {
        return Err(EvaluationPlanError::ReferenceMismatch);
    }
    if field(&artifact.payload, "evaluation_role")? != role {
        return Err(EvaluationPlanError::ReferenceMismatch);
    }
    let scoped = artifact
        .payload
        .get("scope")
        .and_then(Value::as_object)
        .ok_or(EvaluationPlanError::ScopeMismatch)?;
    for (key, expected) in [
        ("tenant_id", scope.tenant_id()),
        ("job_id", scope.job_id()),
        ("grant_id", scope.grant_id()),
        ("authority_ref", scope.authority_ref()),
    ] {
        if scoped.get(key).and_then(Value::as_str) != Some(expected) {
            return Err(EvaluationPlanError::ScopeMismatch);
        }
    }
    if oracle_authority
        && artifact
            .payload
            .get("oracle_authority")
            .and_then(Value::as_str)
            != Some(scope.authority_ref())
    {
        return Err(EvaluationPlanError::OracleAuthorityMismatch);
    }
    Ok(artifact.payload)
}
fn field(value: &Value, name: &str) -> Result<String, EvaluationPlanError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or(EvaluationPlanError::InvalidInput)
}
fn digest(values: &[&str]) -> String {
    let mut h = Sha256::new();
    for v in values {
        h.update(v.len().to_be_bytes());
        h.update(v.as_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationPlanError {
    BridgeNotEvaluable,
    CrossTenantReference,
    ReferenceUnavailable,
    ReferenceMismatch,
    ScopeMismatch,
    OracleAuthorityMismatch,
    OracleSemanticMismatch,
    DuplicateInputReference,
    InvalidInput,
}

#[cfg(test)]
mod tests {
    use super::{EvaluationPlanError, verified_input};
    use crate::core_task::CoreTaskScope;
    use crate::{ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository};
    use serde_json::json;

    fn scoped_payload(role: &str) -> serde_json::Value {
        json!({"evaluation_role":role,"scope":{"tenant_id":"tenant_a","job_id":"job_a","grant_id":"grant_a","authority_ref":"authority_a"},"oracle_authority":"authority_a","target_outcome":"payment_resolution","unit_of_analysis":"customer_goal","oracle_measure":"resolved"})
    }

    #[test]
    fn rejects_cross_tenant_and_wrong_snapshot_references_before_any_plan_can_be_sealed() {
        let mut repo = InMemoryArtifactRepository::default();
        let snapshot = repo
            .append(
                None,
                ArtifactDraft::new(
                    "tenant_a",
                    "018f0f4e-7bbd-7000-8000-000000000101",
                    1,
                    ArtifactKind::SourceSnapshot,
                    json!({}),
                    None,
                ),
            )
            .unwrap()
            .reference();
        let scenario = repo
            .append(
                None,
                ArtifactDraft::new(
                    "tenant_a",
                    "018f0f4e-7bbd-7000-8000-000000000102",
                    1,
                    ArtifactKind::ScenarioSet,
                    scoped_payload("oracle"),
                    Some(snapshot.clone()),
                ),
            )
            .unwrap()
            .reference();
        let scope = CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap();
        assert!(verified_input(&mut repo, &scope, &snapshot, &scenario, "oracle", true).is_ok());
        let mut cross = scenario.clone();
        cross.tenant_id = "tenant_b".to_owned();
        assert_eq!(
            verified_input(&mut repo, &scope, &snapshot, &cross, "oracle", true),
            Err(EvaluationPlanError::CrossTenantReference)
        );
        let other_snapshot = repo
            .append(
                Some(1),
                ArtifactDraft::new(
                    "tenant_a",
                    "018f0f4e-7bbd-7000-8000-000000000101",
                    2,
                    ArtifactKind::SourceSnapshot,
                    json!({"other":true}),
                    None,
                ),
            )
            .unwrap()
            .reference();
        assert_eq!(
            verified_input(
                &mut repo,
                &scope,
                &other_snapshot,
                &scenario,
                "oracle",
                true
            ),
            Err(EvaluationPlanError::ReferenceMismatch)
        );
    }
}
