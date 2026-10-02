//! Fail-closed admission boundary before a native Agent Core evaluation.
//!
//! This module deliberately does not implement an Agent Core runtime, HTTP
//! client, evaluator, or sandbox adapter. It can only seal a request after
//! trusted composition supplies independently registered native artifacts.

use sha2::{Digest, Sha256};

use crate::ArtifactReference;
use crate::core_task::CoreTaskScope;
use crate::evaluation_plan::EvaluationPlan;

/// Public marker for the U19-0 boundary; it has no public constructor.
pub struct NativeEvaluationAdmission {
    _private: (),
}

/// A request is intentionally opaque: it proves admission, not execution.
pub struct NativeEvaluationRequest {
    commitment: String,
    attempt_id: String,
}

impl NativeEvaluationRequest {
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }

    #[must_use]
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
}

/// A dispatch must remain truthful until an independently attributable readback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeEvaluationDispatchState {
    NotDispatched,
    UnknownAfterDispatch,
}

impl NativeEvaluationDispatchState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotDispatched => "not_dispatched",
            Self::UnknownAfterDispatch => "unknown_after_dispatch",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeEvaluationAdmissionError {
    DependencyUnavailable,
    ScopeMismatch,
    CandidateMismatch,
    SuiteMismatch,
    CapabilityMismatch,
    ReadbackMismatch,
    FailedInfrastructure,
}

impl NativeEvaluationAdmissionError {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DependencyUnavailable => "dependency_unavailable",
            Self::ScopeMismatch => "scope_mismatch",
            Self::CandidateMismatch => "candidate_mismatch",
            Self::SuiteMismatch => "suite_mismatch",
            Self::CapabilityMismatch => "capability_mismatch",
            Self::ReadbackMismatch => "readback_mismatch",
            Self::FailedInfrastructure => "failed_infrastructure",
        }
    }
}

/// Only trusted composition can issue this after native registry readback.
///
/// ```compile_fail
/// use improvement_engine_core::native_evaluation::RegisteredNativeCandidateReceipt;
/// let _ = RegisteredNativeCandidateReceipt {};
/// ```
#[allow(dead_code)] // Only a future trusted registry adapter may issue it.
pub struct RegisteredNativeCandidateReceipt {
    scope: CoreTaskScope,
    proposal_id: String,
    candidate_hash: String,
    base_release_id: String,
    agent_id: String,
    suite_id: String,
    suite_version: String,
    suite_digest: String,
    plan_commitment: String,
    source_snapshot: ArtifactReference,
}

/// Only trusted composition can issue this from the pinned Agent Core manifest.
///
/// ```compile_fail
/// use improvement_engine_core::native_evaluation::NativeEvaluatorCapability;
/// let _ = NativeEvaluatorCapability {};
/// ```
#[allow(dead_code)] // Only a pinned external evaluator manifest may issue it.
pub struct NativeEvaluatorCapability {
    agent_core_sha: String,
    schema_digest: String,
    evaluator_contract_digest: String,
    agent_id: String,
}

/// Public-suite binding only. Final/holdout suite and oracle never have a slot here.
///
/// ```compile_fail
/// use improvement_engine_core::native_evaluation::NativeSuiteBinding;
/// let _ = NativeSuiteBinding {};
/// ```
#[allow(dead_code)] // Only trusted suite registration may issue it.
pub struct NativeSuiteBinding {
    suite_id: String,
    suite_version: String,
    suite_digest: String,
    source_reference: ArtifactReference,
}

/// Trusted projection of a registry readback. It intentionally has no verdict:
/// admission validates attribution before a later boundary interprets a report.
#[allow(dead_code)]
pub struct NativeEvaluationReadback {
    observed_eval_id: String,
    proposal_id: String,
    candidate_hash: String,
    base_release_id: String,
    suite_id: String,
    suite_version: String,
}

/// Opaque trusted composition. A future real adapter may be installed here;
/// this slice intentionally has no public factory and no transport method.
pub struct TrustedNativeEvaluationComposer {
    _private: (),
}

