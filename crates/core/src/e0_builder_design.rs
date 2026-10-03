//! Versioned, Pulso-owned input/output envelope for the E0 builder seam.
//!
//! This is an internal design contract, not an Agent Core DTO or permission.
//! A valid model response can only describe a proposal against the exact
//! digest-sealed input that Pulso supplied; it cannot choose tenant, run,
//! snapshot, base/version, capability, grant, or execution authority.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::e0_mechanism_resolution::{E0MechanismEvidencePacket, RouteResolution};
use crate::e0_proposal_assembly::LocalProposalCandidate;
use crate::e0_safety_oracle::E0SafetyOracle;
use crate::evaluation_plan::EvaluationPlan;

/// Stable adapter version. Change this when the Pulso-owned input/output
/// contract changes; it is deliberately independent of Agent Core's wire DTO.
pub const E0_BUILDER_DESIGN_CONTRACT_VERSION: &str = "pulso.e0_builder_design.v1";

/// A restricted, non-executable model response. Unknown fields (including
/// proposed authority or Core artifact fields) fail deserialization.
///
/// ```compile_fail
/// use improvement_engine_core::change_compiler::ChangeCompiler;
/// use improvement_engine_core::e0_builder_design::E0BuilderDesignOutcome;
/// fn cannot_submit_to_compiler(intent: E0BuilderDesignOutcome) {
///     let _ = ChangeCompiler::compile(intent);
/// }
/// ```
///
/// The design-intent type deliberately has no conversion to `UntrustedChangeSpec`
/// or `AuthorizedChangeSpec`; a separate U35-gated trusted adapter is required.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UntrustedDesignIntent {
    pub input_digest: String,
    pub disposition: DesignIntentDisposition,
    pub title: String,
    pub problem_statement: String,
    pub proposed_behavior: String,
    pub rationale: String,
    pub expected_effect: String,
    pub tests_to_add: Vec<String>,
    pub evidence_digests: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignIntentDisposition {
    ReviewRequired,
    DoNothing,
}

/// Accepted output remains a review/design artifact only. There is no variant
/// representing an executable change, a native proposal, or Core evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum E0BuilderDesignOutcome {
    ReviewRequired(UntrustedDesignIntent),
    DoNothing(UntrustedDesignIntent),
}

/// Privacy-minimized evidence identity sent to the model. This carries no raw
/// customer/contact text and does not itself establish statistical support.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0BuilderEvidence {
    pub metric_id: String,
    pub signal_digest: String,
    pub summary_commitment: String,
    pub numerator: u64,
    pub denominator: u64,
    pub missing: u64,
    pub claim_level: String,
}

/// Immutable, digest-sealed exploratory design input. Construction is private
/// to trusted crate composition; this is not a Core builder request or grant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0BuilderDesignInput {
    contract_version: String,
    #[serde(skip_serializing)]
    tenant_id: String,
    #[serde(skip_serializing)]
    source_run_id: String,
    source_snapshot_id: String,
    source_snapshot_revision: u64,
    source_snapshot_digest: String,
    observed_cutoff_rfc3339: String,
    u20_cutoff_at_unix_seconds: u64,
    evidence: E0BuilderEvidence,
    route_status: BuilderRouteStatus,
    route_catalog_digest: Option<String>,
    #[serde(skip_serializing)]
    u20_plan_commitment: String,
    #[serde(skip_serializing)]
    u20_safety_commitment: String,
    input_digest: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BuilderRouteStatus {
    Unavailable,
    Unlinked,
    CatalogDeclared,
}

