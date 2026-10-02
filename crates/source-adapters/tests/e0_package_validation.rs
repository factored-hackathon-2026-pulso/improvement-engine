use std::fs;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Int32Array, RecordBatch, StringArray, TimestampMicrosecondArray,
};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use improvement_engine_source_adapters::validate_e0_operational_package;
use parquet::arrow::ArrowWriter;
use serde_json::{Value, json};
use tempfile::TempDir;

fn write_parquet(path: &Path, schema: Schema, columns: Vec<ArrayRef>) {
    let schema = Arc::new(schema);
    let batch = RecordBatch::try_new(schema.clone(), columns).expect("valid fixture batch");
    let file = fs::File::create(path).expect("create parquet");
    let mut writer = ArrowWriter::try_new(file, schema, None).expect("writer");
    writer.write(&batch).expect("write parquet");
    writer.close().expect("close parquet");
}

fn write_tool_calls(root: &Path, case_id: &str, call_id: &str, event_time: i64) {
    let fields = fields("tool_call");
    let schema = Schema::new(
        fields
            .iter()
            .map(|(name, data_type, nullable)| Field::new(*name, data_type.clone(), *nullable))
            .collect::<Vec<_>>(),
    );
    let columns = fields
        .iter()
        .map(|(name, data_type, _)| -> ArrayRef {
            match data_type {
                DataType::Utf8 => Arc::new(StringArray::from(vec![Some(match *name {
                    "call_id" => call_id,
                    "case_id" => case_id,
                    "actor_role" => "tree",
                    "tool_id" => "read_txn",
                    "permission_level" => "read",
                    "status" => "ok",
                    _ => "row-1",
                })])),
                DataType::Boolean => Arc::new(BooleanArray::from(vec![Some(true)])),
                DataType::Int32 => Arc::new(Int32Array::from(vec![Some(1)])),
                DataType::Int64 => Arc::new(arrow_array::Int64Array::from(vec![Some(1)])),
                DataType::Timestamp(_, _) => {
                    Arc::new(TimestampMicrosecondArray::from(vec![event_time]).with_timezone("UTC"))
                }
                _ => unreachable!(),
            }
        })
        .collect();
    write_parquet(&root.join("datos/tool_call.parquet"), schema, columns);
}

