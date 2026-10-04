use std::collections::BTreeMap;

use improvement_engine_core::paired_scenario::{
    ArmBinding, ArmInfrastructureFailure, ArmObservation, ComparedArm, ComparisonAuthority,
    InfrastructureFailure, PairError, PairPlan, PairVerdict, ProjectionField, ProjectionValue,
    evaluate_pair,
};

fn oracle() -> BTreeMap<ProjectionField, ProjectionValue> {
    BTreeMap::from([(ProjectionField::PaymentStatus, ProjectionValue::Settled)])
}

fn digest(char: char) -> String {
    format!("sha256:{}", char.to_string().repeat(64))
}

const EVALUATION_ID: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const SCENARIO_ID: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const FIXTURE_ID: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
const BASELINE_ARM_ID: &str =
    "sha256:4444444444444444444444444444444444444444444444444444444444444444";
const CANDIDATE_ARM_ID: &str =
    "sha256:5555555555555555555555555555555555555555555555555555555555555555";

fn plan() -> PairPlan {
    PairPlan::new(
        EVALUATION_ID,
        SCENARIO_ID,
        FIXTURE_ID,
        digest('a'),
        digest('b'),
        digest('c'),
        oracle(),
    )
    .unwrap()
}

fn completed(arm_id: &str, artifact_digest: &str, value: ProjectionValue) -> ArmObservation {
    ArmObservation::completed(
        ArmBinding {
            evaluation_id: EVALUATION_ID.to_owned(),
            scenario_id: SCENARIO_ID.to_owned(),
            fixture_id: FIXTURE_ID.to_owned(),
            arm_id: match arm_id {
                "baseline-arm" | "shared-arm" => BASELINE_ARM_ID,
                "candidate-arm" => CANDIDATE_ARM_ID,
                other => other,
            }
            .to_owned(),
            seed_digest: digest('a'),
            artifact_digest: artifact_digest.to_owned(),
        },
        BTreeMap::from([(ProjectionField::PaymentStatus, value)]),
        vec![digest('d'), digest('e')],
    )
}

#[test]
fn same_frozen_seed_and_oracle_produce_bound_pair_receipt_without_lift_claim() {
    let result = evaluate_pair(
        &plan(),
        &completed("baseline-arm", &digest('b'), ProjectionValue::Settled),
        &completed("candidate-arm", &digest('c'), ProjectionValue::Settled),
    )
    .unwrap();

    assert_eq!(result.verdict(), &PairVerdict::NoRegressionObserved);
    assert_eq!(result.seed_digest(), digest('a'));
    assert_eq!(result.baseline_matches_oracle(), Some(true));
    assert_eq!(result.candidate_matches_oracle(), Some(true));
    assert!(!result.business_lift_measured());
    assert!(result.evidence_digest().starts_with("sha256:"));
    assert!(result.validate_integrity());
    assert_eq!(
        result.authority(),
        &ComparisonAuthority::FixtureOnlyUnverified
    );
}

#[test]
fn candidate_oracle_regression_is_fail_not_infrastructure_failure() {
    let result = evaluate_pair(
        &plan(),
        &completed("baseline-arm", &digest('b'), ProjectionValue::Settled),
        &completed("candidate-arm", &digest('c'), ProjectionValue::Pending),
    )
    .unwrap();

    assert_eq!(result.verdict(), &PairVerdict::CandidateRegression);
    assert_eq!(result.baseline_matches_oracle(), Some(true));
    assert_eq!(result.candidate_matches_oracle(), Some(false));
}

#[test]
fn unavailable_arm_is_failed_infra_not_a_candidate_regression() {
    let unavailable = ArmObservation::failed_infra(
        ArmBinding {
            evaluation_id: EVALUATION_ID.to_owned(),
            scenario_id: SCENARIO_ID.to_owned(),
            fixture_id: FIXTURE_ID.to_owned(),
            arm_id: CANDIDATE_ARM_ID.to_owned(),
            seed_digest: digest('a'),
            artifact_digest: digest('c'),
        },
        InfrastructureFailure::SandboxUnavailable,
    );

    let result = evaluate_pair(
        &plan(),
        &completed("baseline-arm", &digest('b'), ProjectionValue::Settled),
        &unavailable,
    )
    .unwrap();

    assert_eq!(result.verdict(), &PairVerdict::FailedInfra);
    assert_eq!(result.baseline_matches_oracle(), Some(true));
    assert_eq!(result.candidate_matches_oracle(), None);
    assert_eq!(
        result.infrastructure_failures(),
        vec![ArmInfrastructureFailure {
            arm: ComparedArm::Candidate,
            failure: InfrastructureFailure::SandboxUnavailable,
        }]
    );
}

