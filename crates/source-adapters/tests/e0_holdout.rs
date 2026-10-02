use std::fs;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray, RecordBatch, StringArray, TimestampMicrosecondArray};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use improvement_engine_source_adapters::{
    E0HoldoutPolicy, E0HoldoutStatus, PreparationConfig, PreparedSource,
    attest_selected_e0_recurrence_candidate, evaluate_e0_recurrence_holdout, prepare_e0_package,
};
use parquet::arrow::ArrowWriter;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const DISCOVERY_SUPPORT: u64 = 3;
const HOLDOUT_SUPPORT: u64 = 5;

fn write_parquet(path: &Path, schema: Schema, arrays: Vec<ArrayRef>) {
    let schema = Arc::new(schema);
    let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
    let file = fs::File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn fixture(replay_signatures: &[&str], include_query_table: bool) -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let data = root.join("datos");
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(root.join("contratos")).unwrap();
    fs::write(root.join("contratos/platform_history.json"), "{} ").unwrap();

    let ids = (1..=8)
        .map(|i| format!("test-case-{i}"))
        .collect::<Vec<_>>();
    write_parquet(
        &data.join("case.parquet"),
        Schema::new(vec![
            Field::new("case_id", DataType::Utf8, false),
            Field::new(
                "opened_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ),
            Field::new("channel", DataType::Utf8, false),
            Field::new("language", DataType::Utf8, false),
            Field::new("topic", DataType::Utf8, false),
            Field::new("priority", DataType::Utf8, false),
        ]),
        vec![
            Arc::new(StringArray::from(
                ids.iter().map(String::as_str).collect::<Vec<_>>(),
            )),
            Arc::new(
                TimestampMicrosecondArray::from((1..=8).map(|i| i * 1_000_000).collect::<Vec<_>>())
                    .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec!["chat"; 8])),
            Arc::new(StringArray::from(vec!["es"; 8])),
            Arc::new(StringArray::from(vec!["support"; 8])),
            Arc::new(StringArray::from(vec!["normal"; 8])),
        ],
    );

    write_parquet(
        &data.join("tool_call.parquet"),
        Schema::new(vec![
            Field::new("case_id", DataType::Utf8, false),
            Field::new(
                "event_time",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ),
            Field::new("call_id", DataType::Utf8, false),
            Field::new("actor_role", DataType::Utf8, false),
            Field::new("tool_id", DataType::Utf8, false),
            Field::new("permission_level", DataType::Utf8, false),
            Field::new("status", DataType::Utf8, false),
            Field::new("verified", DataType::Boolean, true),
        ]),
        vec![
            Arc::new(StringArray::from(
                ids.iter().map(String::as_str).collect::<Vec<_>>(),
            )),
            Arc::new(
                TimestampMicrosecondArray::from(
                    (1..=8).map(|i| i * 1_000_000 + 100).collect::<Vec<_>>(),
                )
                .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(
                (1..=8).map(|i| format!("call-{i}")).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(vec!["tree"; 8])),
            Arc::new(StringArray::from(vec!["status_lookup"; 8])),
            Arc::new(StringArray::from(vec!["read"; 8])),
            Arc::new(StringArray::from(vec!["ok"; 8])),
            Arc::new(BooleanArray::from(vec![Some(true); 8])),
        ],
    );

    if include_query_table {
        let mut query_case_ids = vec![
            "test-case-1".to_owned(),
            "test-case-2".to_owned(),
            "test-case-3".to_owned(),
            "test-case-4".to_owned(),
        ];
        let mut signatures = vec!["normalized-pattern-alpha"; 3];
        signatures.push("other-arranque-pattern");
        let replay_case_numbers = [4, 5, 6, 7, 8, 8];
        for (index, signature) in replay_signatures.iter().enumerate() {
            let case_number = replay_case_numbers[index.min(replay_case_numbers.len() - 1)];
            query_case_ids.push(format!("test-case-{case_number}"));
            signatures.push(signature);
        }
        let query_ids = (1..=query_case_ids.len())
            .map(|i| format!("query-{i}"))
            .collect::<Vec<_>>();
        let row_times = (0..query_case_ids.len())
            .map(|i| (i as i64 + 1) * 1_000_000 + 200)
            .collect::<Vec<_>>();
        write_parquet(
            &data.join("copilot_query.parquet"),
            Schema::new(vec![
                Field::new("query_id", DataType::Utf8, false),
                Field::new("case_id", DataType::Utf8, false),
                Field::new(
                    "event_time",
                    DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                    false,
                ),
                Field::new("query_signature", DataType::Utf8, false),
                Field::new("answered_by", DataType::Utf8, false),
            ]),
            vec![
                Arc::new(StringArray::from(
                    query_ids.iter().map(String::as_str).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    query_case_ids
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                )),
                Arc::new(TimestampMicrosecondArray::from(row_times).with_timezone("UTC")),
                Arc::new(StringArray::from(signatures)),
                Arc::new(StringArray::from(vec!["tool:lookup"; query_ids.len()])),
            ],
        );
    }
    temp
}

fn prepare(temp: &TempDir, arranque_cases: usize) -> PreparedSource {
    prepare_for_tenant(temp, arranque_cases, "test-tenant")
}

fn prepare_for_tenant(temp: &TempDir, arranque_cases: usize, tenant_id: &str) -> PreparedSource {
    let config = PreparationConfig::new(tenant_id, "1970-01-01T00:02:00Z", arranque_cases).unwrap();
    prepare_e0_package(temp.path(), &config).unwrap()
}

// Independently reproduce the current core's Arranque-only pattern commitment
// for these synthetic fixtures; no Replay data is used to construct it.
fn core_pattern_ref(source: &PreparedSource) -> String {
    use improvement_engine_source_adapters::{CasePhase, E0Fact};
    use std::collections::{BTreeMap, BTreeSet};

    let arranque = source
        .agent_inputs()
        .cases()
        .iter()
        .filter(|case| case.phase() == CasePhase::Arranque)
        .map(|case| case.ordinal())
        .collect::<BTreeSet<_>>();
    let mut support = BTreeMap::<String, BTreeSet<u32>>::new();
    for fact in source.agent_inputs().facts() {
        if let E0Fact::CopilotQuery {
            case_ordinal,
            query_signature,
            ..
        } = fact
        {
            if arranque.contains(case_ordinal) {
                support
                    .entry(query_signature.clone())
                    .or_default()
                    .insert(*case_ordinal);
            }
        }
    }
    let (signature, _) = support
        .iter()
        .max_by(|left, right| {
            left.1
                .len()
                .cmp(&right.1.len())
                .then_with(|| right.0.cmp(left.0))
        })
        .unwrap();
    let reference = format!(
        "pattern-v1:{}:{}:{}:{}",
        source.snapshot_ref().tenant_id,
        source.manifest_digest(),
        source.snapshot_ref().digest,
        signature
    );
    format!("sha256:{:x}", Sha256::digest(reference.as_bytes()))
}

fn policy(minimum_distinct_cases: u64) -> E0HoldoutPolicy {
    E0HoldoutPolicy::new(minimum_distinct_cases).unwrap()
}

#[test]
fn holdout_policy_rejects_aggregates_below_the_five_case_privacy_floor() {
    assert_eq!(
        E0HoldoutPolicy::new(4),
        Err(improvement_engine_source_adapters::E0HoldoutError::InvalidPolicy)
    );
    assert_eq!(policy(5).minimum_distinct_case_support(), 5);
}

#[test]
fn holdout_counts_distinct_reproduction_cases_and_reports_safe_rate() {
    let temp = fixture(&["normalized-pattern-alpha"; 6], true);
    let source = prepare(&temp, 3);
    let candidate = attest_selected_e0_recurrence_candidate(
        &source,
        &core_pattern_ref(&source),
        DISCOVERY_SUPPORT,
    )
    .expect("core candidate should be attested using Arranque evidence only");

    let result =
        evaluate_e0_recurrence_holdout(&candidate, &source, &policy(HOLDOUT_SUPPORT)).unwrap();
    assert_eq!(result.status(), E0HoldoutStatus::Replicated);
    assert_eq!(result.reproduction_case_count(), Some(5));
    assert_eq!(result.queried_case_count(), Some(5));
    assert_eq!(result.matching_case_count(), Some(5));
    assert_eq!(result.recurrence_rate_basis_points(), Some(10_000));
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains("normalized-pattern-alpha"));
    assert!(!serialized.contains("test-case-"));
}

