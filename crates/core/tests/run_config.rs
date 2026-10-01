use improvement_engine_core::run_config::{
    Cadence, EligibleSource, RunConfig, RunConfigError, ScanBudget,
};

fn contacts() -> EligibleSource {
    EligibleSource::new("latam_bank", "call_center_interactions").expect("valid fixture source")
}

fn transactions() -> EligibleSource {
    EligibleSource::new("latam_bank", "card_transactions").expect("valid fixture source")
}

#[test]
fn accepts_a_bounded_scheduled_run_for_an_explicit_source() {
    let config = RunConfig::new(
        1,
        Cadence::scheduled_every_minutes(60).expect("hourly cadence is supported"),
        ScanBudget::new(2, 50_000, 16 * 1024 * 1024).expect("bounded budget"),
        vec![contacts()],
    )
    .expect("complete run configuration is valid");

    assert!(config.allows(&contacts()).is_ok());
    assert!(config.identity().as_str().starts_with("run-config:sha256:"));
}

#[test]
fn identity_is_stable_for_the_same_configuration_regardless_of_input_order() {
    let budget = ScanBudget::new(2, 50_000, 16 * 1024 * 1024).expect("bounded budget");
    let cadence = Cadence::scheduled_every_minutes(60).expect("hourly cadence is supported");

    let left = RunConfig::new(
        1,
        cadence.clone(),
        budget.clone(),
        vec![contacts(), transactions()],
    )
    .expect("valid config");
    let right =
        RunConfig::new(1, cadence, budget, vec![transactions(), contacts()]).expect("valid config");

    assert_eq!(left.identity(), right.identity());
    assert_eq!(
        left.eligible_sources().collect::<Vec<_>>(),
        right.eligible_sources().collect::<Vec<_>>()
    );
}

#[test]
fn identity_changes_when_a_runtime_bound_changes() {
    let cadence = Cadence::scheduled_every_minutes(60).expect("hourly cadence is supported");
    let small = RunConfig::new(
        1,
        cadence.clone(),
        ScanBudget::new(1, 50_000, 16 * 1024 * 1024).expect("bounded budget"),
        vec![contacts()],
    )
    .expect("valid config");
    let larger = RunConfig::new(
        1,
        cadence,
        ScanBudget::new(1, 50_001, 16 * 1024 * 1024).expect("bounded budget"),
        vec![contacts()],
    )
    .expect("valid config");

    assert_ne!(small.identity(), larger.identity());
}

#[test]
fn rejects_invalid_run_bounds_with_explicit_errors() {
    assert_eq!(
        Cadence::scheduled_every_minutes(14),
        Err(RunConfigError::CadenceBelowMinimum {
            minimum_minutes: 15,
            actual_minutes: 14,
        })
    );
    assert_eq!(
        ScanBudget::new(0, 1, 1),
        Err(RunConfigError::BudgetMustBePositive {
            field: "max_sources"
        })
    );
    assert_eq!(
        ScanBudget::new(1, 5_000_001, 1),
        Err(RunConfigError::BudgetExceedsMaximum {
            field: "max_rows_per_source",
            maximum: 5_000_000,
            actual: 5_000_001,
        })
    );
}

#[test]
fn rejects_duplicate_or_unbounded_source_eligibility() {
    let budget = ScanBudget::new(1, 50_000, 16 * 1024 * 1024).expect("bounded budget");
    let cadence = Cadence::scheduled_every_minutes(60).expect("hourly cadence is supported");

    assert_eq!(
        RunConfig::new(
            1,
            cadence.clone(),
            budget.clone(),
            vec![contacts(), contacts()]
        ),
        Err(RunConfigError::DuplicateEligibleSource)
    );
    assert_eq!(
        RunConfig::new(1, cadence, budget, vec![contacts(), transactions()]),
        Err(RunConfigError::EligibleSourcesExceedBudget {
            eligible: 2,
            maximum: 1,
        })
    );
}

#[test]
fn rejects_an_unsupported_source_without_reading_source_data() {
    let config = RunConfig::new(
        1,
        Cadence::EventTriggered,
        ScanBudget::new(1, 50_000, 16 * 1024 * 1024).expect("bounded budget"),
        vec![contacts()],
    )
    .expect("valid config");

    assert_eq!(
        config.allows(&transactions()),
        Err(RunConfigError::UnsupportedSource {
            source_namespace: "latam_bank".to_owned(),
            table: "card_transactions".to_owned(),
        })
    );
}
