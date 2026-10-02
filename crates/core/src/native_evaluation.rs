//! Fail-closed admission boundary before a native Agent Core evaluation.
//!
//! This slice owns neither an Agent Core registry nor evaluator transport. It
//! only admits a request from a fresh, trusted registry readback; it never
//! interprets a transport response as a verdict.

use sha2::{Digest, Sha256};

use crate::ArtifactReference;
use crate::core_task::CoreTaskScope;
use crate::evaluation_plan::EvaluationPlan;

/// Public marker for the U19-0 boundary; no public constructor exists.
pub struct NativeEvaluationAdmission {
    _private: (),
}

/// Opaque admission request; it is never an execution or verdict.
pub struct NativeEvaluationRequest {
    commitment: String,
    binding: NativeEvaluationRequestBinding,
}
impl NativeEvaluationRequest {
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }
    #[must_use]
    pub fn attempt_id(&self) -> &str {
        &self.binding.attempt_id
    }
}

#[derive(Clone, Eq, PartialEq)]
struct NativeEvaluationRequestBinding {
    scope: CoreTaskScope,
    attempt_id: String,
    plan_commitment: String,
    candidate: NativeCoreEntityReference,
    candidate_hash: String,
    proposal_id: String,
    base_release_id: String,
    suite: NativeCoreEntityReference,
    evaluator: NativeEvaluatorBinding,
    registry_receipt_id: String,
    registry_revision: u64,
}

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
    InvalidAttempt,
    ScopeMismatch,
    CandidateMismatch,
    SuiteMismatch,
    CapabilityMismatch,
    ReceiptExpired,
    ReceiptRevoked,
    ReadbackMismatch,
    FailedInfrastructure,
}
impl NativeEvaluationAdmissionError {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DependencyUnavailable => "dependency_unavailable",
            Self::InvalidAttempt => "invalid_attempt",
            Self::ScopeMismatch => "scope_mismatch",
            Self::CandidateMismatch => "candidate_mismatch",
            Self::SuiteMismatch => "suite_mismatch",
            Self::CapabilityMismatch => "capability_mismatch",
            Self::ReceiptExpired => "receipt_expired",
            Self::ReceiptRevoked => "receipt_revoked",
            Self::ReadbackMismatch => "readback_mismatch",
            Self::FailedInfrastructure => "failed_infrastructure",
        }
    }
}

/// Exact versioned identity returned by Agent Core's registry, deliberately
/// distinct from a local Pulso ArtifactReference.
#[derive(Clone, Eq, PartialEq)]
struct NativeCoreEntityReference {
    tenant_id: String,
    entity_id: String,
    revision: u64,
    digest: String,
    route: String,
}
#[derive(Clone, Eq, PartialEq)]
struct NativeEvaluatorBinding {
    agent_core_sha: String,
    schema_digest: String,
    evaluator_contract_digest: String,
}

/// Opaque evidence emitted only from a trusted Agent Core registry readback.
/// U18's local receipt does not implement or enter this boundary.
///
/// ```compile_fail
/// use improvement_engine_core::native_evaluation::RegisteredNativeCandidateReceipt;
/// let _ = RegisteredNativeCandidateReceipt {};
/// ```
pub struct RegisteredNativeCandidateReceipt {
    binding: NativeEvaluationRequestBinding,
    expires_at_unix_seconds: u64,
}

/// Private native-registry observation. Public consumers cannot self-attest
/// coherent strings into a candidate receipt.
#[derive(Clone)]
pub(crate) struct NativeRegistryCandidateObservation {
    scope: CoreTaskScope,
    plan_commitment: String,
    proposal_id: String,
    candidate: NativeCoreEntityReference,
    candidate_hash: String,
    base_release_id: String,
    suite: NativeCoreEntityReference,
    evaluator: NativeEvaluatorBinding,
    registry_receipt_id: String,
    registry_revision: u64,
    observed_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
    revoked: bool,
}