#[test]
fn when_both_arms_fail_infrastructure_both_are_preserved_in_the_receipt() {
    let baseline = ArmObservation::failed_infra(
        ArmBinding {
            evaluation_id: EVALUATION_ID.to_owned(),
            scenario_id: SCENARIO_ID.to_owned(),
            fixture_id: FIXTURE_ID.to_owned(),
            arm_id: BASELINE_ARM_ID.to_owned(),
            seed_digest: digest('a'),
            artifact_digest: digest('b'),
        },
        InfrastructureFailure::Timeout,
    );
    let candidate = ArmObservation::failed_infra(
        ArmBinding {
            evaluation_id: EVALUATION_ID.to_owned(),
            scenario_id: SCENARIO_ID.to_owned(),
            fixture_id: FIXTURE_ID.to_owned(),
            arm_id: CANDIDATE_ARM_ID.to_owned(),
            seed_digest: digest('a'),
            artifact_digest: digest('c'),
        },
        InfrastructureFailure::SandboxUnavailable,
    );

    let result = evaluate_pair(&plan(), &baseline, &candidate).unwrap();

    assert_eq!(result.verdict(), &PairVerdict::FailedInfra);
    assert_eq!(
        result.infrastructure_failures(),
        vec![
            ArmInfrastructureFailure {
                arm: ComparedArm::Baseline,
                failure: InfrastructureFailure::Timeout,
            },
            ArmInfrastructureFailure {
                arm: ComparedArm::Candidate,
                failure: InfrastructureFailure::SandboxUnavailable,
            },
        ]
    );
}

#[test]
fn baseline_that_misses_the_oracle_makes_the_pair_inconclusive_not_a_pass() {
    let result = evaluate_pair(
        &plan(),
        &completed("baseline-arm", &digest('b'), ProjectionValue::Pending),
        &completed("candidate-arm", &digest('c'), ProjectionValue::Settled),
    )
    .unwrap();

    assert_eq!(result.verdict(), &PairVerdict::NotComparable);
    assert_eq!(result.baseline_matches_oracle(), Some(false));
    assert_eq!(result.candidate_matches_oracle(), Some(true));
}

#[test]
fn stale_or_mutated_candidate_or_base_is_rejected_before_comparison() {
    let result = evaluate_pair(
        &plan(),
        &completed("baseline-arm", &digest('f'), ProjectionValue::Settled),
        &completed("candidate-arm", &digest('c'), ProjectionValue::Settled),
    );
    assert!(matches!(
        result,
        Err(PairError::ArtifactDigestMismatch { arm: "baseline" })
    ));
}

#[test]
fn mismatched_seed_is_not_a_valid_pair() {
    let candidate = ArmObservation::completed(
        ArmBinding {
            evaluation_id: EVALUATION_ID.to_owned(),
            scenario_id: SCENARIO_ID.to_owned(),
            fixture_id: FIXTURE_ID.to_owned(),
            arm_id: CANDIDATE_ARM_ID.to_owned(),
            seed_digest: digest('f'),
            artifact_digest: digest('c'),
        },
        oracle(),
        vec!["sha256:receipt".to_owned()],
    );

    assert!(matches!(
        evaluate_pair(
            &plan(),
            &completed("baseline-arm", &digest('b'), ProjectionValue::Settled),
            &candidate
        ),
        Err(PairError::BindingMismatch { arm: "candidate" })
    ));
}

#[test]
fn mismatched_scenario_is_not_a_valid_pair() {
    let candidate = ArmObservation::completed(
        ArmBinding {
            evaluation_id: EVALUATION_ID.to_owned(),
            scenario_id: digest('9'),
            fixture_id: FIXTURE_ID.to_owned(),
            arm_id: CANDIDATE_ARM_ID.to_owned(),
            seed_digest: digest('a'),
            artifact_digest: digest('c'),
        },
        oracle(),
        vec![digest('d')],
    );

    assert!(matches!(
        evaluate_pair(
            &plan(),
            &completed("baseline-arm", &digest('b'), ProjectionValue::Settled),
            &candidate
        ),
        Err(PairError::BindingMismatch { arm: "candidate" })
    ));
}

#[test]
fn malformed_or_non_hex_content_pins_cannot_seal_a_plan() {
    let malformed = format!("sha256:{}z", "a".repeat(63));
    assert!(matches!(
        PairPlan::new(
            EVALUATION_ID,
            SCENARIO_ID,
            FIXTURE_ID,
            digest('a'),
            malformed,
            digest('c'),
            oracle(),
        ),
        Err(PairError::InvalidPlan)
    ));
}

#[test]
fn raw_email_phone_and_free_text_identifiers_cannot_be_sealed_or_echoed() {
    for unsafe_id in ["alex@example.com", "+573001234567", "customer called angry"] {
        assert!(matches!(
            PairPlan::new(
                unsafe_id,
                digest('2'),
                digest('3'),
                digest('a'),
                digest('b'),
                digest('c'),
                oracle(),
            ),
            Err(PairError::InvalidPlan)
        ));
    }

    let binding = ArmBinding {
        evaluation_id: EVALUATION_ID.to_owned(),
        scenario_id: SCENARIO_ID.to_owned(),
        fixture_id: FIXTURE_ID.to_owned(),
        arm_id: "+573001234567".to_owned(),
        seed_digest: digest('a'),
        artifact_digest: digest('b'),
    };
    let invalid_arm = ArmObservation::completed(binding, oracle(), vec![]);
    assert!(matches!(
        evaluate_pair(
            &plan(),
            &invalid_arm,
            &completed("candidate-arm", &digest('c'), ProjectionValue::Settled)
        ),
        Err(PairError::BindingMismatch { arm: "baseline" })
    ));
}

#[test]
fn pair_must_use_distinct_baseline_and_candidate_arms() {
    assert!(matches!(
        evaluate_pair(
            &plan(),
            &completed("shared-arm", &digest('b'), ProjectionValue::Settled),
            &completed("shared-arm", &digest('c'), ProjectionValue::Settled),
        ),
        Err(PairError::SharedArm)
    ));
}
