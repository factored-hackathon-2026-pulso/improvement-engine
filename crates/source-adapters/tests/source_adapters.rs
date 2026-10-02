use std::fs;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray, RecordBatch, StringArray, TimestampMicrosecondArray};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use improvement_engine_source_adapters::{
    CasePhase, PreparationConfig, PreparedSource, SourceKind, evaluator, prepare_e0_package,
    prepare_original_bank,
};
use parquet::arrow::ArrowWriter;
use tempfile::TempDir;

fn config(arranque_cases: usize) -> PreparationConfig {
    PreparationConfig::new("demo-tenant", "2025-07-01T00:00:00Z", arranque_cases)
        .expect("valid config")
}

fn config_with_cutoff(arranque_cases: usize, cutoff: &str) -> PreparationConfig {
    PreparationConfig::new("demo-tenant", cutoff, arranque_cases).expect("valid config")
}

fn write_parquet(path: &Path, schema: Schema, columns: Vec<ArrayRef>) {
    let schema = Arc::new(schema);
    let batch = RecordBatch::try_new(schema.clone(), columns).expect("valid synthetic batch");
    let file = fs::File::create(path).expect("create parquet fixture");
    let mut writer = ArrowWriter::try_new(file, schema, None).expect("create parquet writer");
    writer.write(&batch).expect("write parquet fixture");
    writer.close().expect("close parquet fixture");
}