/// Required external boundary. Production composition must provide a real
/// Agent Core adapter; absence must surface `DependencyUnavailable`.
pub(crate) trait NativeCandidateRegistryReadbackPort {
    fn read_current(
        &mut self,
        lookup: &NativeCandidateRegistryLookup,
    ) -> Result<NativeRegistryCandidateObservation, NativeEvaluationAdmissionError>;
}

/// ```compile_fail
/// use improvement_engine_core::native_evaluation::NativeCandidateRegistryReadbackPort;
/// let _ = std::any::TypeId::of::<NativeCandidateRegistryReadbackPort>();
/// ```
///
/// An API consumer cannot select or implement the registry adapter; only
/// trusted in-crate composition can connect the future Agent Core boundary.
const _NO_PUBLIC_REGISTRY_ADAPTER: () = ();
#[allow(dead_code)] // Read by a future in-crate Agent Core registry adapter.
pub(crate) struct NativeCandidateRegistryLookup {
    scope: CoreTaskScope,
    plan_commitment: String,
    attempt_id: String,
    expected_candidate: Option<NativeCoreEntityReference>,
    expected_registry_receipt_id: Option<String>,
    expected_registry_revision: Option<u64>,
}

/// Opaque trusted registry readback for one dispatched request. It contains no
/// result/pass/fail field and cannot be created from an HTTP response.
pub struct NativeEvaluationReadback {
    request_commitment: String,
    attempt_id: String,
    observed_eval_id: String,
    binding: NativeEvaluationRequestBinding,
}

/// Trusted in-crate composition only. It exposes no public registry injection,
/// HTTP client or evaluator call.
pub struct TrustedNativeEvaluationComposer {
    _private: (),
}
impl TrustedNativeEvaluationComposer {
    #[allow(dead_code)]
    pub(crate) fn admit_from_registry<R: NativeCandidateRegistryReadbackPort>(
        &self,
        registry: &mut R,
        scope: &CoreTaskScope,
        attempt_id: &str,
        plan: &EvaluationPlan,
    ) -> Result<
        (NativeEvaluationRequest, RegisteredNativeCandidateReceipt),
        NativeEvaluationAdmissionError,
    > {
        if attempt_id.is_empty() {
            return Err(NativeEvaluationAdmissionError::InvalidAttempt);
        }
        let material = plan.native_evaluation_material();
        let observed = registry.read_current(&NativeCandidateRegistryLookup {
            scope: scope.clone(),
            plan_commitment: material.plan_commitment.clone(),
            attempt_id: attempt_id.to_owned(),
            expected_candidate: None,
            expected_registry_receipt_id: None,
            expected_registry_revision: None,
        })?;
        let binding = validate_observation(
            scope,
            attempt_id,
            &material.plan_commitment,
            &material.development_suite,
            observed.clone(),
        )?;
        let request = NativeEvaluationRequest {
            commitment: request_commitment(&binding),
            binding: binding.clone(),
        };
        Ok((
            request,
            RegisteredNativeCandidateReceipt {
                binding,
                expires_at_unix_seconds: observed.expires_at_unix_seconds,
            },
        ))
    }

