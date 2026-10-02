//! U20-E seals the E0 safety context before any future evaluator can use it.
//!
//! This is deliberately not an evaluation runner, an identity service, or an
//! Agent Core integration. It joins already-verified U04-B replay timing, U20
//! exact immutable inputs, and U36 protected-fixture commitments. No identity
//! answer, principal, proof nonce, customer state or candidate output crosses
//! this boundary.

use sha2::{Digest, Sha256};

use crate::enriched_history::VerifiedReplayAvailability;
use crate::evaluation_plan::{EvaluationPlan, EvaluationPlanBinding};
use crate::sandbox::{ProtectedFixtureIdentityBinding, SandboxFixture};

/// Opaque, immutable safety context for one E0 evaluation plan. Possessing it
/// does not execute a sandbox action, assert a successful outcome, or allow a
/// candidate to be published or released.
#[derive(Clone, Eq, PartialEq)]
pub struct E0SafetyOracle {
    commitment: String,
    tenant_id: String,
    source_snapshot_digest: String,
    availability_profile_digest: String,
    cutoff_at_unix_seconds: u64,
    fixture_binding: ProtectedFixtureIdentityBinding,
    plan_binding: EvaluationPlanBinding,
}

impl E0SafetyOracle {
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }

    /// U20-E does not itself authorize execution. A later evaluator must
    /// consume this context alongside its own sandbox/runtime fences.
    #[must_use]
    pub fn authorizes_execution_or_release(&self) -> bool {
        false
    }

    /// Revalidates an already sealed context against fresh, trusted U04-B,
    /// U20 and U36 inputs. It has no effects and fails closed if any exact
    /// reference, profile, cutoff or commitment has drifted.
    #[allow(dead_code)] // Called by the later U19/U23 trusted evaluation composition.
    pub(crate) fn revalidate(
        &self,
        replay: &VerifiedReplayAvailability,
        plan: &EvaluationPlan,
        fixture: &SandboxFixture,
    ) -> Result<(), E0SafetyOracleError> {
        let current = TrustedE0SafetyOracleComposer::seal(replay, plan, fixture)?;
        if current.commitment == self.commitment {
            Ok(())
        } else {
            Err(E0SafetyOracleError::ContextMismatch)
        }
    }

    /// Applies a private, typed observation emitted by a later U36-aware
    /// evaluator. Unknown and unsafe observations never become safe merely
    /// because the sealed context exists.
    #[must_use]
    #[allow(dead_code)] // Called by the later U19/U23 trusted evaluation composition.
    pub(crate) fn classify_identity_check(
        &self,
        check: ExpectedIdentityCheck,
    ) -> SafetyDisposition {
        match check {
            ExpectedIdentityCheck::Verified => SafetyDisposition::Safe,
            ExpectedIdentityCheck::Missing
            | ExpectedIdentityCheck::Mismatch
            | ExpectedIdentityCheck::Expired
            | ExpectedIdentityCheck::Revoked => SafetyDisposition::Unsafe,
            ExpectedIdentityCheck::Unavailable => SafetyDisposition::Unknown,
        }
    }
}

/// Only trusted in-crate wiring may compose this oracle from the three sealed
/// upstream boundaries. There is intentionally no public constructor or policy
/// trait that a consumer could satisfy with an allow-all implementation.
#[allow(dead_code)] // Installed only by trusted U20-E service composition.
pub(crate) struct TrustedE0SafetyOracleComposer;

/// ```compile_fail
/// use improvement_engine_core::e0_safety_oracle::E0SafetyOracle;
/// let _ = E0SafetyOracle { commitment: String::new() };
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::e0_safety_oracle::E0SafetyOracle;
/// fn requires_debug<T: std::fmt::Debug>() {}
/// requires_debug::<E0SafetyOracle>();
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::e0_safety_oracle::TrustedE0SafetyOracleComposer;
/// let _ = TrustedE0SafetyOracleComposer;
/// ```
///
/// An API consumer cannot manufacture either the context or the trusted
/// composition boundary from policy strings, artifact digests or fixtures.
const _NO_PUBLIC_E0_SAFETY_COMPOSITION: () = ();

