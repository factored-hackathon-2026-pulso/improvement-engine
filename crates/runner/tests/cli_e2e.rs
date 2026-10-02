use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Int32Array, Int64Array, LargeStringArray, RecordBatch,
    TimestampMicrosecondArray,
};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use parquet::arrow::ArrowWriter;
use tempfile::TempDir;

fn write_parquet(path: &Path, schema: Schema, columns: Vec<ArrayRef>) {
    let schema = Arc::new(schema);
    let batch = RecordBatch::try_new(schema.clone(), columns).expect("fixture batch");
    let file = File::create(path).expect("create parquet fixture");
    let mut writer = ArrowWriter::try_new(file, schema, None).expect("create parquet writer");
    writer.write(&batch).expect("write parquet fixture");
    writer.close().expect("close parquet writer");
}

fn large_strings(values: Vec<&str>) -> ArrayRef {
    Arc::new(LargeStringArray::from(values))
}

fn e0_fixture(root: &Path, discovery_status: &str, replay_status: &str) {
    let data = root.join("datos");
    fs::create_dir_all(&data).expect("create data directory");
    fs::create_dir_all(root.join("contratos")).expect("create contracts directory");
    fs::write(
        root.join("contratos/platform_history.json"),
        "{\"fixture\":\"synthetic\"}",
    )
    .expect("write non-sensitive source contract");
    let at = 1_750_000_000_000_000_i64;
    write_parquet(
        &data.join("case.parquet"),
        Schema::new(vec![
            Field::new("case_id", DataType::LargeUtf8, false),
            Field::new(
                "opened_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ),
            Field::new("channel", DataType::LargeUtf8, false),
            Field::new("language", DataType::LargeUtf8, false),
            Field::new("topic", DataType::LargeUtf8, false),
            Field::new("priority", DataType::LargeUtf8, false),
        ]),
        vec![
            large_strings(vec!["private-case-id", "private-replay-case-id"]),
            Arc::new(TimestampMicrosecondArray::from(vec![at, at + 10]).with_timezone("UTC")),
            large_strings(vec!["chat", "chat"]),
            large_strings(vec!["es", "es"]),
            large_strings(vec!["payments", "payments"]),
            large_strings(vec!["normal", "normal"]),
        ],
    );
    write_parquet(
        &data.join("tool_call.parquet"),
        Schema::new(vec![
            Field::new("case_id", DataType::LargeUtf8, false),
            Field::new(
                "event_time",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ),
            Field::new("call_id", DataType::LargeUtf8, false),
            Field::new("actor_role", DataType::LargeUtf8, false),
            Field::new("tool_id", DataType::LargeUtf8, false),
            Field::new("permission_level", DataType::LargeUtf8, false),
            Field::new("status", DataType::LargeUtf8, false),
            Field::new("verified", DataType::Boolean, true),
            Field::new("state_change", DataType::LargeUtf8, true),
            Field::new("retry_count", DataType::Int32, true),
            Field::new("latency_ms", DataType::Int64, true),
        ]),
        vec![
            large_strings(vec!["private-case-id", "private-replay-case-id"]),
            Arc::new(TimestampMicrosecondArray::from(vec![at + 1, at + 11]).with_timezone("UTC")),
            large_strings(vec!["private-tool-call-id", "private-replay-call-id"]),
            large_strings(vec!["tree", "tree"]),
            large_strings(vec!["status_lookup", "status_lookup"]),
            large_strings(vec!["read", "read"]),
            large_strings(vec![discovery_status, replay_status]),
            Arc::new(BooleanArray::from(vec![Some(false), Some(false)])),
            large_strings(vec!["{}", "{}"]),
            Arc::new(Int32Array::from(vec![Some(0), Some(0)])),
            Arc::new(Int64Array::from(vec![Some(5), Some(5)])),
        ],
    );
}

fn e0_recurrence_fixture(root: &Path, replay_signature: &str) {
    e0_fixture(root, "ok", "ok");
    let data = root.join("datos");
    let case_ids = (1..=22)
        .map(|ordinal| format!("private-case-{ordinal}"))
        .collect::<Vec<_>>();
    let case_id_refs = case_ids.iter().map(String::as_str).collect::<Vec<_>>();
    let at = 1_750_000_000_000_000_i64;
    write_parquet(
        &data.join("case.parquet"),
        Schema::new(vec![
            Field::new("case_id", DataType::LargeUtf8, false),
            Field::new(
                "opened_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ),
            Field::new("channel", DataType::LargeUtf8, false),
            Field::new("language", DataType::LargeUtf8, false),
            Field::new("topic", DataType::LargeUtf8, false),
            Field::new("priority", DataType::LargeUtf8, false),
        ]),
        vec![
            large_strings(case_id_refs.clone()),
            Arc::new(
                TimestampMicrosecondArray::from(
                    (0..22)
                        .map(|offset| at + offset * 1_000_000)
                        .collect::<Vec<_>>(),
                )
                .with_timezone("UTC"),
            ),
            large_strings(vec!["chat"; 22]),
            large_strings(vec!["es"; 22]),
            large_strings(vec!["support"; 22]),
            large_strings(vec!["normal"; 22]),
        ],
    );
    let call_ids = (1..=22)
        .map(|ordinal| format!("private-call-{ordinal}"))
        .collect::<Vec<_>>();
    write_parquet(
        &data.join("tool_call.parquet"),
        Schema::new(vec![
            Field::new("case_id", DataType::LargeUtf8, false),
            Field::new(
                "event_time",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ),
            Field::new("call_id", DataType::LargeUtf8, false),
            Field::new("actor_role", DataType::LargeUtf8, false),
            Field::new("tool_id", DataType::LargeUtf8, false),
            Field::new("permission_level", DataType::LargeUtf8, false),
            Field::new("status", DataType::LargeUtf8, false),
            Field::new("verified", DataType::Boolean, true),
            Field::new("state_change", DataType::LargeUtf8, true),
            Field::new("retry_count", DataType::Int32, true),
            Field::new("latency_ms", DataType::Int64, true),
        ]),
        vec![
            large_strings(case_id_refs),
            Arc::new(
                TimestampMicrosecondArray::from(
                    (0..22)
                        .map(|offset| at + offset * 1_000_000 + 10)
                        .collect::<Vec<_>>(),
                )
                .with_timezone("UTC"),
            ),
            large_strings(call_ids.iter().map(String::as_str).collect()),
            large_strings(vec!["tree"; 22]),
            large_strings(vec!["status_lookup"; 22]),
            large_strings(vec!["read"; 22]),
            large_strings(vec!["ok"; 22]),
            Arc::new(BooleanArray::from(vec![Some(true); 22])),
            large_strings(vec!["{}"; 22]),
            Arc::new(Int32Array::from(vec![Some(0); 22])),
            Arc::new(Int64Array::from(vec![Some(20); 22])),
        ],
    );
    let mut query_case_ids = (1..=20)
        .map(|ordinal| format!("private-case-{ordinal}"))
        .collect::<Vec<_>>();
    query_case_ids.extend([
        "private-case-1".into(),
        "private-case-21".into(),
        "private-case-22".into(),
    ]);
    let mut signatures = vec!["normalized-query-pattern"; 20];
    signatures.extend([
        "normalized-query-pattern",
        "unique-pattern",
        replay_signature,
    ]);
    let query_ids = (1..=23)
        .map(|ordinal| format!("private-query-{ordinal}"))
        .collect::<Vec<_>>();
    write_parquet(
        &data.join("copilot_query.parquet"),
        Schema::new(vec![
            Field::new("query_id", DataType::LargeUtf8, false),
            Field::new("case_id", DataType::LargeUtf8, false),
            Field::new(
                "event_time",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ),
            Field::new("query_signature", DataType::LargeUtf8, false),
            Field::new("answered_by", DataType::LargeUtf8, false),
        ]),
        vec![
            large_strings(query_ids.iter().map(String::as_str).collect()),
            large_strings(query_case_ids.iter().map(String::as_str).collect()),
            Arc::new(
                TimestampMicrosecondArray::from(
                    (0..23)
                        .map(|offset| at + offset * 1_000_000 + 20)
                        .collect::<Vec<_>>(),
                )
                .with_timezone("UTC"),
            ),
            large_strings(signatures),
            large_strings(vec!["tool:status_lookup"; 23]),
        ],
    );
}

#[test]
fn binary_persists_simulated_result_and_timeline_without_source_identifiers() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("e0");
    let output = temp.path().join("runs");
    e0_fixture(&input, "error", "error");

    let completed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "e0",
            "--input",
        ])
        .arg(&input)
        .args(["--output"])
        .arg(&output)
        .args([
            "--tenant-id",
            "pulso_local",
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--arranque-cases",
            "1",
        ])
        .output()
        .expect("run CLI binary");
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );

    let run_dir = fs::read_dir(&output)
        .expect("output directory")
        .next()
        .expect("run directory")
        .expect("read run directory")
        .path();
    let result: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("result.json")).expect("result file"))
            .expect("valid result json");
    let timeline = fs::read_to_string(run_dir.join("events.ndjson")).expect("timeline file");
    let serialized = result.to_string();
    assert_eq!(result["execution_mode"], "local_simulation");
    assert_eq!(result["terminal_status"], "complete_simulated");
    assert_eq!(result["observed_cutoff_rfc3339"], "2025-07-01T00:00:00Z");
    assert_eq!(result["evaluation"]["status"], "simulated");
    assert_eq!(result["proposal"]["status"], "simulated_unverified");
    assert_eq!(result["formal_route"], "do_nothing");
    assert_eq!(result["discovery_case_count"], 1);
    assert_eq!(result["excluded_replay_case_count"], 1);
    assert_eq!(result["signal"]["numerator"], 1);
    assert_eq!(result["signal"]["denominator"], 1);
    assert!(timeline.contains("2025-07-01T00:00:00Z"));
    assert!(timeline.contains("improvement_draft"));
    assert!(!serialized.contains("private-case-id"));
    assert!(!serialized.contains("private-tool-call-id"));
    assert!(!serialized.contains("\"label\""));
    assert!(!serialized.contains("final_sla_breached"));

    let altered_input = temp.path().join("e0-altered-replay");
    let altered_output = temp.path().join("runs-altered");
    e0_fixture(&altered_input, "error", "ok");
    let altered_run = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "e0",
            "--input",
        ])
        .arg(&altered_input)
        .args(["--output"])
        .arg(&altered_output)
        .args([
            "--tenant-id",
            "pulso_local",
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--arranque-cases",
            "1",
        ])
        .output()
        .expect("run CLI with changed replay-only event");
    assert!(
        altered_run.status.success(),
        "{}",
        String::from_utf8_lossy(&altered_run.stderr)
    );
    let altered_dir = fs::read_dir(&altered_output)
        .expect("altered output directory")
        .next()
        .expect("altered run directory")
        .expect("read altered run directory")
        .path();
    let altered_result: serde_json::Value = serde_json::from_slice(
        &fs::read(altered_dir.join("result.json")).expect("altered result file"),
    )
    .expect("valid altered result");
    assert_eq!(
        altered_result["signal"]["numerator"],
        result["signal"]["numerator"]
    );
    assert_eq!(
        altered_result["signal"]["denominator"],
        result["signal"]["denominator"]
    );
    assert_eq!(
        altered_result["proposal"]["hypothesis"],
        result["proposal"]["hypothesis"]
    );
    assert_eq!(
        altered_result["candidates"].as_array().unwrap().len(),
        result["candidates"].as_array().unwrap().len()
    );
}