    /// Re-reads the registry at readback time, so a revoked/expired/stale
    /// candidate cannot be attributed after dispatch.
    #[allow(dead_code)]
    pub(crate) fn validate_readback<R: NativeCandidateRegistryReadbackPort>(
        &self,
        registry: &mut R,
        request: &NativeEvaluationRequest,
        receipt: &RegisteredNativeCandidateReceipt,
        readback: &NativeEvaluationReadback,
    ) -> Result<(), NativeEvaluationAdmissionError> {
        if request.binding != receipt.binding
            || request.commitment != request_commitment(&request.binding)
            || readback.observed_eval_id.is_empty()
            || readback.request_commitment != request.commitment
            || readback.attempt_id != request.binding.attempt_id
            || readback.binding != request.binding
        {
            return Err(NativeEvaluationAdmissionError::ReadbackMismatch);
        }
        let observed = registry.read_current(&NativeCandidateRegistryLookup {
            scope: request.binding.scope.clone(),
            plan_commitment: request.binding.plan_commitment.clone(),
            attempt_id: request.binding.attempt_id.clone(),
            expected_candidate: Some(request.binding.candidate.clone()),
            expected_registry_receipt_id: Some(request.binding.registry_receipt_id.clone()),
            expected_registry_revision: Some(request.binding.registry_revision),
        })?;
        if observed.revoked {
            return Err(NativeEvaluationAdmissionError::ReceiptRevoked);
        }
        if observed.observed_at_unix_seconds >= receipt.expires_at_unix_seconds
            || observed.expires_at_unix_seconds != receipt.expires_at_unix_seconds
        {
            return Err(NativeEvaluationAdmissionError::ReceiptExpired);
        }
        let current = validate_observation(
            &request.binding.scope,
            &request.binding.attempt_id,
            &request.binding.plan_commitment,
            &ArtifactReference {
                tenant_id: request.binding.suite.tenant_id.clone(),
                id: request.binding.suite.entity_id.clone(),
                revision: request.binding.suite.revision,
                digest: request.binding.suite.digest.clone(),
            },
            observed,
        )?;
        if current != request.binding {
            return Err(NativeEvaluationAdmissionError::ReadbackMismatch);
        }
        Ok(())
    }
}

fn validate_observation(
    scope: &CoreTaskScope,
    attempt_id: &str,
    plan_commitment: &str,
    development_suite: &ArtifactReference,
    observed: NativeRegistryCandidateObservation,
) -> Result<NativeEvaluationRequestBinding, NativeEvaluationAdmissionError> {
    if observed.revoked {
        return Err(NativeEvaluationAdmissionError::ReceiptRevoked);
    }
    if observed.observed_at_unix_seconds >= observed.expires_at_unix_seconds {
        return Err(NativeEvaluationAdmissionError::ReceiptExpired);
    }
    if observed.scope != *scope || observed.candidate.tenant_id != scope.tenant_id() {
        return Err(NativeEvaluationAdmissionError::ScopeMismatch);
    }
    if observed.plan_commitment != plan_commitment
        || observed.suite.tenant_id != scope.tenant_id()
        || observed.suite.entity_id != development_suite.id
        || observed.suite.revision != development_suite.revision
        || observed.suite.digest != development_suite.digest
    {
        return Err(NativeEvaluationAdmissionError::SuiteMismatch);
    }
    if !entity_ok(&observed.candidate)
        || !entity_ok(&observed.suite)
        || !digest_ok(&observed.candidate_hash)
        || !digest_ok(&observed.evaluator.agent_core_sha)
        || !digest_ok(&observed.evaluator.schema_digest)
        || !digest_ok(&observed.evaluator.evaluator_contract_digest)
    {
        return Err(NativeEvaluationAdmissionError::CapabilityMismatch);
    }
    if attempt_id.is_empty()
        || observed.registry_receipt_id.is_empty()
        || observed.registry_revision == 0
        || observed.proposal_id.is_empty()
        || observed.base_release_id.is_empty()
    {
        return Err(NativeEvaluationAdmissionError::CandidateMismatch);
    }
    Ok(NativeEvaluationRequestBinding {
        scope: scope.clone(),
        attempt_id: attempt_id.to_owned(),
        plan_commitment: observed.plan_commitment,
        candidate: observed.candidate,
        candidate_hash: observed.candidate_hash,
        proposal_id: observed.proposal_id,
        base_release_id: observed.base_release_id,
        suite: observed.suite,
        evaluator: observed.evaluator,
        registry_receipt_id: observed.registry_receipt_id,
        registry_revision: observed.registry_revision,
    })
}
fn entity_ok(e: &NativeCoreEntityReference) -> bool {
    !e.tenant_id.is_empty()
        && !e.entity_id.is_empty()
        && e.revision > 0
        && !e.route.is_empty()
        && digest_ok(&e.digest)
}
fn request_commitment(b: &NativeEvaluationRequestBinding) -> String {
    digest(&[
        b.scope.tenant_id(),
        b.scope.job_id(),
        b.scope.grant_id(),
        b.scope.authority_ref(),
        &b.attempt_id,
        &b.plan_commitment,
        &b.candidate.tenant_id,
        &b.candidate.entity_id,
        &b.candidate.revision.to_string(),
        &b.candidate.digest,
        &b.candidate.route,
        &b.candidate_hash,
        &b.proposal_id,
        &b.base_release_id,
        &b.suite.tenant_id,
        &b.suite.entity_id,
        &b.suite.revision.to_string(),
        &b.suite.digest,
        &b.suite.route,
        &b.evaluator.agent_core_sha,
        &b.evaluator.schema_digest,
        &b.evaluator.evaluator_contract_digest,
        &b.registry_receipt_id,
        &b.registry_revision.to_string(),
    ])
}
fn digest(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.len().to_be_bytes());
        h.update(p.as_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}
