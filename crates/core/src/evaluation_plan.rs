//! U20 seals the comparison contract before a candidate is executed.
//!
//! It deliberately does not execute a sandbox arm, construct a candidate, or
//! authorize a release. A sealed plan remains a `MechanismProxy` input, never
//! proof of an outcome or final proposal eligibility.

use crate::workflow_bridge::{LinkGrade, WorkflowBridgeContract};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSpec {
    tenant_id: String,
    target_outcome: String,
    authority_digest: String,
    source_snapshot_digest: String,
}

impl OracleSpec {
    pub fn new(
        tenant_id: impl Into<String>,
        target_outcome: impl Into<String>,
        authority_digest: impl Into<String>,
        source_snapshot_digest: impl Into<String>,
    ) -> Result<Self, EvaluationPlanError> {
        let value = Self {
            tenant_id: tenant_id.into(),
            target_outcome: target_outcome.into(),
            authority_digest: authority_digest.into(),
            source_snapshot_digest: source_snapshot_digest.into(),
        };
        if !identifier(&value.tenant_id)
            || !identifier(&value.target_outcome)
            || !sha256_digest(&value.authority_digest)
            || !sha256_digest(&value.source_snapshot_digest)
        {
            return Err(EvaluationPlanError::InvalidOracle);
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationMetric {
    metric_id: String,
    unit_of_analysis: String,
}

impl EvaluationMetric {
    pub fn new(
        metric_id: impl Into<String>,
        unit_of_analysis: impl Into<String>,
    ) -> Result<Self, EvaluationPlanError> {
        let value = Self {
            metric_id: metric_id.into(),
            unit_of_analysis: unit_of_analysis.into(),
        };
        if !identifier(&value.metric_id) || !identifier(&value.unit_of_analysis) {
            return Err(EvaluationPlanError::InvalidMetric);
        }
        Ok(value)
    }
}

/// Digest-only suite selection. Case content, including final-holdout cases,
/// never enters the plan API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationSuite {
    development_suite_digest: String,
    final_holdout_digest: String,
}

impl EvaluationSuite {
    pub fn new(
        development_suite_digest: impl Into<String>,
        final_holdout_digest: impl Into<String>,
    ) -> Result<Self, EvaluationPlanError> {
        let value = Self {
            development_suite_digest: development_suite_digest.into(),
            final_holdout_digest: final_holdout_digest.into(),
        };
        if !sha256_digest(&value.development_suite_digest)
            || !sha256_digest(&value.final_holdout_digest)
            || value.development_suite_digest == value.final_holdout_digest
        {
            return Err(EvaluationPlanError::InvalidSuite);
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationPlan {
    tenant_id: String,
    bridge_commitment: String,
    source_snapshot_digest: String,
    baseline_digest: String,
    oracle: OracleSpec,
    metric: EvaluationMetric,
    suite: EvaluationSuite,
    commitment: String,
}

impl EvaluationPlan {
    pub fn seal_from_bridge(
        bridge: &WorkflowBridgeContract,
        baseline_digest: impl Into<String>,
        oracle: OracleSpec,
        metric: EvaluationMetric,
        suite: EvaluationSuite,
    ) -> Result<Self, EvaluationPlanError> {
        let baseline_digest = baseline_digest.into();
        if bridge.link_grade() != LinkGrade::MechanismProxy
            || !bridge.alternatives().includes_candidate_route()
        {
            return Err(EvaluationPlanError::BridgeNotEvaluable);
        }
        if !sha256_digest(&baseline_digest) {
            return Err(EvaluationPlanError::InvalidBaseline);
        }
        let tenant_id = bridge.scope().tenant_id().to_owned();
        if oracle.tenant_id != tenant_id {
            return Err(EvaluationPlanError::OracleTenantMismatch);
        }
        if oracle.target_outcome != bridge.input().target_outcome()
            || oracle.source_snapshot_digest != bridge.source_snapshot_ref().digest
        {
            return Err(EvaluationPlanError::OracleBindingMismatch);
        }
        if metric.unit_of_analysis != bridge.input().unit_of_analysis() {
            return Err(EvaluationPlanError::MetricBindingMismatch);
        }
        let bridge_commitment = bridge.commitment().to_owned();
        let source_snapshot_digest = bridge.source_snapshot_ref().digest.clone();
        let commitment = plan_digest(
            &tenant_id,
            &bridge_commitment,
            &source_snapshot_digest,
            &baseline_digest,
            &oracle,
            &metric,
            &suite,
        );
        Ok(Self {
            tenant_id,
            bridge_commitment,
            source_snapshot_digest,
            baseline_digest,
            oracle,
            metric,
            suite,
            commitment,
        })
    }

    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }

    /// U20 can authorize only a sandbox mechanism comparison, never a business
    /// outcome assertion or final promotion.
    #[must_use]
    pub fn allows_same_outcome_claim(&self) -> bool {
        false
    }

    #[must_use]
    pub fn eligible_for_proposal(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationPlanError {
    InvalidOracle,
    InvalidMetric,
    InvalidSuite,
    InvalidBaseline,
    BridgeNotEvaluable,
    OracleTenantMismatch,
    OracleBindingMismatch,
    MetricBindingMismatch,
}

fn plan_digest(
    tenant_id: &str,
    bridge_commitment: &str,
    source_snapshot_digest: &str,
    baseline_digest: &str,
    oracle: &OracleSpec,
    metric: &EvaluationMetric,
    suite: &EvaluationSuite,
) -> String {
    let values = [
        tenant_id,
        bridge_commitment,
        source_snapshot_digest,
        baseline_digest,
        &oracle.tenant_id,
        &oracle.target_outcome,
        &oracle.authority_digest,
        &oracle.source_snapshot_digest,
        &metric.metric_id,
        &metric.unit_of_analysis,
        &suite.development_suite_digest,
        &suite.final_holdout_digest,
    ];
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.len().to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/'))
}

fn sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::{
        EvaluationMetric, EvaluationPlan, EvaluationPlanError, EvaluationSuite, OracleSpec,
    };
    use crate::independent_verifier::{VerificationStatus, report_for_workflow_bridge_test};
    use crate::workflow_bridge::{
        WorkflowBridge, WorkflowBridgeContract, WorkflowBridgeInput, WorkflowBridgeValidator,
        WorkflowCatalogueValidation,
    };

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn bridge(status: VerificationStatus) -> WorkflowBridgeContract {
        let report = report_for_workflow_bridge_test(status);
        let input = WorkflowBridgeInput::new(
            "payment_resolution",
            "customer_goal",
            digest('a'),
            "customer_id",
            "2026-09-30T00:00:00Z",
            "flow/payment_status",
            "payment_status_explains_next_step",
            "after_contact_classification",
            "customer_receives_correct_payment_status",
            "scenario_oracle/payment_status_resolution_v1",
            vec![
                report.receipt().evidence_commitment().to_owned(),
                report.receipt().digest().to_owned(),
                report.source_snapshot_ref().digest.clone(),
            ],
        )
        .unwrap();
        let validation = WorkflowCatalogueValidation::new(
            digest('b'),
            digest('c'),
            true,
            true,
            true,
            true,
            true,
            true,
            true,
        )
        .unwrap();
        let evidence = WorkflowBridgeValidator::validate(&report, &input, validation).unwrap();
        WorkflowBridge::assess_verified(&report, input, evidence)
            .unwrap()
            .contract()
            .clone()
    }

    fn oracle(contract: &WorkflowBridgeContract) -> OracleSpec {
        OracleSpec::new(
            contract.scope().tenant_id(),
            contract.input().target_outcome(),
            digest('d'),
            &contract.source_snapshot_ref().digest,
        )
        .unwrap()
    }

    fn metric() -> EvaluationMetric {
        EvaluationMetric::new("resolved_goal_rate", "customer_goal").unwrap()
    }

    fn suite() -> EvaluationSuite {
        EvaluationSuite::new(digest('e'), digest('f')).unwrap()
    }

    #[test]
    fn seals_mechanism_proxy_inputs_without_claiming_outcome_or_eligibility() {
        let contract = bridge(VerificationStatus::Supported);
        let plan = EvaluationPlan::seal_from_bridge(
            &contract,
            digest('1'),
            oracle(&contract),
            metric(),
            suite(),
        )
        .unwrap();

        assert!(super::sha256_digest(plan.commitment()));
        assert!(!plan.allows_same_outcome_claim());
        assert!(!plan.eligible_for_proposal());
    }

    #[test]
    fn rejects_unlinked_bridge_and_cross_tenant_oracle_before_plan_exists() {
        let unevaluable = bridge(VerificationStatus::Uncertain);
        assert_eq!(
            EvaluationPlan::seal_from_bridge(
                &unevaluable,
                digest('1'),
                oracle(&unevaluable),
                metric(),
                suite()
            ),
            Err(EvaluationPlanError::BridgeNotEvaluable)
        );

        let contract = bridge(VerificationStatus::Supported);
        let cross_tenant = OracleSpec::new(
            "other_tenant",
            contract.input().target_outcome(),
            digest('d'),
            &contract.source_snapshot_ref().digest,
        )
        .unwrap();
        assert_eq!(
            EvaluationPlan::seal_from_bridge(
                &contract,
                digest('1'),
                cross_tenant,
                metric(),
                suite()
            ),
            Err(EvaluationPlanError::OracleTenantMismatch)
        );
    }

    #[test]
    fn changing_any_baseline_or_oracle_input_changes_the_sealed_plan_commitment() {
        let contract = bridge(VerificationStatus::Supported);
        let first = EvaluationPlan::seal_from_bridge(
            &contract,
            digest('1'),
            oracle(&contract),
            metric(),
            suite(),
        )
        .unwrap();
        let second = EvaluationPlan::seal_from_bridge(
            &contract,
            digest('2'),
            OracleSpec::new(
                contract.scope().tenant_id(),
                contract.input().target_outcome(),
                digest('3'),
                &contract.source_snapshot_ref().digest,
            )
            .unwrap(),
            metric(),
            suite(),
        )
        .unwrap();

        assert_ne!(first.commitment(), second.commitment());
    }
}