impl TrustedNativeEvaluationComposer {
    #[allow(clippy::too_many_arguments, dead_code)]
    pub(crate) fn admit(
        &self,
        scope: &CoreTaskScope,
        attempt_id: &str,
        plan: &EvaluationPlan,
        candidate: &RegisteredNativeCandidateReceipt,
        suite: &NativeSuiteBinding,
        capability: &NativeEvaluatorCapability,
    ) -> Result<NativeEvaluationRequest, NativeEvaluationAdmissionError> {
        let material = plan.native_evaluation_material();
        if attempt_id.is_empty() {
            return Err(NativeEvaluationAdmissionError::DependencyUnavailable);
        }
        if candidate.scope != *scope || candidate.source_snapshot != material.source_snapshot {
            return Err(NativeEvaluationAdmissionError::ScopeMismatch);
        }
        if candidate.plan_commitment != material.plan_commitment {
            return Err(NativeEvaluationAdmissionError::CandidateMismatch);
        }
        if candidate.suite_id != suite.suite_id
            || candidate.suite_version != suite.suite_version
            || candidate.suite_digest != suite.suite_digest
            || suite.source_reference != material.development_suite
        {
            return Err(NativeEvaluationAdmissionError::SuiteMismatch);
        }
        if capability.agent_id != candidate.agent_id
            || !digest_ok(&capability.agent_core_sha)
            || !digest_ok(&capability.schema_digest)
            || !digest_ok(&capability.evaluator_contract_digest)
        {
            return Err(NativeEvaluationAdmissionError::CapabilityMismatch);
        }
        Ok(NativeEvaluationRequest {
            commitment: digest(&[
                scope.tenant_id(),
                scope.job_id(),
                scope.grant_id(),
                scope.authority_ref(),
                attempt_id,
                &candidate.proposal_id,
                &candidate.candidate_hash,
                &candidate.base_release_id,
                &candidate.agent_id,
                &candidate.suite_id,
                &candidate.suite_version,
                &candidate.suite_digest,
                &candidate.plan_commitment,
                &candidate.source_snapshot.tenant_id,
                &candidate.source_snapshot.id,
                &candidate.source_snapshot.revision.to_string(),
                &candidate.source_snapshot.digest,
                &suite.source_reference.tenant_id,
                &suite.source_reference.id,
                &suite.source_reference.revision.to_string(),
                &suite.source_reference.digest,
                &capability.agent_core_sha,
                &capability.schema_digest,
                &capability.evaluator_contract_digest,
            ]),
            attempt_id: attempt_id.to_owned(),
        })
    }

    #[allow(dead_code)] // Invoked by the future trusted dispatch/readback adapter.
    /// A 200 response cannot call this method. The caller must supply a
    /// newly-observed registry readback with all admission bindings.
    pub(crate) fn validate_readback(
        &self,
        request: &NativeEvaluationRequest,
        candidate: &RegisteredNativeCandidateReceipt,
        readback: &NativeEvaluationReadback,
    ) -> Result<(), NativeEvaluationAdmissionError> {
        if readback.observed_eval_id.is_empty()
            || request.attempt_id.is_empty()
            || readback.proposal_id != candidate.proposal_id
            || readback.candidate_hash != candidate.candidate_hash
            || readback.base_release_id != candidate.base_release_id
            || readback.suite_id != candidate.suite_id
            || readback.suite_version != candidate.suite_version
        {
            return Err(NativeEvaluationAdmissionError::ReadbackMismatch);
        }
        Ok(())
    }
}

