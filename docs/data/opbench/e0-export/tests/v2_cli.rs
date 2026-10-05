use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arrow_array::{
    ArrayRef, BooleanArray, Int32Array, Int64Array, RecordBatch, StringArray,
    TimestampMicrosecondArray, TimestampMillisecondArray,
};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use parquet::arrow::ArrowWriter;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const CASES: usize = 2_000;
const CUTOFF_MICROS: i64 = 1_791_158_400_000_000;
const TABLES: [&str; 9] = [
    "case",
    "identity_check",
    "turn",
    "routing_step",
    "copilot_query",
    "tool_call",
    "approval",
    "case_close",
    "signal",
];

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct FixtureRoot(PathBuf);

impl FixtureRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "opbench-e0-v2-cli-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create synthetic CLI fixture root");
        Self(path)
    }

    fn package(&self) -> PathBuf {
        self.0.join("package")
    }

    fn data(&self) -> PathBuf {
        self.package().join("datos")
    }

    fn digest_file(&self) -> PathBuf {
        self.0.join("complaint-digests.txt")
    }
}

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn table_fields(
    table: &str,
    include_complaint_id: bool,
    timestamp_unit: TimeUnit,
) -> Vec<(String, DataType, bool)> {
    let mut fields = Vec::new();
    let id = match table {
        "case" | "case_close" => "case_id",
        "identity_check" => "check_id",
        "turn" => "turn_id",
        "routing_step" => "step_id",
        "copilot_query" => "query_id",
        "tool_call" => "call_id",
        "approval" => "approval_id",
        "signal" => "signal_id",
        _ => unreachable!(),
    };
    fields.push((id.to_owned(), DataType::Utf8, false));
    if !matches!(table, "case" | "signal" | "case_close") {
        fields.push(("case_id".to_owned(), DataType::Utf8, false));
    }
    let timestamp = |unit| DataType::Timestamp(unit, Some("UTC".into()));
    match table {
        "case" => {
            fields.push(("opened_at".into(), timestamp(timestamp_unit), false));
            fields.push(("customer_id".into(), DataType::Utf8, false));
            fields.push(("channel".into(), DataType::Utf8, false));
            fields.push(("language".into(), DataType::Utf8, false));
            fields.push(("topic".into(), DataType::Utf8, false));
            fields.push(("priority".into(), DataType::Utf8, false));
            if include_complaint_id {
                fields.push(("complaint_id".into(), DataType::Utf8, true));
            }
        }
        "identity_check" => {
            fields.push(("started_at".into(), timestamp(TimeUnit::Microsecond), false));
            fields.push(("ended_at".into(), timestamp(TimeUnit::Microsecond), false));
        }
        "turn" => {
            fields.push(("event_time".into(), timestamp(TimeUnit::Microsecond), false));
            fields.push(("evidence_ids".into(), DataType::Utf8, true));
        }
        "routing_step" => {
            fields.push(("event_time".into(), timestamp(TimeUnit::Microsecond), false))
        }
        "copilot_query" => {
            fields.push(("event_time".into(), timestamp(TimeUnit::Microsecond), false));
            fields.push(("query_signature".into(), DataType::Utf8, false));
            fields.push(("answered_by".into(), DataType::Utf8, false));
        }
        "tool_call" => {
            fields.push(("event_time".into(), timestamp(TimeUnit::Microsecond), false));
            fields.push(("actor_role".into(), DataType::Utf8, false));
            fields.push(("tool_id".into(), DataType::Utf8, false));
            fields.push(("permission_level".into(), DataType::Utf8, false));
            fields.push(("status".into(), DataType::Utf8, false));
            fields.push(("verified".into(), DataType::Boolean, true));
            fields.push(("state_change".into(), DataType::Utf8, true));
            fields.push(("retry_count".into(), DataType::Int32, true));
            fields.push(("latency_ms".into(), DataType::Int64, true));
            fields.push(("approval_id".into(), DataType::Utf8, true));
        }
        "approval" => {
            fields.push((
                "requested_at".into(),
                timestamp(TimeUnit::Microsecond),
                false,
            ));
            fields.push(("decided_at".into(), timestamp(TimeUnit::Microsecond), true));
            fields.push(("executed_call_id".into(), DataType::Utf8, true));
        }
        "case_close" => {
            fields.push(("closed_at".into(), timestamp(TimeUnit::Microsecond), false));
            fields.push(("resolved".into(), DataType::Boolean, false));
        }
        "signal" => {
            fields.push((
                "window_start".into(),
                timestamp(TimeUnit::Microsecond),
                false,
            ));
            fields.push(("window_end".into(), timestamp(TimeUnit::Microsecond), false));
            fields.push(("support_cases".into(), DataType::Int32, false));
            fields.push(("evidence_case_ids".into(), DataType::Utf8, false));
        }
        _ => unreachable!(),
    }
    fields
}