#[test]
fn binary_emits_no_opportunity_when_discovery_has_no_positive_support() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("e0-no-positive-signal");
    let output = temp.path().join("runs-no-positive-signal");
    e0_fixture(&input, "ok", "error");

    let completed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "e0",
            "--input",
        ])
        .arg(&input)
        .args(["--output"])
        .arg(&output)
        .args([
            "--tenant-id",
            "pulso_local",
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--arranque-cases",
            "1",
        ])
        .output()
        .expect("run CLI binary with no positive discovery signal");
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let run_dir = fs::read_dir(&output)
        .expect("output directory")
        .next()
        .expect("run directory")
        .expect("read run directory")
        .path();
    let result: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("result.json")).expect("result file"))
            .expect("valid result json");
    let timeline = fs::read_to_string(run_dir.join("events.ndjson")).expect("timeline file");

    assert_eq!(result["signal"]["numerator"], 0);
    assert_eq!(
        result["recurrence_measurement_status"],
        "source_table_unavailable"
    );
    assert!(
        result["signals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|signal| signal["metric_id"] != "e0_recurring_copilot_query_cases")
    );
    assert_eq!(result["excluded_replay_case_count"], 1);
    assert_eq!(result["terminal_status"], "complete_no_opportunity");
    assert!(result["candidates"].as_array().unwrap().is_empty());
    assert!(result["proposal"].is_null());
    assert!(result["evaluation"].is_null());
    assert!(timeline.contains("no_opportunity"));
    assert!(!timeline.contains("improvement_draft"));
    assert!(!timeline.contains("jev_or_agent_core_scout"));
}