fn e0_fixture(root: &Path) {
    let data = root.join("datos");
    fs::create_dir_all(&data).expect("create data dir");
    fs::create_dir_all(root.join("contratos")).expect("create contracts dir");
    fs::write(
        root.join("contratos/platform_history.json"),
        "{\"synthetic\":true}",
    )
    .expect("write synthetic platform contract");

    let cases = Schema::new(vec![
        Field::new("case_id", DataType::Utf8, false),
        Field::new(
            "opened_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("customer_id", DataType::Utf8, false),
        Field::new("channel", DataType::Utf8, false),
        Field::new("language", DataType::Utf8, false),
        Field::new("topic", DataType::Utf8, false),
        Field::new("priority", DataType::Utf8, false),
    ]);
    write_parquet(
        &data.join("case.parquet"),
        cases,
        vec![
            Arc::new(StringArray::from(vec!["case-b", "case-a", "case-c"])),
            Arc::new(
                TimestampMicrosecondArray::from(vec![20_000_000_i64, 10_000_000, 30_000_000])
                    .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec![
                "synthetic-customer-2",
                "synthetic-customer-1",
                "synthetic-customer-3",
            ])),
            Arc::new(StringArray::from(vec!["chat", "phone", "web"])),
            Arc::new(StringArray::from(vec!["es", "es", "es"])),
            Arc::new(StringArray::from(vec![
                "payments",
                "person@example.test",
                "account",
            ])),
            Arc::new(StringArray::from(vec!["normal", "high", "normal"])),
        ],
    );

    let calls = Schema::new(vec![
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
        Field::new("state_change", DataType::Utf8, true),
        Field::new("retry_count", DataType::Int32, true),
        Field::new("latency_ms", DataType::Int64, true),
        Field::new("params", DataType::Utf8, true),
    ]);
    write_parquet(
        &data.join("tool_call.parquet"),
        calls,
        vec![
            Arc::new(StringArray::from(vec![
                "case-b", "case-a", "case-a", "case-c",
            ])),
            Arc::new(
                TimestampMicrosecondArray::from(vec![
                    20_500_000_i64,
                    11_000_000,
                    12_000_000,
                    31_000_000,
                ])
                .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec![
                "call-1", "call-2", "call-3", "call-4",
            ])),
            Arc::new(StringArray::from(vec![
                "analyst", "tree", "ai_agent", "judge",
            ])),
            Arc::new(StringArray::from(vec![
                "get_balance",
                "read_txn",
                "create_dispute",
                "credit",
            ])),
            Arc::new(StringArray::from(vec![
                "read",
                "read",
                "confirm",
                "human_only",
            ])),
            Arc::new(StringArray::from(vec!["timeout", "ok", "error", "denied"])),
            Arc::new(BooleanArray::from(vec![true, true, true, false])),
            Arc::new(arrow_array::StringArray::from(vec![
                None,
                None,
                Some("{\"status\":\"updated\"}"),
                None,
            ])),
            Arc::new(arrow_array::Int32Array::from(vec![0_i32, 0, 1, 0])),
            Arc::new(arrow_array::Int64Array::from(vec![100_i64, 20, 200, 0])),
            Arc::new(StringArray::from(vec![
                "{MONTO}",
                "private",
                "{NOMBRE}",
                "{TARJETA}",
            ])),
        ],
    );

    let turns = Schema::new(vec![
        Field::new("turn_id", DataType::Utf8, false),
        Field::new("case_id", DataType::Utf8, false),
        Field::new(
            "event_time",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("author_role", DataType::Utf8, false),
        Field::new("language", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
    ]);
    write_parquet(
        &data.join("turn.parquet"),
        turns,
        vec![
            Arc::new(StringArray::from(vec!["turn-1", "turn-2"])),
            Arc::new(StringArray::from(vec!["case-a", "case-b"])),
            Arc::new(
                TimestampMicrosecondArray::from(vec![13_000_000_i64, 22_000_000])
                    .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec!["customer", "analyst"])),
            Arc::new(StringArray::from(vec!["es", "es"])),
            Arc::new(StringArray::from(vec![
                "email@example.test",
                "masked {NOMBRE}",
            ])),
        ],
    );

    let labels = Schema::new(vec![
        Field::new("case_id", DataType::Utf8, false),
        Field::new("rank", DataType::Int64, false),
        Field::new("split", DataType::Utf8, false),
        Field::new("final_status", DataType::Utf8, false),
        Field::new("final_sla_breached", DataType::Boolean, false),
    ]);
    write_parquet(
        &data.join("labels.parquet"),
        labels,
        vec![
            Arc::new(StringArray::from(vec!["case-a", "case-b", "case-c"])),
            Arc::new(arrow_array::Int64Array::from(vec![1_i64, 2, 3])),
            Arc::new(StringArray::from(vec![
                "arranque",
                "arranque",
                "reproduccion",
            ])),
            Arc::new(StringArray::from(vec!["Resolved", "Open", "Closed"])),
            Arc::new(BooleanArray::from(vec![false, true, false])),
        ],
    );
}

#[test]
fn original_bank_adapter_seals_recursively_partitioned_csv_without_exposing_values() {
    let temp = TempDir::new().expect("temp dir");
    let table = temp
        .path()
        .join("call_center_interactions/year=2023/month=06");
    fs::create_dir_all(&table).expect("create partition");
    fs::write(
        table.join("part-1.csv"),
        "interaction_id,interaction_date,customer_id,description\nsecret-id,2023-06-01T10:00:00Z,PERSON-123,never expose this\n\"id,with,commas\",2023-06-02T11:00:00Z,PERSON-456,\"line one\nline two\"\n",
    )
    .expect("write synthetic csv");

    let prepared = prepare_original_bank(temp.path(), &config(1)).expect("prepare bank data");

    assert_eq!(prepared.source_kind(), SourceKind::OriginalBank);
    assert_eq!(
        prepared.agent_inputs().cases().len(),
        0,
        "original source has no supported technical-error metric yet"
    );
    assert!(prepared.snapshot_ref().digest.starts_with("sha256:"));
    let serialized = serde_json::to_string(prepared.agent_inputs()).expect("serialize safe input");
    assert!(!serialized.contains("PERSON-123"));
    assert!(!serialized.contains("never expose this"));
    assert!(!serialized.contains("secret-id"));
}