fn fields(table: &str) -> Vec<(&'static str, DataType, bool)> {
    let id = match table {
        "case" => "case_id",
        "identity_check" => "check_id",
        "turn" => "turn_id",
        "routing_step" => "step_id",
        "copilot_query" => "query_id",
        "tool_call" => "call_id",
        "approval" => "approval_id",
        "case_close" => "case_id",
        "signal" => "signal_id",
        _ => unreachable!(),
    };
    let mut result = vec![(id, DataType::Utf8, false)];
    if table != "case" && table != "signal" && table != "case_close" {
        result.push(("case_id", DataType::Utf8, false));
    }
    match table {
        "case" => result.push((
            "opened_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        )),
        "identity_check" => {
            result.push((
                "started_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ));
            result.push((
                "ended_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ));
        }
        "turn" | "routing_step" | "copilot_query" | "tool_call" => result.push((
            "event_time",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        )),
        "approval" => {
            result.push((
                "requested_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ));
            result.push((
                "decided_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                true,
            ));
            result.push(("executed_call_id", DataType::Utf8, true));
        }
        "case_close" => {
            result.push((
                "closed_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ));
            result.push(("resolved", DataType::Boolean, false));
        }
        "signal" => {
            result.push((
                "window_start",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ));
            result.push((
                "window_end",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ));
            result.push(("support_cases", DataType::Int32, false));
            result.push(("evidence_case_ids", DataType::Utf8, false));
        }
        _ => unreachable!(),
    }
    if table == "case" {
        result.push(("customer_id", DataType::Utf8, false));
        result.push(("channel", DataType::Utf8, false));
        result.push(("language", DataType::Utf8, false));
        result.push(("topic", DataType::Utf8, false));
        result.push(("priority", DataType::Utf8, false));
    }
    if table == "turn" {
        result.push(("evidence_ids", DataType::Utf8, true));
    }
    if table == "tool_call" {
        result.push(("actor_role", DataType::Utf8, false));
        result.push(("tool_id", DataType::Utf8, false));
        result.push(("permission_level", DataType::Utf8, false));
        result.push(("status", DataType::Utf8, false));
        result.push(("verified", DataType::Boolean, true));
        result.push(("state_change", DataType::Utf8, true));
        result.push(("retry_count", DataType::Int32, true));
        result.push(("latency_ms", DataType::Int64, true));
        result.push(("approval_id", DataType::Utf8, true));
    }
    result
}

fn case_columns(case_ids: &[&str], opened_at: &[i64]) -> Vec<ArrayRef> {
    fields("case")
        .iter()
        .map(|(name, data_type, _)| match (name, data_type) {
            (&"case_id", DataType::Utf8) => {
                Arc::new(StringArray::from(case_ids.to_vec())) as ArrayRef
            }
            (&"customer_id", DataType::Utf8) => {
                Arc::new(StringArray::from(vec!["PRIVATE_CUSTOMER"; case_ids.len()]))
            }
            (&"channel", DataType::Utf8) => {
                Arc::new(StringArray::from(vec!["app_chat"; case_ids.len()]))
            }
            (&"language", DataType::Utf8) => {
                Arc::new(StringArray::from(vec!["es"; case_ids.len()]))
            }
            (&"topic", DataType::Utf8) => {
                Arc::new(StringArray::from(vec!["consultar_cargo"; case_ids.len()]))
            }
            (&"priority", DataType::Utf8) => {
                Arc::new(StringArray::from(vec!["medium"; case_ids.len()]))
            }
            (&"opened_at", DataType::Timestamp(_, _)) => {
                Arc::new(TimestampMicrosecondArray::from(opened_at.to_vec()).with_timezone("UTC"))
            }
            _ => unreachable!(),
        })
        .collect()
}

fn sample_array(name: &str, data_type: &DataType) -> ArrayRef {
    match data_type {
        DataType::Utf8 => Arc::new(StringArray::from(vec![Some(match name {
            "case_id" => "case-1",
            "customer_id" => "PII_SENTINEL_DO_NOT_PRINT",
            "channel" => "app_chat",
            "language" => "es",
            "topic" => "consultar_cargo",
            "priority" => "medium",
            "actor_role" => "tree",
            "tool_id" => "read_txn",
            "permission_level" => "read",
            "status" => "ok",
            "evidence_case_ids" => "[\"case-1\"]",
            "evidence_ids" => "[\"row-1\"]",
            "executed_call_id" | "approval_id" => "row-1",
            _ => "row-1",
        })])),
        DataType::Boolean => Arc::new(BooleanArray::from(vec![Some(true)])),
        DataType::Int32 => Arc::new(Int32Array::from(vec![Some(1)])),
        DataType::Int64 => Arc::new(arrow_array::Int64Array::from(vec![Some(1)])),
        DataType::Timestamp(_, _) => {
            Arc::new(TimestampMicrosecondArray::from(vec![20]).with_timezone("UTC"))
        }
        _ => unreachable!(),
    }
}

fn contract() -> Value {
    let mut entities = serde_json::Map::new();
    for table in [
        "case",
        "identity_check",
        "turn",
        "routing_step",
        "copilot_query",
        "tool_call",
        "approval",
        "case_close",
        "signal",
    ] {
        let mut contract_fields = serde_json::Map::new();
        for (name, data_type, nullable) in fields(table) {
            let type_name = match data_type {
                DataType::Utf8 => "VARCHAR",
                DataType::Boolean => "BOOLEAN",
                DataType::Int32 => "INTEGER",
                DataType::Int64 => "INTEGER",
                DataType::Timestamp(_, _) => "TIMESTAMP",
                _ => unreachable!(),
            };
            contract_fields.insert(
                name.to_owned(),
                json!({"type": type_name, "required": !nullable}),
            );
        }
        entities.insert(table.to_owned(), json!({"fields": contract_fields}));
    }
    json!({"name":"platform_history", "version":"test", "entities":entities})
}

fn fixture() -> TempDir {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path();
    fs::create_dir_all(root.join("datos")).unwrap();
    fs::create_dir_all(root.join("contratos")).unwrap();
    fs::write(
        root.join("contratos/platform_history.json"),
        contract().to_string(),
    )
    .unwrap();
    for table in [
        "case",
        "identity_check",
        "turn",
        "routing_step",
        "copilot_query",
        "tool_call",
        "approval",
        "case_close",
        "signal",
    ] {
        let columns = fields(table);
        let schema = Schema::new(
            columns
                .iter()
                .map(|(name, data_type, nullable)| Field::new(*name, data_type.clone(), *nullable))
                .collect::<Vec<_>>(),
        );
        let arrays = columns
            .iter()
            .map(|(name, data_type, _)| -> ArrayRef {
                match data_type {
                    DataType::Utf8 => Arc::new(StringArray::from(vec![Some(match *name {
                        "case_id" => "case-1",
                        "customer_id" => "PII_SENTINEL_DO_NOT_PRINT",
                        "evidence_case_ids" => "[\"case-1\"]",
                        "channel" => "app_chat",
                        "language" => "es",
                        "topic" => "consultar_cargo",
                        "priority" => "medium",
                        "actor_role" => "tree",
                        "tool_id" => "read_txn",
                        "permission_level" => "read",
                        "status" => "ok",
                        "evidence_ids" => "[\"row-1\"]",
                        "executed_call_id" | "approval_id" => "row-1",
                        _ => "row-1",
                    })])),
                    DataType::Boolean => Arc::new(BooleanArray::from(vec![Some(true)])),
                    DataType::Int32 => Arc::new(Int32Array::from(vec![Some(1)])),
                    DataType::Int64 => Arc::new(arrow_array::Int64Array::from(vec![Some(1)])),
                    DataType::Timestamp(_, _) => Arc::new(
                        TimestampMicrosecondArray::from(vec![match *name {
                            "opened_at" => 10,
                            "closed_at" | "window_end" => 30,
                            "ended_at" => 25,
                            _ => 20,
                        }])
                        .with_timezone("UTC"),
                    ),
                    _ => unreachable!(),
                }
            })
            .collect();
        write_parquet(&root.join(format!("datos/{table}.parquet")), schema, arrays);
    }
    temp
}

#[test]
fn validates_all_nine_operational_tables_without_loading_evaluator_files() {
    let temp = fixture();
    fs::write(
        temp.path().join("datos/labels.parquet"),
        b"not read by package validator",
    )
    .unwrap();
    fs::write(
        temp.path().join("datos/timeline.parquet"),
        b"also evaluator-only",
    )
    .unwrap();
    let result = validate_e0_operational_package(temp.path()).expect("valid operational package");
    assert_eq!(result.table_count(), 9);
    assert_eq!(result.row_count("case_close"), Some(1));
    assert_eq!(result.row_count("signal"), Some(1));
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains("PII_SENTINEL_DO_NOT_PRINT"));
    assert!(!serialized.contains("resolved"));
}

#[test]
fn validates_real_local_sample_when_root_is_explicitly_configured() {
    let Ok(root) = std::env::var("PULSO_E0_SAMPLE_ROOT") else {
        return;
    };
    let error = validate_e0_operational_package(Path::new(&root))
        .expect_err("real package currently contains a declared relation mismatch");
    let message = error.to_string();
    assert!(message.contains("table turn ("));
    assert!(message.contains("broken relationship"));
    assert!(!message.contains("PII"));
}

#[test]
fn rejects_missing_operational_table_and_never_reports_source_row_values() {
    let temp = fixture();
    fs::remove_file(temp.path().join("datos/case_close.parquet")).unwrap();
    let error = validate_e0_operational_package(temp.path()).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("case_close"));
    assert!(!message.contains("PII_SENTINEL_DO_NOT_PRINT"));
}

#[test]
fn rejects_dangling_case_relation_and_event_before_case_open() {
    let temp = fixture();
    write_tool_calls(temp.path(), "missing-case-id-private", "call-1", 5);
    let error = validate_e0_operational_package(temp.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("tool_call"));
    assert!(!error.contains("missing-case-id-private"));
}

#[test]
fn rejects_missing_column_and_null_in_required_field_even_when_arrow_schema_allows_null() {
    let temp = fixture();
    let incomplete_fields = fields("tool_call")
        .into_iter()
        .filter(|(name, _, _)| *name != "event_time")
        .collect::<Vec<_>>();
    let incomplete = Schema::new(
        incomplete_fields
            .iter()
            .map(|(name, data_type, nullable)| Field::new(*name, data_type.clone(), *nullable))
            .collect::<Vec<_>>(),
    );
    write_parquet(
        &temp.path().join("datos/tool_call.parquet"),
        incomplete,
        incomplete_fields
            .iter()
            .map(|(name, data_type, _)| sample_array(name, data_type))
            .collect(),
    );
    let error = validate_e0_operational_package(temp.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("tool_call") && error.contains("event_time"));

    let temp = fixture();
    let schema = Schema::new(
        fields("case")
            .iter()
            .map(|(name, data_type, nullable)| {
                Field::new(
                    *name,
                    data_type.clone(),
                    if *name == "opened_at" {
                        true
                    } else {
                        *nullable
                    },
                )
            })
            .collect::<Vec<_>>(),
    );
    let mut case_arrays = case_columns(&["case-1"], &[10]);
    let opened_index = fields("case")
        .iter()
        .position(|(name, _, _)| *name == "opened_at")
        .unwrap();
    case_arrays[opened_index] =
        Arc::new(TimestampMicrosecondArray::from(vec![None]).with_timezone("UTC"));
    write_parquet(&temp.path().join("datos/case.parquet"), schema, case_arrays);
    let error = validate_e0_operational_package(temp.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("case") && error.contains("opened_at"));
    assert!(error.contains("null values"));
    assert!(!error.contains("PII_SENTINEL_DO_NOT_PRINT"));
}

#[test]
fn rejects_physical_type_mismatch() {
    let temp = fixture();
    let mismatch_fields = fields("tool_call")
        .into_iter()
        .map(|(name, data_type, nullable)| {
            if name == "event_time" {
                (name, DataType::Utf8, nullable)
            } else {
                (name, data_type, nullable)
            }
        })
        .collect::<Vec<_>>();
    let schema = Schema::new(
        mismatch_fields
            .iter()
            .map(|(name, data_type, nullable)| Field::new(*name, data_type.clone(), *nullable))
            .collect::<Vec<_>>(),
    );
    write_parquet(
        &temp.path().join("datos/tool_call.parquet"),
        schema,
        mismatch_fields
            .iter()
            .map(|(name, data_type, _)| {
                if *name == "event_time" {
                    Arc::new(StringArray::from(vec!["1970-01-01T00:00:00Z"])) as ArrayRef
                } else {
                    sample_array(name, data_type)
                }
            })
            .collect(),
    );
    let error = validate_e0_operational_package(temp.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("tool_call") && error.contains("event_time"));
}

#[test]
fn rejects_event_before_open_and_duplicate_case_cardinality() {
    let temp = fixture();
    write_tool_calls(temp.path(), "case-1", "call-1", 5);
    let error = validate_e0_operational_package(temp.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("tool_call"));

    let temp = fixture();
    let schema = Schema::new(
        fields("case")
            .iter()
            .map(|(name, data_type, nullable)| Field::new(*name, data_type.clone(), *nullable))
            .collect::<Vec<_>>(),
    );
    write_parquet(
        &temp.path().join("datos/case.parquet"),
        schema,
        case_columns(&["case-1", "case-1"], &[10, 11]),
    );
    let error = validate_e0_operational_package(temp.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("case") && error.contains("cardinality"));
    assert!(!error.contains("private-customer"));
}

#[test]
fn validates_case_close_after_observed_events_and_signal_window_order() {
    let temp = fixture();
    let close_schema = Schema::new(
        fields("case_close")
            .iter()
            .map(|(name, data_type, nullable)| Field::new(*name, data_type.clone(), *nullable))
            .collect::<Vec<_>>(),
    );
    write_parquet(
        &temp.path().join("datos/case_close.parquet"),
        close_schema,
        vec![
            Arc::new(StringArray::from(vec!["case-1"])),
            Arc::new(TimestampMicrosecondArray::from(vec![15]).with_timezone("UTC")),
            Arc::new(BooleanArray::from(vec![true])),
        ],
    );
    let error = validate_e0_operational_package(temp.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("case_close"));

    let temp = fixture();
    let signal_schema = Schema::new(
        fields("signal")
            .iter()
            .map(|(name, data_type, nullable)| Field::new(*name, data_type.clone(), *nullable))
            .collect::<Vec<_>>(),
    );
    write_parquet(
        &temp.path().join("datos/signal.parquet"),
        signal_schema,
        vec![
            Arc::new(StringArray::from(vec!["signal-1"])),
            Arc::new(TimestampMicrosecondArray::from(vec![40]).with_timezone("UTC")),
            Arc::new(TimestampMicrosecondArray::from(vec![30]).with_timezone("UTC")),
            Arc::new(Int32Array::from(vec![1])),
            Arc::new(StringArray::from(vec!["[\"case-1\"]"])),
        ],
    );
    let error = validate_e0_operational_package(temp.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("signal"));
}

#[test]
fn case_close_and_signal_are_not_discovery_inputs_or_manifest_dependencies() {
    let temp = fixture();
    for table in [
        "identity_check",
        "turn",
        "routing_step",
        "copilot_query",
        "approval",
        "signal",
    ] {
        fs::remove_file(temp.path().join(format!("datos/{table}.parquet"))).unwrap();
    }
    let first = improvement_engine_source_adapters::prepare_e0_package(
        temp.path(),
        &improvement_engine_source_adapters::PreparationConfig::new(
            "tenant",
            "1970-01-01T00:01:00Z",
            1,
        )
        .unwrap(),
    )
    .expect("prepare discovery projection");
    fs::write(
        temp.path().join("datos/signal.parquet"),
        b"mutated out-of-band observation",
    )
    .unwrap();
    fs::write(
        temp.path().join("datos/case_close.parquet"),
        b"post-outcome values changed",
    )
    .unwrap();
    let second = improvement_engine_source_adapters::prepare_e0_package(
        temp.path(),
        &improvement_engine_source_adapters::PreparationConfig::new(
            "tenant",
            "1970-01-01T00:01:00Z",
            1,
        )
        .unwrap(),
    )
    .expect("prepare without consuming quarantined tables");
    assert_eq!(first.manifest_digest(), second.manifest_digest());
    assert_eq!(first.agent_inputs(), second.agent_inputs());
}