#[test]
fn binary_detects_recurring_copilot_query_from_arranque_without_using_replay() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("e0-recurrence");
    let output = temp.path().join("runs-recurrence");
    e0_recurrence_fixture(&input, "normalized-query-pattern");

    let completed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "e0",
            "--input",
        ])
        .arg(&input)
        .args(["--output"])
        .arg(&output)
        .args([
            "--tenant-id",
            "pulso_local",
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--arranque-cases",
            "21",
        ])
        .output()
        .expect("run E0 recurrence fixture through CLI");
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let run_dir = fs::read_dir(&output)
        .expect("output directory")
        .next()
        .expect("run directory")
        .expect("read run directory")
        .path();
    let result: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("result.json")).expect("result file"))
            .expect("valid result JSON");
    let serialized = result.to_string();

    assert_eq!(result["terminal_status"], "complete_simulated");
    assert_eq!(result["excluded_replay_case_count"], 1);
    assert_eq!(result["primary_signal_policy"], "local_primary_signal_v2");
    assert_eq!(
        result["signal"]["metric_id"],
        "e0_recurring_copilot_query_cases"
    );
    assert_eq!(result["signal"]["numerator"], 20);
    assert_eq!(result["signal"]["denominator"], 21);
    assert_eq!(result["signal"]["minimum_support"], 20);
    assert_eq!(result["proposal"]["status"], "simulated_unverified");
    assert_eq!(result["proposal"]["execution_status"], "not_executed");
    assert_eq!(result["formal_route"], "do_nothing");
    assert_eq!(result["signals"].as_array().unwrap().len(), 2);
    assert!(result["signal"]["pattern_ref"].as_str().is_some());
    assert!(!serialized.contains("private-case-"));
    assert!(!serialized.contains("private-query-"));
    assert!(!serialized.contains("normalized-query-pattern"));

    fs::write(
        input.join("datos/signal.parquet"),
        b"deliberately invalid bytes: discovery must never read signal.parquet",
    )
    .expect("add excluded platform signal file");
    let changed_output = temp.path().join("runs-recurrence-with-signal-table");
    let changed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "e0",
            "--input",
        ])
        .arg(&input)
        .args(["--output"])
        .arg(&changed_output)
        .args([
            "--tenant-id",
            "pulso_local",
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--arranque-cases",
            "21",
        ])
        .output()
        .expect("rerun with excluded platform signal file");
    assert!(
        changed.status.success(),
        "{}",
        String::from_utf8_lossy(&changed.stderr)
    );
    let changed_dir = fs::read_dir(&changed_output)
        .expect("changed output directory")
        .next()
        .expect("changed run directory")
        .expect("read changed run directory")
        .path();
    let changed_result: serde_json::Value = serde_json::from_slice(
        &fs::read(changed_dir.join("result.json")).expect("changed result file"),
    )
    .expect("valid changed result JSON");
    assert_eq!(changed_result["manifest_digest"], result["manifest_digest"]);
    assert_eq!(
        changed_result["signal"]["metric_id"],
        result["signal"]["metric_id"]
    );
    assert_eq!(
        changed_result["signal"]["numerator"],
        result["signal"]["numerator"]
    );
    assert_eq!(
        changed_result["signal"]["denominator"],
        result["signal"]["denominator"]
    );
    assert_eq!(
        changed_result["signal"]["pattern_ref"],
        result["signal"]["pattern_ref"]
    );
    assert_eq!(changed_result["candidates"].as_array().unwrap().len(), 3);
}