#[test]
fn original_bank_snapshot_changes_when_source_bytes_change() {
    let temp = TempDir::new().expect("temp dir");
    fs::write(
        temp.path().join("call_center_interactions.csv"),
        "interaction_id,interaction_date\nid-1,2023-06-01T10:00:00Z\n",
    )
    .expect("write synthetic csv");
    let first = prepare_original_bank(temp.path(), &config(1)).expect("prepare first snapshot");
    fs::write(
        temp.path().join("call_center_interactions.csv"),
        "interaction_id,interaction_date\nid-2,2023-06-01T10:00:00Z\n",
    )
    .expect("change synthetic csv");
    let second = prepare_original_bank(temp.path(), &config(1)).expect("prepare second snapshot");
    assert_ne!(first.manifest_digest(), second.manifest_digest());
    assert_ne!(first.snapshot_ref(), second.snapshot_ref());
}

#[test]
fn e0_agent_projection_is_chronological_safe_and_excludes_evaluator_labels() {
    let temp = TempDir::new().expect("temp dir");
    e0_fixture(temp.path());

    let prepared = prepare_e0_package(temp.path(), &config(2)).expect("prepare E0");
    assert_eq!(prepared.source_kind(), SourceKind::E0);
    let PreparedSource::E0(e0) = &prepared else {
        panic!("expected E0 source");
    };

    let cases = e0.agent_inputs().cases();
    assert_eq!(cases.len(), 3);
    assert_eq!(cases[0].ordinal(), 1);
    assert_eq!(cases[0].phase(), CasePhase::Arranque);
    assert_eq!(cases[0].opened_at(), "1970-01-01T00:00:10Z");
    assert_eq!(cases[0].events().len(), 3);
    assert_eq!(cases[0].events()[0].technical_error(), Some(false));
    assert_eq!(cases[0].events()[1].technical_error(), Some(true));
    assert_eq!(cases[0].events()[0].event_kind(), "tool_call");
    assert_eq!(cases[0].events()[0].actor_role(), Some("tree"));
    assert!(
        cases[0].events()[0]
            .tool_code()
            .is_some_and(|code| code.starts_with("sha256:"))
    );
    assert_eq!(cases[2].phase(), CasePhase::Reproduccion);
    assert_eq!(cases[2].ordinal(), 3);

    let serialized = serde_json::to_string(e0.agent_inputs()).expect("serialize agent inputs");
    assert!(!serialized.contains("labels"));
    assert!(!serialized.contains("final_status"));
    assert!(!serialized.contains("Resolved"));
    assert!(!serialized.contains("synthetic-customer"));
    assert!(!serialized.contains("private"));
    assert!(!serialized.contains("email@example.test"));
    assert!(!serialized.contains("{MONTO}"));
    assert!(e0.agent_inputs().facts().iter().any(|fact| matches!(
        fact,
        improvement_engine_source_adapters::E0Fact::ToolCall {
            case_ordinal: 1,
            state_change: Some(true),
            ..
        }
    )));
    assert!(e0.agent_inputs().facts().iter().any(|fact| matches!(
        fact,
        improvement_engine_source_adapters::E0Fact::Turn { .. }
    )));
    assert!(e0.agent_inputs().facts().iter().any(|fact| matches!(
        fact,
        improvement_engine_source_adapters::E0Fact::Case {
            case_ordinal: 1,
            topic,
            ..
        } if topic == "unknown"
    )));
    assert!(!serialized.contains("person@example.test"));
}

