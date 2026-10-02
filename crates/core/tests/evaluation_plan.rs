//! Public compile-surface coverage for U20. Behavioral sealing remains a
//! crate-private unit test because production callers must not be able to mint
//! a U16 `MechanismProxy` bridge as a test fixture.

use improvement_engine_core::ArtifactReference;
use improvement_engine_core::core_task::CoreTaskScope;
use improvement_engine_core::evaluation_plan::{
    EvaluationArtifactAuthorityPort, EvaluationArtifactRef, EvaluationAuthorityError,
    EvaluationInputs,
};

struct DenyAuthority;

impl EvaluationArtifactAuthorityPort for DenyAuthority {
    fn verify_artifact(
        &mut self,
        _: &CoreTaskScope,
        _: &ArtifactReference,
    ) -> Result<(), EvaluationAuthorityError> {
        Err(EvaluationAuthorityError::DependencyUnavailable)
    }
}

fn reference(id: &str) -> ArtifactReference {
    ArtifactReference {
        tenant_id: "tenant_a".to_owned(),
        id: id.to_owned(),
        revision: 1,
        digest: format!("sha256:{}", "a".repeat(64)),
    }
}

#[test]
fn public_typed_inputs_and_independent_authority_port_are_composable() {
    let inputs = EvaluationInputs::new(
        EvaluationArtifactRef::baseline(reference("018f0f4e-7bbd-7000-8000-000000000501")),
        EvaluationArtifactRef::oracle(reference("018f0f4e-7bbd-7000-8000-000000000502")),
        EvaluationArtifactRef::development_suite(reference("018f0f4e-7bbd-7000-8000-000000000503")),
        EvaluationArtifactRef::final_suite(reference("018f0f4e-7bbd-7000-8000-000000000504")),
    );
    let scope = CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap();
    let mut authority = DenyAuthority;
    assert_eq!(
        authority.verify_artifact(&scope, &reference("018f0f4e-7bbd-7000-8000-000000000501")),
        Err(EvaluationAuthorityError::DependencyUnavailable)
    );
    assert!(std::mem::size_of_val(&inputs) > 0);
}