impl TrustedE0SafetyOracleComposer {
    #[allow(dead_code)] // Called by trusted U20-E service composition.
    pub(crate) fn seal(
        replay: &VerifiedReplayAvailability,
        plan: &EvaluationPlan,
        fixture: &SandboxFixture,
    ) -> Result<E0SafetyOracle, E0SafetyOracleError> {
        let fixture_binding = fixture.protected_identity_binding()?;
        let plan_binding = plan.e0_safety_binding();
        validate_bindings(replay, &plan_binding, &fixture_binding)?;
        let commitment = commitment(replay, &plan_binding, &fixture_binding);
        Ok(E0SafetyOracle {
            commitment,
            tenant_id: replay.tenant_id().to_owned(),
            source_snapshot_digest: replay.source_snapshot_digest().to_owned(),
            availability_profile_digest: replay.availability_profile_digest().to_owned(),
            cutoff_at_unix_seconds: replay.cutoff_at_unix_seconds(),
            fixture_binding,
            plan_binding,
        })
    }
}

/// Internal interpretation of a U36-aware identity check. It has no public
/// constructor and intentionally contains no answer, prompt, principal or
/// proof material.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Produced by a later U36-aware evaluator, never public input.
pub(crate) enum ExpectedIdentityCheck {
    Verified,
    Missing,
    Mismatch,
    Expired,
    Revoked,
    Unavailable,
}

/// A safety verdict is distinct from an evaluation or release result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SafetyDisposition {
    Safe,
    Unsafe,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum E0SafetyOracleError {
    MissingProtectedFixture,
    TenantMismatch,
    SnapshotMismatch,
    FixtureBindingInvalid,
    InvalidPlanReference,
    PolicyExpiredAtReplayCutoff,
    ContextMismatch,
}

impl From<crate::sandbox::SandboxError> for E0SafetyOracleError {
    fn from(value: crate::sandbox::SandboxError) -> Self {
        match value {
            crate::sandbox::SandboxError::IdentityEvidenceMissing => Self::MissingProtectedFixture,
            _ => Self::FixtureBindingInvalid,
        }
    }
}

#[allow(dead_code)] // Reached from the crate-private trusted composer.
fn validate_bindings(
    replay: &VerifiedReplayAvailability,
    plan: &EvaluationPlanBinding,
    fixture: &ProtectedFixtureIdentityBinding,
) -> Result<(), E0SafetyOracleError> {
    if fixture.tenant_id() != replay.tenant_id()
        || plan.source_snapshot_ref().tenant_id != replay.tenant_id()
        || [
            plan.baseline_ref(),
            plan.oracle_ref(),
            plan.development_suite_ref(),
            plan.final_suite_ref(),
        ]
        .iter()
        .any(|reference| reference.tenant_id != replay.tenant_id())
    {
        return Err(E0SafetyOracleError::TenantMismatch);
    }
    if plan.source_snapshot_binding_digest() != replay.source_snapshot_digest() {
        return Err(E0SafetyOracleError::SnapshotMismatch);
    }
    if plan.commitment().is_empty()
        || plan.source_snapshot_ref().id.is_empty()
        || plan.source_snapshot_ref().revision == 0
        || [
            plan.baseline_ref(),
            plan.oracle_ref(),
            plan.development_suite_ref(),
            plan.final_suite_ref(),
        ]
        .iter()
        .any(|reference| {
            reference.id.is_empty() || reference.revision == 0 || reference.digest.is_empty()
        })
    {
        return Err(E0SafetyOracleError::InvalidPlanReference);
    }
    if fixture.valid_until() <= replay.cutoff_at_unix_seconds() {
        return Err(E0SafetyOracleError::PolicyExpiredAtReplayCutoff);
    }
    Ok(())
}

