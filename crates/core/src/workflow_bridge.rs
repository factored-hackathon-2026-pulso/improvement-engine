//! U16: a provisional, deterministic bridge from verified evidence to one
//! sandbox mechanism hypothesis and explicit alternatives.
//!
//! This module deliberately does not create a ChangeSpec, select an Agent Core
//! artifact, persist a proposal, or decide final eligibility.  U20/U35 own the
//! sealed oracle/baseline and final promotion gates.  A supported verifier
//! result therefore reaches at most `mechanism_proxy` in this slice.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::core_task::CoreTaskScope;
use crate::independent_verifier::{VerificationReport, VerificationStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkGrade {
    MechanismProxy,
    Unlinked,
    NotEvaluable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingLink {
    StableEntityKey,
    PopulationJoin,
    PreInterventionState,
    IndependentOracle,
    ValidTemporalAvailability,
    EligibleSample,
    RouteCapability,
    VerificationRefuted,
    VerificationUncertain,
}

/// Facts computed outside the LLM from available source/catalogue contracts.
/// No caller can supply a [`LinkGrade`]; it is derived by [`WorkflowBridge`].
#[derive(Clone, Copy, Eq, PartialEq, Serialize)]
pub struct LinkEvidence {
    route_can_alter_outcome: bool,
    population_join_available: bool,
    stable_entity_key_available: bool,
    pre_intervention_state_available: bool,
    independent_oracle_available: bool,
    temporal_availability_valid: bool,
    eligible_sample_available: bool,
}

impl LinkEvidence {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn mechanism_proxy() -> Self {
        Self {
            route_can_alter_outcome: true,
            population_join_available: false,
            stable_entity_key_available: false,
            pre_intervention_state_available: false,
            independent_oracle_available: false,
            temporal_availability_valid: true,
            eligible_sample_available: true,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn new(
        route_can_alter_outcome: bool,
        population_join_available: bool,
        stable_entity_key_available: bool,
        pre_intervention_state_available: bool,
        independent_oracle_available: bool,
        temporal_availability_valid: bool,
        eligible_sample_available: bool,
    ) -> Self {
        Self {
            route_can_alter_outcome,
            population_join_available,
            stable_entity_key_available,
            pre_intervention_state_available,
            independent_oracle_available,
            temporal_availability_valid,
            eligible_sample_available,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkflowBridgeInput {
    target_outcome: String,
    unit_of_analysis: String,
    eligible_population_query_digest: String,
    entity_key: String,
    as_of_cutoff: String,
    candidate_route: String,
    mechanism: String,
    intervention_point: String,
    observable_effect: String,
    oracle_measure: String,
    support_refs: Vec<String>,
}

impl WorkflowBridgeInput {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        target_outcome: impl Into<String>,
        unit_of_analysis: impl Into<String>,
        eligible_population_query_digest: impl Into<String>,
        entity_key: impl Into<String>,
        as_of_cutoff: impl Into<String>,
        candidate_route: impl Into<String>,
        mechanism: impl Into<String>,
        intervention_point: impl Into<String>,
        observable_effect: impl Into<String>,
        oracle_measure: impl Into<String>,
        mut support_refs: Vec<String>,
    ) -> Result<Self, WorkflowBridgeError> {
        let input = Self {
            target_outcome: target_outcome.into(),
            unit_of_analysis: unit_of_analysis.into(),
            eligible_population_query_digest: eligible_population_query_digest.into(),
            entity_key: entity_key.into(),
            as_of_cutoff: as_of_cutoff.into(),
            candidate_route: candidate_route.into(),
            mechanism: mechanism.into(),
            intervention_point: intervention_point.into(),
            observable_effect: observable_effect.into(),
            oracle_measure: oracle_measure.into(),
            support_refs: {
                support_refs.sort();
                support_refs.dedup();
                support_refs
            },
        };
        if [
            &input.target_outcome,
            &input.unit_of_analysis,
            &input.entity_key,
            &input.as_of_cutoff,
            &input.candidate_route,
            &input.mechanism,
            &input.intervention_point,
            &input.observable_effect,
            &input.oracle_measure,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
            || !is_sha256_digest(&input.eligible_population_query_digest)
            || input.support_refs.is_empty()
            || input.support_refs.len() > 32
            || input
                .support_refs
                .iter()
                .any(|reference| !is_sha256_digest(reference))
        {
            return Err(WorkflowBridgeError::InvalidInput);
        }
        Ok(input)
    }

    #[must_use]
    pub fn target_outcome(&self) -> &str {
        &self.target_outcome
    }
    #[must_use]
    pub fn unit_of_analysis(&self) -> &str {
        &self.unit_of_analysis
    }
    #[must_use]
    pub fn eligible_population_query_digest(&self) -> &str {
        &self.eligible_population_query_digest
    }
    #[must_use]
    pub fn entity_key(&self) -> &str {
        &self.entity_key
    }
    #[must_use]
    pub fn as_of_cutoff(&self) -> &str {
        &self.as_of_cutoff
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
    pub fn intervention_point(&self) -> &str {
        &self.intervention_point
    }
    #[must_use]
    pub fn observable_effect(&self) -> &str {
        &self.observable_effect
    }
    #[must_use]
    pub fn oracle_measure(&self) -> &str {
        &self.oracle_measure
    }
    #[must_use]
    pub fn support_refs(&self) -> &[String] {
        &self.support_refs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Alternatives {
    candidate_route: Option<String>,
}

impl Alternatives {
    #[must_use]
    pub fn includes_do_nothing(&self) -> bool {
        true
    }

    #[must_use]
    pub fn includes_candidate_route(&self) -> bool {
        self.candidate_route.is_some()
    }

    #[must_use]
    pub fn candidate_route(&self) -> Option<&str> {
        self.candidate_route.as_deref()
    }
}

/// Read-only output consumed by later U20/U35 boundaries. It retains every
/// normative field; no consumer has to reconstruct it from a digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowBridgeContract {
    scope: CoreTaskScope,
    source_snapshot_ref: crate::ArtifactReference,
    input: WorkflowBridgeInput,
    candidate_digest: String,
    provenance_commitment: String,
    verification_input_commitment: String,
    verification_receipt_digest: String,
    linkage_validation_receipt_digest: String,
    link_grade: LinkGrade,
    missing_links: Vec<MissingLink>,
    alternatives: Alternatives,
    commitment: String,
}

impl WorkflowBridgeContract {
    #[must_use]
    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }

    #[must_use]
    pub fn source_snapshot_ref(&self) -> &crate::ArtifactReference {
        &self.source_snapshot_ref
    }

    #[must_use]
    pub fn input(&self) -> &WorkflowBridgeInput {
        &self.input
    }

    #[must_use]
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }

    #[must_use]
    pub fn provenance_commitment(&self) -> &str {
        &self.provenance_commitment
    }

    #[must_use]
    pub fn verification_input_commitment(&self) -> &str {
        &self.verification_input_commitment
    }

    #[must_use]
    pub fn verification_receipt_digest(&self) -> &str {
        &self.verification_receipt_digest
    }

    #[must_use]
    pub fn linkage_validation_receipt_digest(&self) -> &str {
        &self.linkage_validation_receipt_digest
    }

    #[must_use]
    pub fn link_grade(&self) -> LinkGrade {
        self.link_grade
    }

    #[must_use]
    pub fn missing_links(&self) -> &[MissingLink] {
        &self.missing_links
    }

    #[must_use]
    pub fn alternatives(&self) -> &Alternatives {
        &self.alternatives
    }

    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }
}

/// A non-persisted, non-authoritative bridge. Its fields are intentionally
/// private so callers cannot elevate a proxy to a same-outcome claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowBridge {
    contract: WorkflowBridgeContract,
}

impl WorkflowBridge {
    /// Assesses only an independently verified U14 result. It deliberately
    /// cannot accept a raw Scout draft or caller-provided verification status.
    pub fn assess_verified(
        report: &VerificationReport,
        input: WorkflowBridgeInput,
        evidence: ValidatedLinkEvidence,
    ) -> Result<Self, WorkflowBridgeError> {
        if !input
            .support_refs
            .iter()
            .any(|reference| reference == report.receipt().evidence_commitment())
            || !input
                .support_refs
                .iter()
                .any(|reference| reference == report.receipt().digest())
            || !input
                .support_refs
                .iter()
                .any(|reference| reference == &report.source_snapshot_ref().digest)
        {
            return Err(WorkflowBridgeError::MissingVerificationSupportReference);
        }
        if !evidence.matches(report, &input) {
            return Err(WorkflowBridgeError::ValidationBindingMismatch);
        }
        let status = report.status();
        let (grade, missing_links) = derive_grade(evidence.facts, status);
        let candidate_route =
            matches!(grade, LinkGrade::MechanismProxy).then(|| input.candidate_route.clone());
        let commitment = bridge_commitment(report, &input, &evidence, status);
        Ok(Self {
            contract: WorkflowBridgeContract {
                scope: report.scope().clone(),
                source_snapshot_ref: report.source_snapshot_ref().clone(),
                input,
                candidate_digest: report.candidate_digest().to_owned(),
                provenance_commitment: report.provenance_commitment().to_owned(),
                verification_input_commitment: report.input_commitment().to_owned(),
                verification_receipt_digest: report.receipt().digest().to_owned(),
                linkage_validation_receipt_digest: evidence.receipt_digest.clone(),
                link_grade: grade,
                missing_links,
                alternatives: Alternatives { candidate_route },
                commitment,
            },
        })
    }

    #[must_use]
    pub fn link_grade(&self) -> LinkGrade {
        self.contract.link_grade()
    }

    #[must_use]
    pub fn missing_links(&self) -> &[MissingLink] {
        self.contract.missing_links()
    }

    #[must_use]
    pub fn alternatives(&self) -> &Alternatives {
        self.contract.alternatives()
    }

    #[must_use]
    pub fn allows_sandbox_mechanism_claim(&self) -> bool {
        self.contract.link_grade() == LinkGrade::MechanismProxy
    }

    #[must_use]
    pub fn allows_same_outcome_claim(&self) -> bool {
        false
    }

    #[must_use]
    pub fn commitment(&self) -> &str {
        self.contract.commitment()
    }

    #[must_use]
    pub fn candidate_digest(&self) -> &str {
        self.contract.candidate_digest()
    }

    #[must_use]
    pub fn contract(&self) -> &WorkflowBridgeContract {
        &self.contract
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowBridgeError {
    InvalidInput,
    InvalidValidationInput,
    MissingVerificationSupportReference,
    ValidationBindingMismatch,
}

/// Facts read by the trusted catalogue/source adapter. This input is internal:
/// proposal builders and external callers cannot manufacture a grade.
#[allow(dead_code)] // Constructed by the catalogue/source adapter composition slice.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct WorkflowCatalogueValidation {
    workflow_catalogue_digest: String,
    source_availability_receipt_digest: String,
    facts: LinkEvidence,
}

impl WorkflowCatalogueValidation {
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Constructed by the catalogue/source adapter composition slice.
    pub(crate) fn new(
        workflow_catalogue_digest: impl Into<String>,
        source_availability_receipt_digest: impl Into<String>,
        route_can_alter_outcome: bool,
        population_join_available: bool,
        stable_entity_key_available: bool,
        pre_intervention_state_available: bool,
        independent_oracle_available: bool,
        temporal_availability_valid: bool,
        eligible_sample_available: bool,
    ) -> Result<Self, WorkflowBridgeError> {
        let workflow_catalogue_digest = workflow_catalogue_digest.into();
        let source_availability_receipt_digest = source_availability_receipt_digest.into();
        if !is_sha256_digest(&workflow_catalogue_digest)
            || !is_sha256_digest(&source_availability_receipt_digest)
        {
            return Err(WorkflowBridgeError::InvalidValidationInput);
        }
        Ok(Self {
            workflow_catalogue_digest,
            source_availability_receipt_digest,
            facts: LinkEvidence {
                route_can_alter_outcome,
                population_join_available,
                stable_entity_key_available,
                pre_intervention_state_available,
                independent_oracle_available,
                temporal_availability_valid,
                eligible_sample_available,
            },
        })
    }
}

/// Opaque output of the deterministic catalogue/source validator. It prevents
/// a builder from supplying its own booleans for outcome, population or route.
#[derive(Clone, Eq, PartialEq)]
pub struct ValidatedLinkEvidence {
    report_input_commitment: String,
    bridge_input_commitment: String,
    source_snapshot_digest: String,
    workflow_catalogue_digest: String,
    source_availability_receipt_digest: String,
    facts: LinkEvidence,
    receipt_digest: String,
}

impl ValidatedLinkEvidence {
    fn matches(&self, report: &VerificationReport, input: &WorkflowBridgeInput) -> bool {
        self.report_input_commitment == report.input_commitment()
            && self.bridge_input_commitment == bridge_input_commitment(input)
            && self.source_snapshot_digest == report.source_snapshot_ref().digest
            && self.receipt_digest
                == linkage_receipt_digest(
                    report,
                    input,
                    &self.workflow_catalogue_digest,
                    &self.source_availability_receipt_digest,
                    self.facts,
                )
    }
}

#[allow(dead_code)] // Called by the catalogue/source adapter composition slice.
pub(crate) struct WorkflowBridgeValidator;

impl WorkflowBridgeValidator {
    #[allow(dead_code)] // Called by the catalogue/source adapter composition slice.
    pub(crate) fn validate(
        report: &VerificationReport,
        input: &WorkflowBridgeInput,
        validation: WorkflowCatalogueValidation,
    ) -> Result<ValidatedLinkEvidence, WorkflowBridgeError> {
        let receipt_digest = linkage_receipt_digest(
            report,
            input,
            &validation.workflow_catalogue_digest,
            &validation.source_availability_receipt_digest,
            validation.facts,
        );
        Ok(ValidatedLinkEvidence {
            report_input_commitment: report.input_commitment().to_owned(),
            bridge_input_commitment: bridge_input_commitment(input),
            source_snapshot_digest: report.source_snapshot_ref().digest.clone(),
            workflow_catalogue_digest: validation.workflow_catalogue_digest,
            source_availability_receipt_digest: validation.source_availability_receipt_digest,
            facts: validation.facts,
            receipt_digest,
        })
    }
}

fn derive_grade(
    evidence: LinkEvidence,
    verification_status: VerificationStatus,
) -> (LinkGrade, Vec<MissingLink>) {
    match verification_status {
        VerificationStatus::Refuted => {
            (LinkGrade::Unlinked, vec![MissingLink::VerificationRefuted])
        }
        VerificationStatus::Uncertain => (
            LinkGrade::NotEvaluable,
            vec![MissingLink::VerificationUncertain],
        ),
        VerificationStatus::Supported if !evidence.route_can_alter_outcome => {
            (LinkGrade::Unlinked, vec![MissingLink::RouteCapability])
        }
        VerificationStatus::Supported if !evidence.temporal_availability_valid => (
            LinkGrade::NotEvaluable,
            vec![MissingLink::ValidTemporalAvailability],
        ),
        VerificationStatus::Supported if !evidence.eligible_sample_available => {
            (LinkGrade::NotEvaluable, vec![MissingLink::EligibleSample])
        }
        VerificationStatus::Supported => {
            let mut missing = Vec::new();
            if !evidence.stable_entity_key_available {
                missing.push(MissingLink::StableEntityKey);
            }
            if !evidence.population_join_available {
                missing.push(MissingLink::PopulationJoin);
            }
            if !evidence.pre_intervention_state_available {
                missing.push(MissingLink::PreInterventionState);
            }
            if !evidence.independent_oracle_available {
                missing.push(MissingLink::IndependentOracle);
            }
            // U16 cannot make the stronger assertion even if all facts exist.
            (LinkGrade::MechanismProxy, missing)
        }
    }
}

fn bridge_commitment(
    report: &VerificationReport,
    input: &WorkflowBridgeInput,
    evidence: &ValidatedLinkEvidence,
    status: VerificationStatus,
) -> String {
    #[derive(Serialize)]
    struct Commitment<'a> {
        candidate_digest: &'a str,
        provenance_commitment: &'a str,
        verification_input_commitment: &'a str,
        verification_receipt_digest: &'a str,
        source_snapshot_ref: &'a crate::ArtifactReference,
        input: &'a WorkflowBridgeInput,
        linkage_validation_receipt_digest: &'a str,
        evidence: LinkEvidence,
        status: VerificationStatus,
    }
    let bytes = serde_json::to_vec(&Commitment {
        candidate_digest: report.candidate_digest(),
        provenance_commitment: report.provenance_commitment(),
        verification_input_commitment: report.input_commitment(),
        verification_receipt_digest: report.receipt().digest(),
        source_snapshot_ref: report.source_snapshot_ref(),
        input,
        linkage_validation_receipt_digest: &evidence.receipt_digest,
        evidence: evidence.facts,
        status,
    })
    .expect("workflow bridge commitment values are serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn bridge_input_commitment(input: &WorkflowBridgeInput) -> String {
    let bytes = serde_json::to_vec(input).expect("bridge input values are serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn linkage_receipt_digest(
    report: &VerificationReport,
    input: &WorkflowBridgeInput,
    workflow_catalogue_digest: &str,
    source_availability_receipt_digest: &str,
    facts: LinkEvidence,
) -> String {
    #[derive(Serialize)]
    struct LinkageReceipt<'a> {
        report_input_commitment: &'a str,
        source_snapshot_ref: &'a crate::ArtifactReference,
        bridge_input_commitment: String,
        workflow_catalogue_digest: &'a str,
        source_availability_receipt_digest: &'a str,
        facts: LinkEvidence,
    }
    let bytes = serde_json::to_vec(&LinkageReceipt {
        report_input_commitment: report.input_commitment(),
        source_snapshot_ref: report.source_snapshot_ref(),
        bridge_input_commitment: bridge_input_commitment(input),
        workflow_catalogue_digest,
        source_availability_receipt_digest,
        facts,
    })
    .expect("linkage receipt values are serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::{
        LinkEvidence, LinkGrade, MissingLink, WorkflowBridge, WorkflowBridgeError,
        WorkflowBridgeInput, WorkflowBridgeValidator, WorkflowCatalogueValidation, derive_grade,
    };
    use crate::independent_verifier::{VerificationStatus, report_for_workflow_bridge_test};

    fn validation_for() -> WorkflowCatalogueValidation {
        WorkflowCatalogueValidation::new(
            format!("sha256:{}", "1".repeat(64)),
            format!("sha256:{}", "2".repeat(64)),
            true,
            false,
            false,
            false,
            false,
            true,
            true,
        )
        .unwrap()
    }

    fn input_for(status: VerificationStatus) -> WorkflowBridgeInput {
        let report = report_for_workflow_bridge_test(status);
        WorkflowBridgeInput::new(
            "reduce_repeat_payment_contacts",
            "customer_episode",
            format!("sha256:{}", "e".repeat(64)),
            "customer_id",
            "2026-09-30T00:00:00Z",
            "flow/payment-status",
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
        .expect("test bridge input is valid")
    }

    #[test]
    fn supported_mechanism_is_only_a_proxy_even_when_every_link_is_present() {
        let (grade, missing) = derive_grade(
            LinkEvidence::new(true, true, true, true, true, true, true),
            VerificationStatus::Supported,
        );

        assert_eq!(grade, LinkGrade::MechanismProxy);
        assert!(missing.is_empty());
    }

    #[test]
    fn refuted_evidence_cannot_select_a_candidate_route() {
        let (grade, missing) =
            derive_grade(LinkEvidence::mechanism_proxy(), VerificationStatus::Refuted);

        assert_eq!(grade, LinkGrade::Unlinked);
        assert_eq!(missing, vec![MissingLink::VerificationRefuted]);
    }

    #[test]
    fn uncertain_evidence_is_not_evaluable() {
        let (grade, missing) = derive_grade(
            LinkEvidence::mechanism_proxy(),
            VerificationStatus::Uncertain,
        );

        assert_eq!(grade, LinkGrade::NotEvaluable);
        assert_eq!(missing, vec![MissingLink::VerificationUncertain]);
    }

    #[test]
    fn bridge_retains_u14_scope_contract_and_explicit_do_nothing() {
        let report = report_for_workflow_bridge_test(VerificationStatus::Supported);
        let input = input_for(VerificationStatus::Supported);
        let evidence =
            WorkflowBridgeValidator::validate(&report, &input, validation_for()).unwrap();
        let bridge = WorkflowBridge::assess_verified(&report, input, evidence)
            .expect("bound support references permit assessment");

        assert_eq!(bridge.contract().scope(), report.scope());
        assert_eq!(
            bridge.contract().candidate_digest(),
            report.candidate_digest()
        );
        assert_eq!(
            bridge.contract().source_snapshot_ref(),
            report.source_snapshot_ref()
        );
        assert_eq!(
            bridge.contract().input().target_outcome(),
            "reduce_repeat_payment_contacts"
        );
        assert!(bridge.contract().alternatives().includes_do_nothing());
        assert_eq!(
            bridge.contract().alternatives().candidate_route(),
            Some("flow/payment-status")
        );
    }

    #[test]
    fn bridge_rejects_input_not_bound_to_u14_receipt_and_snapshot() {
        let report = report_for_workflow_bridge_test(VerificationStatus::Supported);
        let input = WorkflowBridgeInput::new(
            "reduce_repeat_payment_contacts",
            "customer_episode",
            format!("sha256:{}", "e".repeat(64)),
            "customer_id",
            "2026-09-30T00:00:00Z",
            "flow/payment-status",
            "mechanism",
            "intervention",
            "effect",
            "oracle",
            vec![format!("sha256:{}", "f".repeat(64))],
        )
        .unwrap();

        assert_eq!(
            WorkflowBridge::assess_verified(
                &report,
                input,
                WorkflowBridgeValidator::validate(
                    &report,
                    &input_for(VerificationStatus::Supported),
                    validation_for(),
                )
                .unwrap(),
            ),
            Err(super::WorkflowBridgeError::MissingVerificationSupportReference)
        );
    }

    #[test]
    fn support_references_are_bounded_and_canonicalized_before_commitment() {
        let report = report_for_workflow_bridge_test(VerificationStatus::Supported);
        let canonical = input_for(VerificationStatus::Supported);
        let reordered = WorkflowBridgeInput::new(
            "reduce_repeat_payment_contacts",
            "customer_episode",
            format!("sha256:{}", "e".repeat(64)),
            "customer_id",
            "2026-09-30T00:00:00Z",
            "flow/payment-status",
            "payment_status_explains_next_step",
            "after_contact_classification",
            "customer_receives_correct_payment_status",
            "scenario_oracle/payment_status_resolution_v1",
            vec![
                report.source_snapshot_ref().digest.clone(),
                report.receipt().digest().to_owned(),
                report.receipt().evidence_commitment().to_owned(),
                report.receipt().digest().to_owned(),
            ],
        )
        .unwrap();
        assert_eq!(canonical, reordered);

        let too_many = (0..33)
            .map(|value| format!("sha256:{value:064x}"))
            .collect();
        assert_eq!(
            WorkflowBridgeInput::new(
                "outcome",
                "unit",
                format!("sha256:{}", "b".repeat(64)),
                "key",
                "cutoff",
                "route",
                "mechanism",
                "point",
                "effect",
                "oracle",
                too_many,
            ),
            Err(WorkflowBridgeError::InvalidInput)
        );
    }

    #[test]
    fn refuted_and_uncertain_reports_keep_do_nothing_without_a_candidate_route() {
        for (status, grade) in [
            (VerificationStatus::Refuted, LinkGrade::Unlinked),
            (VerificationStatus::Uncertain, LinkGrade::NotEvaluable),
        ] {
            let report = report_for_workflow_bridge_test(status);
            let input = input_for(status);
            let evidence =
                WorkflowBridgeValidator::validate(&report, &input, validation_for()).unwrap();
            let bridge = WorkflowBridge::assess_verified(&report, input, evidence).unwrap();

            assert_eq!(bridge.link_grade(), grade);
            assert!(bridge.alternatives().includes_do_nothing());
            assert_eq!(bridge.alternatives().candidate_route(), None);
        }
    }
}