fn digest_ok(v: &str) -> bool {
    v.len() == 71
        && v.starts_with("sha256:")
        && v.as_bytes()[7..]
            .iter()
            .all(|b| matches!(*b,b'0'..=b'9'|b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;
    const D: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn scope() -> CoreTaskScope {
        CoreTaskScope::new("tenant-a", "job-a", "grant-a", "authority-a").unwrap()
    }
    fn reference(id: &str) -> ArtifactReference {
        ArtifactReference {
            tenant_id: "tenant-a".into(),
            id: id.into(),
            revision: 1,
            digest: D.into(),
        }
    }
    fn plan() -> EvaluationPlan {
        crate::evaluation_plan::plan_for_native_evaluation_test(
            reference("00000000-0000-7000-8000-000000000001"),
            reference("00000000-0000-7000-8000-000000000002"),
        )
    }
    fn entity(id: &str, route: &str) -> NativeCoreEntityReference {
        NativeCoreEntityReference {
            tenant_id: "tenant-a".into(),
            entity_id: id.into(),
            revision: 1,
            digest: D.into(),
            route: route.into(),
        }
    }
    #[derive(Clone)]
    struct Registry {
        record: NativeRegistryCandidateObservation,
        unavailable: bool,
    }
    impl NativeCandidateRegistryReadbackPort for Registry {
        fn read_current(
            &mut self,
            l: &NativeCandidateRegistryLookup,
        ) -> Result<NativeRegistryCandidateObservation, NativeEvaluationAdmissionError> {
            if self.unavailable {
                return Err(NativeEvaluationAdmissionError::DependencyUnavailable);
            };
            if l.scope != self.record.scope
                || l.plan_commitment != self.record.plan_commitment
                || l.attempt_id.is_empty()
                || l.expected_candidate
                    .as_ref()
                    .is_some_and(|x| x != &self.record.candidate)
                || l.expected_registry_receipt_id
                    .as_ref()
                    .is_some_and(|x| x != &self.record.registry_receipt_id)
                || l.expected_registry_revision
                    .is_some_and(|x| x != self.record.registry_revision)
            {
                return Err(NativeEvaluationAdmissionError::CandidateMismatch);
            };
            Ok(self.record.clone())
        }
    }
    fn registry(p: &EvaluationPlan) -> Registry {
        let m = p.native_evaluation_material();
        Registry {
            unavailable: false,
            record: NativeRegistryCandidateObservation {
                scope: scope(),
                plan_commitment: m.plan_commitment,
                proposal_id: "proposal-a".into(),
                candidate: entity("candidate-a", "agent"),
                candidate_hash: D.into(),
                base_release_id: "base-a".into(),
                suite: NativeCoreEntityReference {
                    tenant_id: "tenant-a".into(),
                    entity_id: m.development_suite.id,
                    revision: m.development_suite.revision,
                    digest: m.development_suite.digest,
                    route: "suite".into(),
                },
                evaluator: NativeEvaluatorBinding {
                    agent_core_sha: D.into(),
                    schema_digest: D.into(),
                    evaluator_contract_digest: D.into(),
                },
                registry_receipt_id: "registry-receipt-a".into(),
                registry_revision: 7,
                observed_at_unix_seconds: 100,
                expires_at_unix_seconds: 200,
                revoked: false,
            },
        }
    }
    fn readback(r: &NativeEvaluationRequest) -> NativeEvaluationReadback {
        NativeEvaluationReadback {
            request_commitment: r.commitment.clone(),
            attempt_id: r.binding.attempt_id.clone(),
            observed_eval_id: "eval-a".into(),
            binding: r.binding.clone(),
        }
    }
    #[test]
    fn red_boundary_is_invalid_attempt_or_missing_registry() {
        let p = plan();
        let c = TrustedNativeEvaluationComposer { _private: () };
        assert!(matches!(
            c.admit_from_registry(&mut registry(&p), &scope(), "", &p),
            Err(NativeEvaluationAdmissionError::InvalidAttempt)
        ));
        let mut r = registry(&p);
        r.unavailable = true;
        assert!(matches!(
            c.admit_from_registry(&mut r, &scope(), "attempt-a", &p),
            Err(NativeEvaluationAdmissionError::DependencyUnavailable)
        ));
    }
    #[test]
    fn registry_issued_receipt_binds_full_request_and_readback() {
        let p = plan();
        let c = TrustedNativeEvaluationComposer { _private: () };
        let mut r = registry(&p);
        let (q, receipt) = c
            .admit_from_registry(&mut r, &scope(), "attempt-a", &p)
            .unwrap();
        assert_eq!(
            c.validate_readback(&mut r, &q, &receipt, &readback(&q)),
            Ok(())
        );
    }
    #[test]
    fn readback_rejects_cross_attempt_capability_plan_and_scope() {
        let p = plan();
        let c = TrustedNativeEvaluationComposer { _private: () };
        let mut r = registry(&p);
        let (q, receipt) = c
            .admit_from_registry(&mut r, &scope(), "attempt-a", &p)
            .unwrap();
        let mut b = readback(&q);
        b.attempt_id = "attempt-b".into();
        assert_eq!(
            c.validate_readback(&mut r, &q, &receipt, &b),
            Err(NativeEvaluationAdmissionError::ReadbackMismatch)
        );
        let mut b = readback(&q);
        b.binding.evaluator.schema_digest =
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        assert_eq!(
            c.validate_readback(&mut r, &q, &receipt, &b),
            Err(NativeEvaluationAdmissionError::ReadbackMismatch)
        );
        let mut b = readback(&q);
        b.binding.plan_commitment =
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into();
        assert_eq!(
            c.validate_readback(&mut r, &q, &receipt, &b),
            Err(NativeEvaluationAdmissionError::ReadbackMismatch)
        );
        let mut b = readback(&q);
        b.binding.scope =
            CoreTaskScope::new("tenant-b", "job-a", "grant-a", "authority-a").unwrap();
        assert_eq!(
            c.validate_readback(&mut r, &q, &receipt, &b),
            Err(NativeEvaluationAdmissionError::ReadbackMismatch)
        );
    }
    #[test]
    fn registry_revoke_or_expiry_after_admission_fails_closed() {
        let p = plan();
        let c = TrustedNativeEvaluationComposer { _private: () };
        let mut r = registry(&p);
        let (q, receipt) = c
            .admit_from_registry(&mut r, &scope(), "attempt-a", &p)
            .unwrap();
        r.record.revoked = true;
        assert_eq!(
            c.validate_readback(&mut r, &q, &receipt, &readback(&q)),
            Err(NativeEvaluationAdmissionError::ReceiptRevoked)
        );
        let mut r = registry(&p);
        r.record.observed_at_unix_seconds = 200;
        assert_eq!(
            c.validate_readback(&mut r, &q, &receipt, &readback(&q)),
            Err(NativeEvaluationAdmissionError::ReceiptExpired)
        );
    }
}