fn parquet_type(data_type: &DataType) -> &'static str {
    match data_type {
        DataType::Utf8 | DataType::LargeUtf8 => "VARCHAR",
        DataType::Timestamp(_, _) => "TIMESTAMP",
        DataType::Boolean => "BOOLEAN",
        DataType::Int32 | DataType::Int64 => "INTEGER",
        _ => unreachable!("fixture uses only supported contract types"),
    }
}

fn contract(include_complaint_id: bool, timestamp_unit: TimeUnit) -> Value {
    let mut entities = Map::new();
    for table in TABLES {
        let mut columns = Map::new();
        for (name, data_type, nullable) in table_fields(table, include_complaint_id, timestamp_unit)
        {
            columns.insert(
                name,
                json!({"type": parquet_type(&data_type), "required": !nullable}),
            );
        }
        entities.insert(table.into(), json!({"fields": columns}));
    }
    json!({"name":"platform_history","version":"synthetic-test","entities":entities})
}

fn write_table(
    data: &Path,
    table: &str,
    row_count: usize,
    include_complaint_id: bool,
    case_timestamp_unit: TimeUnit,
    null_first_complaint: bool,
    duplicate_last_case: bool,
) {
    let fields = table_fields(table, include_complaint_id, case_timestamp_unit);
    let schema = Arc::new(Schema::new(
        fields
            .iter()
            .map(|(name, data_type, nullable)| Field::new(name, data_type.clone(), *nullable))
            .collect::<Vec<_>>(),
    ));
    let mut arrays: Vec<ArrayRef> = Vec::new();
    for (name, data_type, _) in &fields {
        let array: ArrayRef = match data_type {
            DataType::Utf8 => {
                if name == "complaint_id" && null_first_complaint && row_count > 0 {
                    let mut values = (0..row_count)
                        .map(|i| Some(format!("SYNTH-COMPLAINT-{i:04}")))
                        .collect::<Vec<_>>();
                    values[0] = None;
                    Arc::new(StringArray::from(values))
                } else {
                    let values = (0..row_count)
                        .map(|i| {
                            let effective =
                                if duplicate_last_case && table == "case" && i + 1 == row_count {
                                    0
                                } else {
                                    i
                                };
                            match name.as_str() {
                                "case_id" => format!("SYNTH-PRIVATE-CASE-{effective:04}"),
                                "complaint_id" => format!("SYNTH-PRIVATE-COMPLAINT-{effective:04}"),
                                "customer_id" => format!("SYNTH-PRIVATE-CUSTOMER-{effective:04}"),
                                "query_id" => format!("SYNTH-PRIVATE-QUERY-{effective:04}"),
                                "query_signature" => "SYNTH-PRIVATE-SIGNATURE".to_owned(),
                                "answered_by" => "tree:synthetic".to_owned(),
                                "channel" => "app_chat".to_owned(),
                                "language" => "es".to_owned(),
                                "topic" => "synthetic_support".to_owned(),
                                "priority" => "normal".to_owned(),
                                "evidence_ids" | "evidence_case_ids" => "[]".to_owned(),
                                _ => format!("synthetic-{table}-{effective:04}"),
                            }
                        })
                        .collect::<Vec<_>>();
                    Arc::new(StringArray::from(
                        values.iter().map(String::as_str).collect::<Vec<_>>(),
                    ))
                }
            }
            DataType::Timestamp(TimeUnit::Microsecond, _) => {
                let values = (0..row_count)
                    .map(|i| {
                        if table == "case" && name == "opened_at" && i + 1 == row_count {
                            CUTOFF_MICROS
                        } else if table == "case" && name == "opened_at" {
                            CUTOFF_MICROS - 1_000
                        } else {
                            CUTOFF_MICROS
                        }
                    })
                    .collect::<Vec<_>>();
                Arc::new(TimestampMicrosecondArray::from(values).with_timezone("UTC"))
            }
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                let values = (0..row_count)
                    .map(|i| {
                        if table == "case" && name == "opened_at" && i + 1 == row_count {
                            CUTOFF_MICROS / 1_000
                        } else if table == "case" && name == "opened_at" {
                            CUTOFF_MICROS / 1_000 - 1
                        } else {
                            CUTOFF_MICROS / 1_000
                        }
                    })
                    .collect::<Vec<_>>();
                Arc::new(TimestampMillisecondArray::from(values).with_timezone("UTC"))
            }
            DataType::Boolean => Arc::new(BooleanArray::from(vec![Some(true); row_count])),
            DataType::Int32 => Arc::new(Int32Array::from(vec![1; row_count])),
            DataType::Int64 => Arc::new(Int64Array::from(vec![1; row_count])),
            _ => unreachable!("fixture only contains described arrays"),
        };
        arrays.push(array);
    }
    let batch =
        RecordBatch::try_new(schema.clone(), arrays).expect("valid synthetic package batch");
    let file = File::create(data.join(format!("{table}.parquet"))).expect("create table file");
    let mut writer = ArrowWriter::try_new(file, schema, None).expect("create parquet writer");
    writer.write(&batch).expect("write table batch");
    writer.close().expect("close table parquet");
}

