mod common;
use common::{migrations_dir, TempDb};
use pg::{migrate, schema};
use std::fs;

/// Existing Codex test setups (crates/core/tests/*), merged: 0001+0002_model (model_attempt_repository),
/// 0001+0002_memory (postgres_artifact_migration), 0001..0004 (postgres_memory_temporal_receipts),
/// 0002_platform (postgres_platform_observations), 0003 (postgres_run_events).
const UNION_ORDER: [&str; 6] = [
    "0001_pulso_artifact_revisions.sql",
    "0002_pulso_memory_control.sql",
    "0003_pulso_run_events.sql",
    "0004_pulso_memory_temporal_receipts.sql",
    "0002_model_attempt_ledger.sql",
    "0002_pulso_platform_observations.sql",
];

#[test]
fn runner_schema_equals_union_of_existing_test_setups() {
    let Some(reference) = TempDb::create() else { return };
    let mut rc = reference.connect();
    for f in UNION_ORDER {
        let sql = fs::read_to_string(migrations_dir().join(f)).unwrap();
        rc.batch_execute(&sql).unwrap_or_else(|e| panic!("{f}: {e}"));
    }
    let expected = schema::snapshot(&mut rc).unwrap();
    assert!(expected.len() > 50, "reference snapshot looks empty: {}", expected.len());

    let fresh = TempDb::create().unwrap();
    let mut c = fresh.connect();
    let mut baseline = migrate::discover(&migrations_dir()).unwrap();
    baseline.retain(|m| m.version < 50); // 005x are deliberate deltas, tested separately
    migrate::migrate(&mut c, &baseline).unwrap();
    let actual = schema::snapshot(&mut c).unwrap();

    let only_expected: Vec<_> = expected.iter().filter(|l| !actual.contains(l)).collect();
    let only_actual: Vec<_> = actual.iter().filter(|l| !expected.contains(l)).collect();
    assert!(
        only_expected.is_empty() && only_actual.is_empty(),
        "schema diff: missing={} extra={}\nmissing: {:#?}\nextra: {:#?}",
        only_expected.len(), only_actual.len(), &only_expected[..only_expected.len().min(5)], &only_actual[..only_actual.len().min(5)]
    );
}