#[test]
fn replay_pattern_changes_cannot_change_attested_arranque_candidate() {
    let replay_a = fixture(&["normalized-pattern-alpha"; 5], true);
    let replay_b = fixture(&["entirely-different-replay-pattern"; 5], true);
    let source_a = prepare(&replay_a, 3);
    let source_b = prepare(&replay_b, 3);
    let candidate_a = attest_selected_e0_recurrence_candidate(
        &source_a,
        &core_pattern_ref(&source_a),
        DISCOVERY_SUPPORT,
    )
    .unwrap();
    let candidate_b = attest_selected_e0_recurrence_candidate(
        &source_b,
        &core_pattern_ref(&source_b),
        DISCOVERY_SUPPORT,
    )
    .unwrap();
    assert_eq!(
        candidate_a.arranque_support_cases(),
        candidate_b.arranque_support_cases()
    );

    let holdout = fixture(&["normalized-pattern-alpha"; 6], true);
    let holdout = prepare(&holdout, 3);
    let result_a =
        evaluate_e0_recurrence_holdout(&candidate_a, &holdout, &policy(HOLDOUT_SUPPORT)).unwrap();
    let result_b =
        evaluate_e0_recurrence_holdout(&candidate_b, &holdout, &policy(HOLDOUT_SUPPORT)).unwrap();
    assert_eq!(
        result_a.matching_case_count(),
        result_b.matching_case_count()
    );
    assert_eq!(result_a.status(), result_b.status());
}