#[allow(dead_code)] // Reached from the crate-private trusted composer.
fn commitment(
    replay: &VerifiedReplayAvailability,
    plan: &EvaluationPlanBinding,
    fixture: &ProtectedFixtureIdentityBinding,
) -> String {
    let mut hasher = Sha256::new();
    let cutoff = replay.cutoff_at_unix_seconds().to_string();
    let valid_until = fixture.valid_until().to_string();
    for part in [
        replay.tenant_id(),
        replay.world_ref(),
        cutoff.as_str(),
        replay.source_snapshot_digest(),
        replay.availability_profile_digest(),
        fixture.tenant_id(),
        fixture.namespace(),
        fixture.fixture_id(),
        fixture.case_id(),
        fixture.channel(),
        fixture.policy_digest(),
        fixture.questions_digest(),
        valid_until.as_str(),
        plan.commitment(),
        plan.source_snapshot_binding_digest(),
    ] {
        hasher.update(part.len().to_be_bytes());
        hasher.update(part.as_bytes());
    }
    for reference in [
        plan.source_snapshot_ref(),
        plan.baseline_ref(),
        plan.oracle_ref(),
        plan.development_suite_ref(),
        plan.final_suite_ref(),
    ] {
        let revision = reference.revision.to_string();
        for part in [
            &reference.tenant_id,
            &reference.id,
            &revision,
            &reference.digest,
        ] {
            hasher.update(part.len().to_be_bytes());
            hasher.update(part.as_bytes());
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::ArtifactReference;
    use crate::enriched_history::{
        AvailabilityClockMode, AvailabilityProfile, EnrichedHistoryAdapter,
        EnrichedHistoryManifest, PackageFile, ProvenanceDigests, ReplayRowAvailability,
        replay_projection_digest, verified_replay_availability_fixture,
    };
    use crate::evaluation_plan::tests::real_e0_plan_and_snapshot;
    use crate::evaluation_plan::{
        e0_safety_plan_fixture, e0_safety_plan_fixture_with_oracle_revision,
    };
    use crate::sandbox::SandboxIdentityPolicy;

    fn source_ref(tenant_id: &str, digest_marker: char) -> ArtifactReference {
        ArtifactReference {
            tenant_id: tenant_id.to_owned(),
            id: "018f0f4e-7bbd-7000-8000-000000000700".to_owned(),
            revision: 1,
            digest: format!("sha256:{}", digest_marker.to_string().repeat(64)),
        }
    }

    fn digest(marker: char) -> String {
        format!("sha256:{}", marker.to_string().repeat(64))
    }

    fn replay(tenant_id: &str, snapshot_digest: &str, cutoff: u64) -> VerifiedReplayAvailability {
        verified_replay_availability_fixture(
            tenant_id,
            "e0-world-1",
            cutoff,
            snapshot_digest,
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
    }

    fn fixture(tenant_id: &str, policy: &str, questions: &str, valid_until: u64) -> SandboxFixture {
        SandboxFixture::with_identity_policy(
            tenant_id,
            "e0-evaluation",
            "identity-check-fixture",
            BTreeMap::from([("account_status".to_owned(), "open".to_owned())]),
            BTreeSet::from(["verify_identity".to_owned()]),
            SandboxIdentityPolicy::new(
                "case-17",
                "chat",
                policy,
                questions,
                BTreeSet::from(["synthetic-principal".to_owned()]),
                valid_until,
            ),
        )
    }

    fn plan(tenant_id: &str, source: ArtifactReference) -> EvaluationPlan {
        // U20-E only needs the exact binding. Tests build the U20 sealed shape
        // through the module-local test fixture, preserving production opacity.
        e0_safety_plan_fixture(tenant_id, source)
    }

    #[test]
    fn seals_exact_u04_u20_u36_context_without_execution_authority() {
        let source = source_ref("tenant_a", 'a');
        let oracle = TrustedE0SafetyOracleComposer::seal(
            &replay("tenant_a", &source.digest, 100),
            &plan("tenant_a", source),
            &fixture("tenant_a", "policy-v1", "questions-v1", 101),
        )
        .expect("matching sealed context");
        assert!(!oracle.authorizes_execution_or_release());
        assert_eq!(
            oracle.classify_identity_check(ExpectedIdentityCheck::Verified),
            SafetyDisposition::Safe
        );
    }

    #[test]
    fn rejects_cross_context_snapshot_tenant_and_expired_policy_before_any_runner_exists() {
        let source = source_ref("tenant_a", 'a');
        assert!(matches!(
            TrustedE0SafetyOracleComposer::seal(
                &replay(
                    "tenant_a",
                    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    100
                ),
                &plan("tenant_a", source.clone()),
                &fixture("tenant_a", "policy-v1", "questions-v1", 101),
            ),
            Err(E0SafetyOracleError::SnapshotMismatch)
        ));
        assert!(matches!(
            TrustedE0SafetyOracleComposer::seal(
                &replay("tenant_a", &source.digest, 100),
                &plan("tenant_a", source),
                &fixture("tenant_b", "policy-v1", "questions-v1", 101),
            ),
            Err(E0SafetyOracleError::TenantMismatch)
        ));
        let source = source_ref("tenant_a", 'a');
        assert!(matches!(
            TrustedE0SafetyOracleComposer::seal(
                &replay("tenant_a", &source.digest, 100),
                &plan("tenant_a", source),
                &fixture("tenant_a", "policy-v1", "questions-v1", 100),
            ),
            Err(E0SafetyOracleError::PolicyExpiredAtReplayCutoff)
        ));
    }

    #[test]
    fn profile_policy_questions_and_plan_reference_drift_change_or_reject_the_sealed_context() {
        let source = source_ref("tenant_a", 'a');
        let plan = plan("tenant_a", source.clone());
        let base_fixture = fixture("tenant_a", "policy-v1", "questions-v1", 101);
        let oracle = TrustedE0SafetyOracleComposer::seal(
            &replay("tenant_a", &source.digest, 100),
            &plan,
            &base_fixture,
        )
        .unwrap();
        assert_eq!(
            oracle.revalidate(
                &replay("tenant_a", &source.digest, 100),
                &plan,
                &fixture("tenant_a", "policy-v2", "questions-v1", 101),
            ),
            Err(E0SafetyOracleError::ContextMismatch)
        );
        assert_eq!(
            oracle.revalidate(
                &replay("tenant_a", &source.digest, 99),
                &plan,
                &base_fixture,
            ),
            Err(E0SafetyOracleError::ContextMismatch)
        );
        assert_eq!(
            oracle.revalidate(
                &replay("tenant_a", &source.digest, 100),
                &plan,
                &fixture("tenant_a", "policy-v1", "questions-v2", 101),
            ),
            Err(E0SafetyOracleError::ContextMismatch)
        );
        assert_eq!(
            oracle.revalidate(
                &replay("tenant_a", &source.digest, 100),
                &e0_safety_plan_fixture_with_oracle_revision("tenant_a", source.clone(), 99,),
                &base_fixture,
            ),
            Err(E0SafetyOracleError::ContextMismatch)
        );
        assert_eq!(
            oracle.revalidate(
                &verified_replay_availability_fixture(
                    "tenant_a",
                    "e0-world-1",
                    100,
                    &source.digest,
                    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ),
                &plan,
                &base_fixture,
            ),
            Err(E0SafetyOracleError::ContextMismatch)
        );
    }

    #[test]
    fn unsafe_and_unknown_identity_observations_fail_closed() {
        let source = source_ref("tenant_a", 'a');
        let oracle = TrustedE0SafetyOracleComposer::seal(
            &replay("tenant_a", &source.digest, 100),
            &plan("tenant_a", source),
            &fixture("tenant_a", "policy-v1", "questions-v1", 101),
        )
        .unwrap();
        for check in [
            ExpectedIdentityCheck::Missing,
            ExpectedIdentityCheck::Mismatch,
            ExpectedIdentityCheck::Expired,
            ExpectedIdentityCheck::Revoked,
        ] {
            assert_eq!(
                oracle.classify_identity_check(check),
                SafetyDisposition::Unsafe
            );
        }
        assert_eq!(
            oracle.classify_identity_check(ExpectedIdentityCheck::Unavailable),
            SafetyDisposition::Unknown
        );
    }

    #[test]
    fn u20_binding_carries_a_distinct_u04_source_snapshot_seal() {
        let source = source_ref("tenant_a", 'a');
        let plan = plan("tenant_a", source);
        assert_eq!(
            plan.e0_safety_binding().source_snapshot_binding_digest(),
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn real_u04b_replay_and_real_u20_plan_bind_the_same_source_byte_seal() {
        let (plan, snapshot) = real_e0_plan_and_snapshot();
        let row = serde_json::json!({
            "event_time": "1970-01-01T00:01:40Z",
            "status": "completed"
        });
        let availability = vec![ReplayRowAvailability::new(BTreeMap::from([
            ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
            ("status".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
        ]))];
        let profile = AvailabilityProfile::new(
            "e0_replay",
            1,
            AvailabilityClockMode::replay_at_event_time("e0_zero_lag"),
            "tenant_a",
            snapshot.binding_digest(),
        );
        let manifest = EnrichedHistoryManifest::new_replay(
            "platform_history",
            "world_a",
            "1970-01-01T00:01:40Z",
            profile,
            vec![
                PackageFile::new(
                    "case",
                    ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                    "1970-01-01T00:01:40Z",
                )
                .with_field_availability(BTreeMap::from([
                    ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                    ("status".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                ]))
                .with_replay_projection_digest(replay_projection_digest(
                    std::slice::from_ref(&row),
                    &availability,
                ))
                .with_source_file_seal(snapshot.source_file_seal("case").unwrap()),
            ],
        );
        let adapter = EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot).unwrap();
        let replay = adapter.verified_replay_availability(&snapshot).unwrap();
        assert_ne!(
            plan.e0_safety_binding().source_snapshot_ref().digest,
            replay.source_snapshot_digest()
        );
        assert_eq!(
            plan.e0_safety_binding().source_snapshot_binding_digest(),
            replay.source_snapshot_digest()
        );
        let fixture = fixture("tenant_a", "policy-v1", "questions-v1", 101);
        assert!(TrustedE0SafetyOracleComposer::seal(&replay, &plan, &fixture).is_ok());
    }
}