fn fixture(
    include_complaint_id: bool,
    timestamp_unit: TimeUnit,
    null_first_complaint: bool,
    duplicate_last_case: bool,
) -> FixtureRoot {
    let root = FixtureRoot::new();
    fs::create_dir_all(root.data()).expect("create synthetic data dir");
    fs::create_dir_all(root.package().join("contratos")).expect("create synthetic contract dir");
    fs::write(
        root.package().join("contratos/platform_history.json"),
        serde_json::to_vec(&contract(include_complaint_id, timestamp_unit)).unwrap(),
    )
    .expect("write synthetic platform contract");
    for table in TABLES {
        let rows = if table == "case" || table == "copilot_query" {
            CASES
        } else {
            0
        };
        write_table(
            &root.data(),
            table,
            rows,
            include_complaint_id,
            timestamp_unit,
            null_first_complaint,
            duplicate_last_case,
        );
    }
    let digests = (0..CASES)
        .map(|index| {
            let complaint_id = format!("SYNTH-PRIVATE-COMPLAINT-{index:04}");
            format!("{:x}", Sha256::digest(complaint_id.as_bytes()))
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.digest_file(), digests).expect("write synthetic complaint digests");
    root
}

fn run_cli(root: &FixtureRoot) -> Output {
    Command::new(env!("CARGO_BIN_EXE_opbench-e0-export"))
        .arg(root.data())
        .arg("--v2")
        .arg(root.digest_file())
        .output()
        .expect("run synthetic E0 V2 exporter binary")
}

fn assert_sanitized_failure(output: Output, root: &FixtureRoot) {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("OPBENCH E0 v2 failed safely"));
    assert!(!stderr.contains(&root.0.to_string_lossy().to_string()));
    for private in [
        "SYNTH-PRIVATE-CASE",
        "SYNTH-PRIVATE-COMPLAINT",
        "SYNTH-PRIVATE-SIGNATURE",
    ] {
        assert!(!stderr.contains(private));
    }
}

#[test]
fn v2_binary_emits_only_four_aggregates_and_includes_exact_cutoff_case() {
    let root = fixture(true, TimeUnit::Microsecond, false, false);
    let output = run_cli(&root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8 JSON");
    for private in [
        "SYNTH-PRIVATE-CASE",
        "SYNTH-PRIVATE-COMPLAINT",
        "SYNTH-PRIVATE-CUSTOMER",
        "SYNTH-PRIVATE-SIGNATURE",
    ] {
        assert!(!stdout.contains(private));
    }
    let result: Value = serde_json::from_str(&stdout).expect("parse aggregate stdout");
    let keys = result
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        keys,
        [
            "discovery",
            "replication",
            "complaint_ids_matched",
            "eligible_cases"
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(result["discovery"]["selected"], 200);
    assert_eq!(result["discovery"]["total"], 200);
    assert_eq!(result["replication"]["selected"], 1_800);
    assert_eq!(result["replication"]["total"], 1_800);
    assert_eq!(result["complaint_ids_matched"], 2_000);
    assert_eq!(result["eligible_cases"], 2_000);
}

#[test]
fn v2_binary_rejects_source_duplicate_case_ids_without_echoing_paths_or_rows() {
    let root = fixture(true, TimeUnit::Microsecond, false, true);
    assert_sanitized_failure(run_cli(&root), &root);
}

#[test]
fn v2_binary_fails_closed_on_missing_column_null_linkage_and_wrong_timestamp_unit() {
    let missing = fixture(false, TimeUnit::Microsecond, false, false);
    assert_sanitized_failure(run_cli(&missing), &missing);

    let null_link = fixture(true, TimeUnit::Microsecond, true, false);
    assert_sanitized_failure(run_cli(&null_link), &null_link);

    let wrong_type = fixture(true, TimeUnit::Millisecond, false, false);
    assert_sanitized_failure(run_cli(&wrong_type), &wrong_type);
}

#[test]
fn v2_binary_fails_closed_on_missing_and_malformed_source_files() {
    let missing = fixture(true, TimeUnit::Microsecond, false, false);
    fs::remove_file(missing.data().join("copilot_query.parquet")).unwrap();
    assert_sanitized_failure(run_cli(&missing), &missing);

    let malformed = fixture(true, TimeUnit::Microsecond, false, false);
    fs::write(malformed.data().join("case.parquet"), b"not parquet").unwrap();
    assert_sanitized_failure(run_cli(&malformed), &malformed);
}