#[allow(dead_code)]
fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.len().to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[allow(dead_code)]
fn digest_ok(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn scope() -> CoreTaskScope {
        CoreTaskScope::new("tenant-a", "job-a", "grant-a", "authority-a").unwrap()
    }

    fn reference(id: &str) -> ArtifactReference {
        ArtifactReference {
            tenant_id: "tenant-a".into(),
            id: id.into(),
            revision: 1,
            digest: DIGEST.into(),
        }
    }

    // The U20 test composer is deliberately crate-private; this local helper
    // only proves U19 admission against a plan-shaped sealed fixture.
    fn plan() -> EvaluationPlan {
        crate::evaluation_plan::plan_for_native_evaluation_test(
            reference("00000000-0000-7000-8000-000000000001"),
            reference("00000000-0000-7000-8000-000000000002"),
        )
    }

    fn candidate(plan: &EvaluationPlan) -> RegisteredNativeCandidateReceipt {
        let material = plan.native_evaluation_material();
        RegisteredNativeCandidateReceipt {
            scope: scope(),
            proposal_id: "proposal-a".into(),
            candidate_hash: DIGEST.into(),
            base_release_id: "release-base".into(),
            agent_id: "agent-a".into(),
            suite_id: "suite-a".into(),
            suite_version: "1.0.0".into(),
            suite_digest: DIGEST.into(),
            plan_commitment: material.plan_commitment,
            source_snapshot: material.source_snapshot,
        }
    }

    fn suite(plan: &EvaluationPlan) -> NativeSuiteBinding {
        NativeSuiteBinding {
            suite_id: "suite-a".into(),
            suite_version: "1.0.0".into(),
            suite_digest: DIGEST.into(),
            source_reference: plan.native_evaluation_material().development_suite,
        }
    }

    fn capability() -> NativeEvaluatorCapability {
        NativeEvaluatorCapability {
            agent_core_sha: DIGEST.into(),
            schema_digest: DIGEST.into(),
            evaluator_contract_digest: DIGEST.into(),
            agent_id: "agent-a".into(),
        }
    }

    fn readback(eval_id: &str) -> NativeEvaluationReadback {
        NativeEvaluationReadback {
            observed_eval_id: eval_id.into(),
            proposal_id: "proposal-a".into(),
            candidate_hash: DIGEST.into(),
            base_release_id: "release-base".into(),
            suite_id: "suite-a".into(),
            suite_version: "1.0.0".into(),
        }
    }

    #[test]
    fn seals_only_registered_candidate_with_pinned_capability_and_public_suite() {
        let plan = plan();
        let candidate = candidate(&plan);
        let suite = suite(&plan);
        let request = TrustedNativeEvaluationComposer { _private: () }
            .admit(
                &scope(),
                "attempt-a",
                &plan,
                &candidate,
                &suite,
                &capability(),
            )
            .unwrap();
        assert!(request.commitment().starts_with("sha256:"));
    }

    #[test]
    fn request_commitment_changes_with_the_registered_candidate_identity() {
        let plan = plan();
        let suite = suite(&plan);
        let first = candidate(&plan);
        let mut changed = candidate(&plan);
        changed.candidate_hash =
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into();
        let composer = TrustedNativeEvaluationComposer { _private: () };
        let first = composer
            .admit(&scope(), "attempt-a", &plan, &first, &suite, &capability())
            .unwrap();
        let changed = composer
            .admit(
                &scope(),
                "attempt-a",
                &plan,
                &changed,
                &suite,
                &capability(),
            )
            .unwrap();
        assert_ne!(first.commitment(), changed.commitment());
    }

    #[test]
    fn mismatched_suite_or_capability_fails_before_any_dispatch() {
        let plan = plan();
        let candidate = candidate(&plan);
        let mut wrong_suite = suite(&plan);
        wrong_suite.suite_version = "2.0.0".into();
        assert!(matches!(
            TrustedNativeEvaluationComposer { _private: () }.admit(
                &scope(),
                "attempt-a",
                &plan,
                &candidate,
                &wrong_suite,
                &capability()
            ),
            Err(NativeEvaluationAdmissionError::SuiteMismatch)
        ));
        let mut capability = capability();
        capability.agent_id = "other".into();
        assert!(matches!(
            TrustedNativeEvaluationComposer { _private: () }.admit(
                &scope(),
                "attempt-a",
                &plan,
                &candidate,
                &suite(&plan),
                &capability
            ),
            Err(NativeEvaluationAdmissionError::CapabilityMismatch)
        ));
    }

    #[test]
    fn stale_candidate_plan_or_cross_scope_fails_closed() {
        let plan = plan();
        let mut stale_candidate = candidate(&plan);
        stale_candidate.plan_commitment = "sha256:stale".into();
        assert!(matches!(
            TrustedNativeEvaluationComposer { _private: () }.admit(
                &scope(),
                "attempt-a",
                &plan,
                &stale_candidate,
                &suite(&plan),
                &capability()
            ),
            Err(NativeEvaluationAdmissionError::CandidateMismatch)
        ));
        let candidate = candidate(&plan);
        let other_scope =
            CoreTaskScope::new("tenant-b", "job-a", "grant-a", "authority-a").unwrap();
        assert!(matches!(
            TrustedNativeEvaluationComposer { _private: () }.admit(
                &other_scope,
                "attempt-a",
                &plan,
                &candidate,
                &suite(&plan),
                &capability()
            ),
            Err(NativeEvaluationAdmissionError::ScopeMismatch)
        ));
    }

    #[test]
    fn http_success_cannot_replace_exact_readback() {
        let plan = plan();
        let candidate = candidate(&plan);
        let suite = suite(&plan);
        let composer = TrustedNativeEvaluationComposer { _private: () };
        let request = composer
            .admit(
                &scope(),
                "attempt-a",
                &plan,
                &candidate,
                &suite,
                &capability(),
            )
            .unwrap();
        assert_eq!(
            composer.validate_readback(&request, &candidate, &readback("")),
            Err(NativeEvaluationAdmissionError::ReadbackMismatch)
        );
        assert!(
            composer
                .validate_readback(&request, &candidate, &readback("eval-new"))
                .is_ok()
        );
    }
}