#[test]
fn e0_cutoff_excludes_future_cases_and_future_interaction_events_before_split() {
    let temp = TempDir::new().expect("temp dir");
    e0_fixture(temp.path());

    let prepared = prepare_e0_package(temp.path(), &config_with_cutoff(1, "1970-01-01T00:00:20Z"))
        .expect("prepare cutoff-limited source");
    let inputs = prepared.agent_inputs();
    assert_eq!(inputs.cases().len(), 2);
    assert_eq!(inputs.cases()[0].phase(), CasePhase::Arranque);
    assert_eq!(inputs.cases()[1].phase(), CasePhase::Reproduccion);
    assert!(inputs.facts().iter().all(|fact| match fact {
        improvement_engine_source_adapters::E0Fact::Case { opened_at, .. } =>
            opened_at.as_str() <= "1970-01-01T00:00:20Z",
        improvement_engine_source_adapters::E0Fact::ToolCall {
            event_time_unix_micros,
            ..
        } => *event_time_unix_micros <= 20_000_000,
        improvement_engine_source_adapters::E0Fact::Signal { window_end, .. } =>
            window_end.as_str() <= "1970-01-01T00:00:20Z",
        improvement_engine_source_adapters::E0Fact::IdentityCheck { event_time, .. }
        | improvement_engine_source_adapters::E0Fact::Turn { event_time, .. }
        | improvement_engine_source_adapters::E0Fact::RoutingStep { event_time, .. }
        | improvement_engine_source_adapters::E0Fact::CopilotQuery { event_time, .. } =>
            event_time.as_str() <= "1970-01-01T00:00:20Z",
        improvement_engine_source_adapters::E0Fact::Approval { requested_at, .. } =>
            requested_at.as_str() <= "1970-01-01T00:00:20Z",
    }));
    assert!(
        inputs.cases()[1].events().is_empty(),
        "case B tool call at 20.5 seconds is after cutoff second 20"
    );
}

#[test]
fn evaluator_labels_are_loaded_through_a_separate_type_after_agent_preparation() {
    let temp = TempDir::new().expect("temp dir");
    e0_fixture(temp.path());
    let prepared = prepare_e0_package(temp.path(), &config(2)).expect("prepare E0");
    let labels = evaluator::load_labels(temp.path()).expect("load evaluator labels explicitly");

    assert_eq!(prepared.agent_inputs().cases().len(), 3);
    assert_eq!(labels.rows().len(), 3);
    assert_eq!(labels.rows()[0].rank(), 1);
    assert_eq!(labels.rows()[0].final_status(), "Resolved");
    let serialized =
        serde_json::to_string(prepared.agent_inputs()).expect("serialize agent inputs");
    assert!(!serialized.contains("Resolved"));
    assert!(prepared.manifest_digest().starts_with("sha256:"));
}

#[test]
fn evaluator_label_bytes_do_not_change_agent_discovery_manifest() {
    let temp = TempDir::new().expect("temp dir");
    e0_fixture(temp.path());
    let first = prepare_e0_package(temp.path(), &config(2)).expect("prepare initial source");

    let labels = Schema::new(vec![
        Field::new("case_id", DataType::Utf8, false),
        Field::new("rank", DataType::Int64, false),
        Field::new("split", DataType::Utf8, false),
        Field::new("final_status", DataType::Utf8, false),
        Field::new("final_sla_breached", DataType::Boolean, false),
    ]);
    write_parquet(
        &temp.path().join("datos/labels.parquet"),
        labels,
        vec![
            Arc::new(StringArray::from(vec!["case-a", "case-b", "case-c"])),
            Arc::new(arrow_array::Int64Array::from(vec![1_i64, 2, 3])),
            Arc::new(StringArray::from(vec![
                "arranque",
                "arranque",
                "reproduccion",
            ])),
            Arc::new(StringArray::from(vec!["Changed", "Changed", "Changed"])),
            Arc::new(BooleanArray::from(vec![true, false, true])),
        ],
    );
    let second = prepare_e0_package(temp.path(), &config(2)).expect("prepare changed evaluator");

    assert_eq!(first.manifest_digest(), second.manifest_digest());
    assert_eq!(first.agent_inputs(), second.agent_inputs());
    assert_ne!(
        evaluator::load_labels(temp.path()).unwrap().rows()[0].final_status(),
        "Resolved"
    );
}