impl E0BuilderDesignInput {
    fn digest(&self) -> String {
        let bytes = serde_json::to_vec(&(
            &self.contract_version,
            &self.tenant_id,
            &self.source_run_id,
            &self.source_snapshot_id,
            self.source_snapshot_revision,
            &self.source_snapshot_digest,
            &self.observed_cutoff_rfc3339,
            self.u20_cutoff_at_unix_seconds,
            &self.evidence,
            &self.route_status,
            &self.route_catalog_digest,
            &self.u20_plan_commitment,
            &self.u20_safety_commitment,
        ))
        .expect("builder input fields serialize infallibly");
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    #[must_use]
    pub fn input_digest(&self) -> &str {
        &self.input_digest
    }

    #[must_use]
    pub fn source_run_id(&self) -> &str {
        &self.source_run_id
    }

    #[must_use]
    pub fn source_snapshot_digest(&self) -> &str {
        &self.source_snapshot_digest
    }

    #[must_use]
    pub fn evidence(&self) -> &E0BuilderEvidence {
        &self.evidence
    }

    /// Digest consistency detects accidental mutation/serialization drift. It
    /// is not a signature and does not confer Core authority.
    #[must_use]
    pub fn validate_integrity(&self) -> bool {
        self.input_digest == self.digest()
    }

    /// Parse and bind an untrusted model response to this exact input. A
    /// `ReviewRequired` outcome is still only a design intent; this method
    /// never decides whether Core authoring or compilation is eligible.
    pub fn validate_model_response(
        &self,
        response_json: &str,
    ) -> Result<E0BuilderDesignOutcome, E0BuilderDesignError> {
        if !self.validate_integrity() {
            return Err(E0BuilderDesignError::InputIntegrityMismatch);
        }
        let response: UntrustedDesignIntent = serde_json::from_str(response_json)
            .map_err(|_| E0BuilderDesignError::MalformedModelResponse)?;
        if response.input_digest != self.input_digest {
            return Err(E0BuilderDesignError::ResponseInputMismatch);
        }
        if [
            &response.title,
            &response.problem_statement,
            &response.proposed_behavior,
            &response.rationale,
            &response.expected_effect,
        ]
        .iter()
        .any(|value| value.trim().is_empty() || value.len() > 8_000)
            || response.tests_to_add.is_empty()
            || response.tests_to_add.len() > 32
            || response
                .tests_to_add
                .iter()
                .any(|test| test.trim().is_empty() || test.len() > 500)
        {
            return Err(E0BuilderDesignError::InvalidRationale);
        }
        let mut evidence = response.evidence_digests.clone();
        evidence.sort();
        evidence.dedup();
        if evidence.len() != response.evidence_digests.len()
            || evidence.iter().any(|digest| {
                digest != &self.evidence.signal_digest
                    && digest != &self.evidence.summary_commitment
                    && digest != &self.source_snapshot_digest
            })
        {
            return Err(E0BuilderDesignError::EvidenceOutsideInput);
        }
        if response.disposition == DesignIntentDisposition::ReviewRequired
            && response.evidence_digests.is_empty()
        {
            return Err(E0BuilderDesignError::ProposalWithoutEvidence);
        }
        Ok(match response.disposition {
            DesignIntentDisposition::ReviewRequired => {
                E0BuilderDesignOutcome::ReviewRequired(response)
            }
            DesignIntentDisposition::DoNothing => E0BuilderDesignOutcome::DoNothing(response),
        })
    }
}

/// Outcome from trusted preparation. Missing candidate or U20 context blocks
/// even exploratory model use. An unlinked route is still eligible for a
/// design-only request; every response remains review-only and non-executable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum E0BuilderPreparation {
    Ready(Box<E0BuilderDesignInput>),
    NonExecutable(E0BuilderBlocker),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum E0BuilderBlocker {
    NoCandidate,
    EvidenceMismatch,
    MissingU20Plan,
    MissingU20SafetyOracle,
    ReadinessMismatch,
}

/// Trusted in-crate composition for exploratory design only. It consumes
/// verified E0 evidence and U20 plan/safety context, but intentionally does not
/// require U35 or an exact route. The resulting intent cannot reach compiler,
/// registry, evaluator, or Agent Core APIs.
#[allow(dead_code)] // Installed by the trusted E0→U17 service composition.
pub(crate) fn prepare_e0_builder_design(
    candidate: Option<&LocalProposalCandidate>,
    evidence: Option<&E0MechanismEvidencePacket>,
    route: Option<&RouteResolution>,
    plan: Option<&EvaluationPlan>,
    safety_oracle: Option<&E0SafetyOracle>,
) -> E0BuilderPreparation {
    let (Some(candidate), Some(evidence)) = (candidate, evidence) else {
        return E0BuilderPreparation::NonExecutable(E0BuilderBlocker::NoCandidate);
    };
    if candidate.claim_level != "descriptive_only"
        || candidate.business_lift.is_some()
        || candidate.native_agent_core_status != "not_connected"
        || candidate.evaluation_status != "not_evaluated"
        || candidate.proposal_ref != evidence.proposal_ref()
        || candidate.source_run_id != evidence.source_run_id()
        || candidate.metric_id != evidence.metric_id()
        || candidate.signal_digest != evidence.signal_digest()
        || candidate.summary_commitment != evidence.summary_commitment()
        || candidate.source_snapshot_ref.id != evidence.source_snapshot_ref().id
        || candidate.source_snapshot_ref.revision != evidence.source_snapshot_ref().revision
        || candidate.source_snapshot_ref.digest != evidence.source_snapshot_ref().digest
        || candidate.observed_cutoff_rfc3339 != evidence.observed_cutoff_rfc3339()
        || candidate_digest(candidate) != evidence.candidate_digest()
        || evidence.claim_level() != "descriptive_only"
    {
        return E0BuilderPreparation::NonExecutable(E0BuilderBlocker::EvidenceMismatch);
    }
    let Some(plan) = plan else {
        return E0BuilderPreparation::NonExecutable(E0BuilderBlocker::MissingU20Plan);
    };
    let Some(safety_oracle) = safety_oracle else {
        return E0BuilderPreparation::NonExecutable(E0BuilderBlocker::MissingU20SafetyOracle);
    };
    let snapshot = plan.source_snapshot_ref_for_builder();
    if !safety_oracle.is_bound_to_plan(plan)
        || rfc3339_utc_to_unix_seconds(evidence.observed_cutoff_rfc3339())
            != Some(safety_oracle.cutoff_at_unix_seconds())
        || snapshot.tenant_id != evidence.tenant_scope()
        || snapshot.id != candidate.source_snapshot_ref.id
        || snapshot.revision != candidate.source_snapshot_ref.revision
        || snapshot.digest != candidate.source_snapshot_ref.digest
    {
        return E0BuilderPreparation::NonExecutable(E0BuilderBlocker::ReadinessMismatch);
    }
    if route.is_some_and(|route| !route.is_bound_to(evidence)) {
        return E0BuilderPreparation::NonExecutable(E0BuilderBlocker::EvidenceMismatch);
    }
    let route_status = route.map_or(BuilderRouteStatus::Unavailable, |route| {
        match route.status() {
            "mapped" => BuilderRouteStatus::CatalogDeclared,
            _ => BuilderRouteStatus::Unlinked,
        }
    });
    let route_catalog_digest = route.map(|route| route.catalog_ref().digest().to_owned());
    if route_status == BuilderRouteStatus::CatalogDeclared
        && route.and_then(RouteResolution::flow_ref).is_none()
    {
        return E0BuilderPreparation::NonExecutable(E0BuilderBlocker::EvidenceMismatch);
    }
    let mut input = E0BuilderDesignInput {
        contract_version: E0_BUILDER_DESIGN_CONTRACT_VERSION.to_owned(),
        tenant_id: evidence.tenant_scope().to_owned(),
        source_run_id: evidence.source_run_id().to_owned(),
        source_snapshot_id: snapshot.id.clone(),
        source_snapshot_revision: snapshot.revision,
        source_snapshot_digest: snapshot.digest.clone(),
        observed_cutoff_rfc3339: evidence.observed_cutoff_rfc3339().to_owned(),
        u20_cutoff_at_unix_seconds: safety_oracle.cutoff_at_unix_seconds(),
        evidence: E0BuilderEvidence {
            metric_id: evidence.metric_id().to_owned(),
            signal_digest: evidence.signal_digest().to_owned(),
            summary_commitment: evidence.summary_commitment().to_owned(),
            numerator: evidence.numerator(),
            denominator: evidence.denominator(),
            missing: evidence.missing(),
            claim_level: evidence.claim_level().to_owned(),
        },
        route_status,
        route_catalog_digest,
        u20_plan_commitment: plan.commitment().to_owned(),
        u20_safety_commitment: safety_oracle.commitment().to_owned(),
        input_digest: String::new(),
    };
    input.input_digest = input.digest();
    E0BuilderPreparation::Ready(Box::new(input))
}

#[allow(dead_code)] // Used by the deferred trusted preparation composition.
fn candidate_digest(candidate: &LocalProposalCandidate) -> String {
    let bytes = serde_json::to_vec(candidate).expect("E0 candidate serializes infallibly");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[allow(dead_code)] // Used by the deferred trusted preparation composition.
fn rfc3339_utc_to_unix_seconds(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let text = std::str::from_utf8(&bytes[range]).ok()?;
        if !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        text.parse().ok()
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    if !(1..=12).contains(&month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=month_days).contains(&day) {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days_since_epoch = era * 146_097 + day_of_era - 719_468;
    u64::try_from(days_since_epoch * 86_400 + hour * 3_600 + minute * 60 + second).ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E0BuilderDesignError {
    InputIntegrityMismatch,
    MalformedModelResponse,
    ResponseInputMismatch,
    InvalidRationale,
    EvidenceOutsideInput,
    ProposalWithoutEvidence,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArtifactReference;
    use crate::e0_mechanism_resolution::{
        E0RouteCatalog, E0RouteMapping, SupportedCoreFlowRef, resolve_e0_mechanism_route,
    };
    use crate::e0_proposal_assembly::assemble_e0_proposals;
    use crate::enriched_history::verified_replay_availability_fixture;
    use crate::local_simulation::{
        LocalObservedQuery, LocalRunInput, LocalRunMetadata, LocalSourceKind, run_local_simulation,
    };
    use crate::sandbox::{SandboxFixture, SandboxIdentityPolicy};
    use std::collections::{BTreeMap, BTreeSet};

    const TENANT: &str = "tenant_a";
    const E0_CUTOFF: u64 = 1_775_001_600;

    fn trusted_plan_and_safety() -> (EvaluationPlan, E0SafetyOracle) {
        let (plan, source) = crate::evaluation_plan::tests::real_e0_plan_and_snapshot();
        let replay = verified_replay_availability_fixture(
            TENANT,
            "e0-builder-design-test-world",
            E0_CUTOFF,
            &source.binding_digest(),
            &format!("sha256:{}", "a".repeat(64)),
        );
        let fixture = SandboxFixture::with_identity_policy(
            TENANT,
            "e0-evaluation",
            "identity-check-fixture",
            BTreeMap::from([("account_status".to_owned(), "open".to_owned())]),
            BTreeSet::from(["verify_identity".to_owned()]),
            SandboxIdentityPolicy::new(
                "case-17",
                "chat",
                format!("sha256:{}", "b".repeat(64)),
                format!("sha256:{}", "c".repeat(64)),
                BTreeSet::from(["synthetic-principal".to_owned()]),
                E0_CUTOFF + 1,
            ),
        );
        let safety =
            crate::e0_safety_oracle::TrustedE0SafetyOracleComposer::seal(&replay, &plan, &fixture)
                .expect("actual U20-E composer seals matching test inputs");
        (plan, safety)
    }

    fn e0_candidate() -> (LocalProposalCandidate, E0MechanismEvidencePacket) {
        e0_candidate_with_snapshot(ArtifactReference {
            tenant_id: TENANT.into(),
            id: "018f0f4e-7bbd-7000-8000-000000000600".into(),
            revision: 1,
            digest: format!("sha256:{}", "a".repeat(64)),
        })
    }

    fn e0_candidate_with_snapshot(
        snapshot: ArtifactReference,
    ) -> (LocalProposalCandidate, E0MechanismEvidencePacket) {
        let run = run_local_simulation(
            LocalRunInput::new(
                LocalRunMetadata::new(
                    "run_123_1775000000",
                    TENANT,
                    LocalSourceKind::E0,
                    format!("sha256:{}", "b".repeat(64)),
                    snapshot,
                    E0_CUTOFF,
                    "2026-04-01T00:00:00Z",
                ),
                (1..=20).collect(),
                0,
                Vec::new(),
            )
            .with_queries(
                (1..=20)
                    .map(|ordinal| {
                        LocalObservedQuery::new(
                            ordinal,
                            format!("sha256_{}", "a".repeat(56)),
                            format!("2026-03-01T00:00:{:02}Z", ordinal % 60),
                        )
                    })
                    .collect(),
            ),
        )
        .expect("simulated E0 run");
        let candidate = assemble_e0_proposals(&run)
            .expect("proposal assembly")
            .candidates
            .into_iter()
            .find(|candidate| candidate.metric_id == "e0_recurring_copilot_query_cases")
            .expect("recurring-query candidate");
        let signal = run
            .signals
            .iter()
            .find(|signal| signal.metric_id == candidate.metric_id)
            .expect("source signal");
        let evidence = E0MechanismEvidencePacket::from_candidate(&run, &candidate, signal)
            .expect("candidate-bound packet");
        (candidate, evidence)
    }

    fn mapped_route(evidence: &E0MechanismEvidencePacket) -> RouteResolution {
        let flow = SupportedCoreFlowRef::new(
            "86a767474042a566a0dbd6ed23588959f27ebdb3",
            "flow/payment-status",
            "1.0.0",
            format!("sha256:{}", "e".repeat(64)),
        )
        .expect("valid supported flow reference");
        let mapping = E0RouteMapping::new(
            evidence.metric_id(),
            evidence.pattern_ref(),
            "flow/payment-status",
            "payment_status_explains_next_step",
            flow,
        )
        .expect("valid E0 mapping");
        let catalog = E0RouteCatalog::new(
            ArtifactReference {
                tenant_id: TENANT.into(),
                id: "0199b21c-7eab-7000-8000-000000000203".into(),
                revision: 1,
                digest: E0RouteCatalog::content_digest(std::slice::from_ref(&mapping)),
            },
            vec![mapping],
        )
        .expect("valid route catalog");
        resolve_e0_mechanism_route(evidence, &catalog)
    }

    fn input() -> E0BuilderDesignInput {
        let mut value = E0BuilderDesignInput {
            contract_version: E0_BUILDER_DESIGN_CONTRACT_VERSION.into(),
            tenant_id: "tenant_test".into(),
            source_run_id: "run_2026_01".into(),
            source_snapshot_id: "snapshot-1".into(),
            source_snapshot_revision: 3,
            source_snapshot_digest: format!("sha256:{}", "a".repeat(64)),
            observed_cutoff_rfc3339: "2026-10-03T00:00:00Z".into(),
            u20_cutoff_at_unix_seconds: 1_791_000_000,
            evidence: E0BuilderEvidence {
                metric_id: "e0_recurring_copilot_query_cases".into(),
                signal_digest: format!("sha256:{}", "b".repeat(64)),
                summary_commitment: format!("sha256:{}", "c".repeat(64)),
                numerator: 12,
                denominator: 20,
                missing: 0,
                claim_level: "descriptive_only".into(),
            },
            route_status: BuilderRouteStatus::CatalogDeclared,
            route_catalog_digest: Some(format!("sha256:{}", "d".repeat(64))),
            u20_plan_commitment: format!("sha256:{}", "f".repeat(64)),
            u20_safety_commitment: format!("sha256:{}", "1".repeat(64)),
            input_digest: String::new(),
        };
        value.input_digest = value.digest();
        value
    }

    fn response_json(input: &E0BuilderDesignInput) -> String {
        serde_json::json!({
            "input_digest": input.input_digest(),
            "disposition": "review_required",
            "title": "Candidate follow-up path",
            "problem_statement": "The descriptive signal indicates repeated unresolved exploration.",
            "proposed_behavior": "Explore a bounded next-step explanation for evaluation.",
            "rationale": "The bounded recurrence evidence may justify a test candidate.",
            "expected_effect": "Reduce repeated exploration if later evaluation supports it.",
            "tests_to_add": ["same-run replay preserves the evidence binding"],
            "evidence_digests": [input.evidence().signal_digest],
        })
        .to_string()
    }

    #[test]
    fn accepts_only_response_bound_to_exact_input_and_evidence() {
        let input = input();
        let response = input
            .validate_model_response(&response_json(&input))
            .expect("valid bounded response");
        assert!(matches!(
            response,
            E0BuilderDesignOutcome::ReviewRequired(_)
        ));
        assert!(input.validate_integrity());
        // This adapter intentionally has no executable/proposal/evaluation
        // outcome: a validated design still awaits trusted authoring and gates.
    }

    #[test]
    fn no_candidate_and_missing_u20_readiness_never_call_the_model() {
        assert_eq!(
            prepare_e0_builder_design(None, None, None, None, None),
            E0BuilderPreparation::NonExecutable(E0BuilderBlocker::NoCandidate)
        );
        let (candidate, evidence) = e0_candidate();
        let mapped = mapped_route(&evidence);
        assert_eq!(
            prepare_e0_builder_design(Some(&candidate), Some(&evidence), Some(&mapped), None, None,),
            E0BuilderPreparation::NonExecutable(E0BuilderBlocker::MissingU20Plan)
        );
    }

    #[test]
    fn unlinked_evidence_with_real_u20_context_can_be_sent_for_review_only_exploration() {
        let (plan, safety) = trusted_plan_and_safety();
        assert!(
            safety.is_bound_to_plan(&plan),
            "U20-E must bind to U20's parsed-source binding digest, not artifact-content digest"
        );
        let (candidate, evidence) =
            e0_candidate_with_snapshot(plan.source_snapshot_ref_for_builder().clone());
        let empty_catalog = E0RouteCatalog::empty(ArtifactReference {
            tenant_id: TENANT.into(),
            id: "0199b21c-7eab-7000-8000-000000000205".into(),
            revision: 1,
            digest: E0RouteCatalog::content_digest(&[]),
        })
        .expect("empty fixture catalog");
        let unlinked = resolve_e0_mechanism_route(&evidence, &empty_catalog);
        let prepared = prepare_e0_builder_design(
            Some(&candidate),
            Some(&evidence),
            Some(&unlinked),
            Some(&plan),
            Some(&safety),
        );
        let E0BuilderPreparation::Ready(input) = prepared else {
            panic!("U20-ready unlinked evidence should allow design-only exploration");
        };
        assert_eq!(input.route_status, BuilderRouteStatus::Unlinked);
        let serialized = serde_json::to_value(&input).expect("design input serializes");
        assert_eq!(serialized["route_status"], "unlinked");
        assert!(serialized.get("flow_ref").is_none());
        assert!(serialized.get("flow_id").is_none());
        assert!(input.validate_integrity());
        let outcome = input
            .validate_model_response(&response_json(&input))
            .expect("supplied response may validate as a bounded review-only design intent");
        assert!(matches!(outcome, E0BuilderDesignOutcome::ReviewRequired(_)));
        // No executable variant, UntrustedChangeSpec, Core write, or evaluator
        // exists at this boundary; a later U35-gated U17 path is separate.
    }

    #[test]
    fn catalog_declared_route_exposes_no_flow_identity_to_model() {
        let (plan, safety) = trusted_plan_and_safety();
        let (candidate, evidence) =
            e0_candidate_with_snapshot(plan.source_snapshot_ref_for_builder().clone());
        let declared = mapped_route(&evidence);
        let prepared = prepare_e0_builder_design(
            Some(&candidate),
            Some(&evidence),
            Some(&declared),
            Some(&plan),
            Some(&safety),
        );
        let E0BuilderPreparation::Ready(input) = prepared else {
            panic!("catalog-declared route should remain an exploratory hint");
        };
        let serialized = serde_json::to_value(&input).expect("design input serializes");
        assert_eq!(serialized["route_status"], "catalog_declared");
        assert!(serialized.get("route_catalog_digest").is_some());
        assert!(serialized.get("flow_ref").is_none());
        assert!(serialized.get("flow_id").is_none());
        assert!(serialized.get("flow_version").is_none());
        assert!(!serialized.to_string().contains("flow/payment-status"));
        assert!(input.validate_integrity());
    }

    #[test]
    fn rejects_cross_run_or_changed_input_replay() {
        let input = input();
        let mut another_run = input.clone();
        another_run.source_run_id = "run_other".into();
        another_run.input_digest = another_run.digest();
        assert!(matches!(
            another_run.validate_model_response(&response_json(&input)),
            Err(E0BuilderDesignError::ResponseInputMismatch)
        ));
        let mut changed = input.clone();
        changed.evidence.numerator += 1;
        assert!(!changed.validate_integrity());
    }

    #[test]
    fn malformed_and_authority_bearing_model_outputs_are_rejected() {
        let input = input();
        assert!(matches!(
            input.validate_model_response("not json"),
            Err(E0BuilderDesignError::MalformedModelResponse)
        ));
        let mut flow_output: serde_json::Value =
            serde_json::from_str(&response_json(&input)).unwrap();
        flow_output["flow_ref"] = serde_json::json!({"flow_id": "flow/payment-status"});
        assert!(matches!(
            input.validate_model_response(&flow_output.to_string()),
            Err(E0BuilderDesignError::MalformedModelResponse)
        ));
        let mut authority_output: serde_json::Value =
            serde_json::from_str(&response_json(&input)).unwrap();
        authority_output["grant"] = serde_json::json!({"allowed": true});
        authority_output["base_release"] = serde_json::json!("release/production");
        authority_output["version"] = serde_json::json!("999.0.0");
        assert!(matches!(
            input.validate_model_response(&authority_output.to_string()),
            Err(E0BuilderDesignError::MalformedModelResponse)
        ));
    }

    #[test]
    fn rejects_unreferenced_evidence_and_empty_proposal_evidence() {
        let input = input();
        let mut untrusted: serde_json::Value =
            serde_json::from_str(&response_json(&input)).unwrap();
        untrusted["evidence_digests"] = serde_json::json!([format!("sha256:{}", "9".repeat(64))]);
        assert!(matches!(
            input.validate_model_response(&untrusted.to_string()),
            Err(E0BuilderDesignError::EvidenceOutsideInput)
        ));
        untrusted["evidence_digests"] = serde_json::json!([]);
        assert!(matches!(
            input.validate_model_response(&untrusted.to_string()),
            Err(E0BuilderDesignError::ProposalWithoutEvidence)
        ));
    }

    #[test]
    fn accepts_non_executable_do_nothing_as_an_honest_model_outcome() {
        let input = input();
        let mut response: serde_json::Value = serde_json::from_str(&response_json(&input)).unwrap();
        response["disposition"] = serde_json::json!("do_nothing");
        response["evidence_digests"] = serde_json::json!([]);
        assert!(matches!(
            input.validate_model_response(&response.to_string()),
            Ok(E0BuilderDesignOutcome::DoNothing(_))
        ));
    }

    #[test]
    fn model_serialization_excludes_tenant_run_and_readiness_commitments() {
        let input = input();
        let json = serde_json::to_value(input).expect("safe model input serializes");
        let text = json.to_string();
        assert!(!text.contains("tenant_test"));
        assert!(!text.contains("run_2026_01"));
        assert!(!text.contains(&"f".repeat(64)));
        assert!(!text.contains(&"1".repeat(64)));
        assert!(!text.contains("u35_bridge_commitment"));
    }
}