#[test]
fn holdout_statuses_distinguish_absent_support_from_no_observation() {
    let discovery = fixture(&[], true);
    let discovery = prepare(&discovery, 3);
    let candidate = attest_selected_e0_recurrence_candidate(
        &discovery,
        &core_pattern_ref(&discovery),
        DISCOVERY_SUPPORT,
    )
    .unwrap();

    let absent = fixture(&[], false);
    let absent = prepare(&absent, 3);
    let unavailable =
        evaluate_e0_recurrence_holdout(&candidate, &absent, &policy(HOLDOUT_SUPPORT)).unwrap();
    assert_eq!(unavailable.status(), E0HoldoutStatus::Unavailable);
    assert_eq!(unavailable.queried_case_count(), None);
    assert_eq!(unavailable.matching_case_count(), None);

    let no_match = fixture(&["other"; 6], true);
    let no_match = prepare(&no_match, 3);
    let not_observed =
        evaluate_e0_recurrence_holdout(&candidate, &no_match, &policy(HOLDOUT_SUPPORT)).unwrap();
    assert_eq!(not_observed.status(), E0HoldoutStatus::NotObserved);
    assert_eq!(not_observed.matching_case_count(), Some(0));

    let weak = fixture(
        &["normalized-pattern-alpha", "other", "other", "other"],
        true,
    );
    let weak = prepare(&weak, 3);
    let insufficient =
        evaluate_e0_recurrence_holdout(&candidate, &weak, &policy(HOLDOUT_SUPPORT)).unwrap();
    assert_eq!(insufficient.status(), E0HoldoutStatus::InsufficientSupport);
    assert_eq!(insufficient.reproduction_case_count(), None);
    assert_eq!(insufficient.queried_case_count(), None);
    assert_eq!(insufficient.matching_case_count(), None);
    assert_eq!(insufficient.recurrence_rate_basis_points(), None);

    for matching_support in 1..HOLDOUT_SUPPORT {
        let mut signatures = vec!["other"; 6];
        signatures[..matching_support as usize].fill("normalized-pattern-alpha");
        let sparse_match = fixture(&signatures, true);
        let sparse_match = prepare(&sparse_match, 3);
        let insufficient_match =
            evaluate_e0_recurrence_holdout(&candidate, &sparse_match, &policy(HOLDOUT_SUPPORT))
                .unwrap();
        assert_eq!(
            insufficient_match.status(),
            E0HoldoutStatus::InsufficientSupport
        );
        assert_eq!(insufficient_match.reproduction_case_count(), None);
        assert_eq!(insufficient_match.queried_case_count(), None);
        assert_eq!(insufficient_match.matching_case_count(), None);
        assert_eq!(insufficient_match.recurrence_rate_basis_points(), None);
    }
}

#[test]
fn selected_candidate_must_be_supported_by_arranque_and_holdout_is_bound_to_policy_and_sources() {
    let discovery = fixture(&["normalized-pattern-alpha"; 4], true);
    let discovery = prepare(&discovery, 3);
    let valid_ref = core_pattern_ref(&discovery);
    let invalid_ref = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert!(
        attest_selected_e0_recurrence_candidate(&discovery, invalid_ref, DISCOVERY_SUPPORT)
            .is_err()
    );
    let candidate =
        attest_selected_e0_recurrence_candidate(&discovery, &valid_ref, DISCOVERY_SUPPORT).unwrap();

    let holdout = fixture(
        &["normalized-pattern-alpha", "other", "other", "other"],
        true,
    );
    let holdout = prepare(&holdout, 3);
    let result =
        evaluate_e0_recurrence_holdout(&candidate, &holdout, &policy(HOLDOUT_SUPPORT)).unwrap();
    assert!(result.discovery_source_commitment().starts_with("sha256:"));
    assert!(result.holdout_source_commitment().starts_with("sha256:"));
    assert_eq!(result.policy_version(), 2);
}

#[test]
fn support_threshold_is_inclusive_and_tenant_scope_must_match() {
    let discovery = fixture(&["normalized-pattern-alpha"; 4], true);
    let discovery = prepare(&discovery, 3);
    let candidate = attest_selected_e0_recurrence_candidate(
        &discovery,
        &core_pattern_ref(&discovery),
        DISCOVERY_SUPPORT,
    )
    .unwrap();

    let exact = fixture(&["normalized-pattern-alpha"; 6], true);
    let exact = prepare(&exact, 3);
    let at_threshold = evaluate_e0_recurrence_holdout(&candidate, &exact, &policy(5)).unwrap();
    assert_eq!(at_threshold.queried_case_count(), Some(5));
    assert_eq!(at_threshold.matching_case_count(), Some(5));
    assert_eq!(at_threshold.status(), E0HoldoutStatus::Replicated);

    let wrong_scope = fixture(&["normalized-pattern-alpha"; 6], true);
    let wrong_scope = prepare_for_tenant(&wrong_scope, 3, "other-tenant");
    assert_eq!(
        evaluate_e0_recurrence_holdout(&candidate, &wrong_scope, &policy(5)),
        Err(improvement_engine_source_adapters::E0HoldoutError::SourceScopeMismatch)
    );
}
