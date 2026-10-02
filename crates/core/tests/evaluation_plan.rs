use improvement_engine_core::ArtifactKind;
use improvement_engine_core::evaluation_plan::{
    EvaluationArtifactRef, EvaluationInputs, EvaluationPlan, EvaluationPlanError,
};
use improvement_engine_core::workflow_bridge::test_support::{
    mechanism_proxy_bridge, non_evaluable_bridge,
};
use improvement_engine_core::{
    ArtifactDraft, ArtifactRepository, InMemoryArtifactRepository, RepositoryError,
};
use serde_json::json;

struct ReadOnlyRepo {
    inner: InMemoryArtifactRepository,
}
impl ArtifactRepository for ReadOnlyRepo {
    fn append(
        &mut self,
        _: Option<u64>,
        _: ArtifactDraft,
    ) -> Result<ArtifactDraft, RepositoryError> {
        panic!("sealing must not append or mutate artifacts")
    }
    fn get(
        &mut self,
        tenant: &str,
        id: &str,
        revision: u64,
    ) -> Result<Option<ArtifactDraft>, RepositoryError> {
        self.inner.get(tenant, id, revision)
    }
}

#[allow(clippy::too_many_arguments)] // Fixture mirrors the sealed wire contract explicitly.
fn artifact(
    repo: &mut InMemoryArtifactRepository,
    id: &str,
    artifact_type: &str,
    partition: &str,
    outcome: &str,
    unit: &str,
    measure: &str,
    snapshot: improvement_engine_core::ArtifactReference,
) -> improvement_engine_core::ArtifactReference {
    repo.append(
        None,
        ArtifactDraft::new(
            "tenant_a",
            id,
            1,
            ArtifactKind::ScenarioSet,
            json!({"evaluation_contract": {
                "artifact_type": artifact_type,
                "partition": partition,
                "target_outcome": outcome,
                "unit_of_analysis": unit,
                "oracle_measure": measure,
                "scope": {"tenant_id":"tenant_a","job_id":"job_a","grant_id":"grant_a","authority_ref":"authority_a"}
            }}),
            Some(snapshot),
        ),
    )
    .unwrap()
    .reference()
}

fn inputs(
    repo: &mut InMemoryArtifactRepository,
    snapshot: improvement_engine_core::ArtifactReference,
    outcome: &str,
    unit: &str,
    measure: &str,
) -> EvaluationInputs {
    inputs_named(
        repo,
        snapshot,
        outcome,
        unit,
        measure,
        [
            "018f0f4e-7bbd-7000-8000-000000000201",
            "018f0f4e-7bbd-7000-8000-000000000202",
            "018f0f4e-7bbd-7000-8000-000000000203",
            "018f0f4e-7bbd-7000-8000-000000000204",
        ],
    )
}

fn inputs_named(
    repo: &mut InMemoryArtifactRepository,
    snapshot: improvement_engine_core::ArtifactReference,
    outcome: &str,
    unit: &str,
    measure: &str,
    ids: [&str; 4],
) -> EvaluationInputs {
    EvaluationInputs::new(
        EvaluationArtifactRef::baseline(artifact(
            repo,
            ids[0],
            "baseline",
            "shared",
            outcome,
            unit,
            measure,
            snapshot.clone(),
        )),
        EvaluationArtifactRef::oracle(artifact(
            repo,
            ids[1],
            "oracle",
            "shared",
            outcome,
            unit,
            measure,
            snapshot.clone(),
        )),
        EvaluationArtifactRef::development_suite(artifact(
            repo,
            ids[2],
            "development_suite",
            "development",
            outcome,
            unit,
            measure,
            snapshot.clone(),
        )),
        EvaluationArtifactRef::final_suite(artifact(
            repo,
            ids[3],
            "final_suite",
            "final",
            outcome,
            unit,
            measure,
            snapshot,
        )),
    )
}

#[test]
fn seals_public_verified_inputs_without_claiming_outcome_or_eligibility() {
    let mut repo = InMemoryArtifactRepository::default();
    let snapshot = repo
        .append(
            None,
            ArtifactDraft::new(
                "tenant_a",
                "018f0f4e-7bbd-7000-8000-000000000200",
                1,
                ArtifactKind::SourceSnapshot,
                json!({}),
                None,
            ),
        )
        .unwrap()
        .reference();
    let bridge = mechanism_proxy_bridge(snapshot.clone());
    let inputs = inputs(
        &mut repo,
        snapshot,
        "reduce_repeat_payment_contacts",
        "customer_episode",
        "scenario_oracle/payment_status_resolution_v1",
    );
    let mut readonly = ReadOnlyRepo { inner: repo };
    let plan = EvaluationPlan::seal_from_bridge(&bridge, inputs, &mut readonly).unwrap();
    assert!(plan.commitment().starts_with("sha256:"));
    assert!(!plan.allows_same_outcome_claim());
    assert!(!plan.eligible_for_proposal());
}

#[test]
fn rejects_non_proxy_bridge_and_semantic_or_partition_mismatches_without_mutation() {
    let mut repo = InMemoryArtifactRepository::default();
    let snapshot = repo
        .append(
            None,
            ArtifactDraft::new(
                "tenant_a",
                "018f0f4e-7bbd-7000-8000-000000000210",
                1,
                ArtifactKind::SourceSnapshot,
                json!({}),
                None,
            ),
        )
        .unwrap()
        .reference();
    let inputs = inputs(
        &mut repo,
        snapshot.clone(),
        "wrong_outcome",
        "customer_episode",
        "scenario_oracle/payment_status_resolution_v1",
    );
    assert_eq!(
        EvaluationPlan::seal_from_bridge(
            &mechanism_proxy_bridge(snapshot.clone()),
            inputs.clone(),
            &mut repo
        ),
        Err(EvaluationPlanError::SemanticMismatch)
    );
    assert_eq!(
        EvaluationPlan::seal_from_bridge(&non_evaluable_bridge(snapshot), inputs, &mut repo,),
        Err(EvaluationPlanError::BridgeNotEvaluable)
    );
}

#[test]
fn rejects_a_final_or_development_partition_swap_and_all_semantic_axes() {
    let mut repo = InMemoryArtifactRepository::default();
    let snapshot = repo
        .append(
            None,
            ArtifactDraft::new(
                "tenant_a",
                "018f0f4e-7bbd-7000-8000-000000000220",
                1,
                ArtifactKind::SourceSnapshot,
                json!({}),
                None,
            ),
        )
        .unwrap()
        .reference();
    let baseline = artifact(
        &mut repo,
        "018f0f4e-7bbd-7000-8000-000000000221",
        "baseline",
        "shared",
        "reduce_repeat_payment_contacts",
        "customer_episode",
        "scenario_oracle/payment_status_resolution_v1",
        snapshot.clone(),
    );
    let oracle = artifact(
        &mut repo,
        "018f0f4e-7bbd-7000-8000-000000000222",
        "oracle",
        "shared",
        "reduce_repeat_payment_contacts",
        "customer_episode",
        "scenario_oracle/payment_status_resolution_v1",
        snapshot.clone(),
    );
    let development = artifact(
        &mut repo,
        "018f0f4e-7bbd-7000-8000-000000000223",
        "development_suite",
        "final",
        "reduce_repeat_payment_contacts",
        "customer_episode",
        "scenario_oracle/payment_status_resolution_v1",
        snapshot.clone(),
    );
    let final_suite = artifact(
        &mut repo,
        "018f0f4e-7bbd-7000-8000-000000000224",
        "final_suite",
        "final",
        "reduce_repeat_payment_contacts",
        "customer_episode",
        "scenario_oracle/payment_status_resolution_v1",
        snapshot.clone(),
    );
    assert_eq!(
        EvaluationPlan::seal_from_bridge(
            &mechanism_proxy_bridge(snapshot.clone()),
            EvaluationInputs::new(
                EvaluationArtifactRef::baseline(baseline),
                EvaluationArtifactRef::oracle(oracle),
                EvaluationArtifactRef::development_suite(development),
                EvaluationArtifactRef::final_suite(final_suite)
            ),
            &mut repo
        ),
        Err(EvaluationPlanError::ReferenceMismatch)
    );
    for (outcome, unit, measure, start) in [
        (
            "wrong",
            "customer_episode",
            "scenario_oracle/payment_status_resolution_v1",
            225,
        ),
        (
            "reduce_repeat_payment_contacts",
            "wrong",
            "scenario_oracle/payment_status_resolution_v1",
            229,
        ),
        (
            "reduce_repeat_payment_contacts",
            "customer_episode",
            "wrong",
            233,
        ),
    ] {
        let values = inputs_named(
            &mut repo,
            snapshot.clone(),
            outcome,
            unit,
            measure,
            [
                &format!("018f0f4e-7bbd-7000-8000-{start:012}"),
                &format!("018f0f4e-7bbd-7000-8000-{:012}", start + 1),
                &format!("018f0f4e-7bbd-7000-8000-{:012}", start + 2),
                &format!("018f0f4e-7bbd-7000-8000-{:012}", start + 3),
            ],
        );
        assert_eq!(
            EvaluationPlan::seal_from_bridge(
                &mechanism_proxy_bridge(snapshot.clone()),
                values,
                &mut repo
            ),
            Err(EvaluationPlanError::SemanticMismatch)
        );
    }
}

#[test]
fn commitment_changes_with_verified_input_identity() {
    let mut repo = InMemoryArtifactRepository::default();
    let snapshot = repo
        .append(
            None,
            ArtifactDraft::new(
                "tenant_a",
                "018f0f4e-7bbd-7000-8000-000000000230",
                1,
                ArtifactKind::SourceSnapshot,
                json!({}),
                None,
            ),
        )
        .unwrap()
        .reference();
    let bridge = mechanism_proxy_bridge(snapshot.clone());
    let first = EvaluationPlan::seal_from_bridge(
        &bridge,
        inputs_named(
            &mut repo,
            snapshot.clone(),
            "reduce_repeat_payment_contacts",
            "customer_episode",
            "scenario_oracle/payment_status_resolution_v1",
            [
                "018f0f4e-7bbd-7000-8000-000000000231",
                "018f0f4e-7bbd-7000-8000-000000000232",
                "018f0f4e-7bbd-7000-8000-000000000233",
                "018f0f4e-7bbd-7000-8000-000000000234",
            ],
        ),
        &mut repo,
    )
    .unwrap();
    let second = EvaluationPlan::seal_from_bridge(
        &bridge,
        inputs_named(
            &mut repo,
            snapshot,
            "reduce_repeat_payment_contacts",
            "customer_episode",
            "scenario_oracle/payment_status_resolution_v1",
            [
                "018f0f4e-7bbd-7000-8000-000000000235",
                "018f0f4e-7bbd-7000-8000-000000000236",
                "018f0f4e-7bbd-7000-8000-000000000237",
                "018f0f4e-7bbd-7000-8000-000000000238",
            ],
        ),
        &mut repo,
    )
    .unwrap();
    assert_ne!(first.commitment(), second.commitment());
}
