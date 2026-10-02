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
    assert_eq!(result["excluded_replay_case_count"], 1);
    assert_eq!(result["terminal_status"], "complete_no_opportunity");
    assert!(result["candidates"].as_array().unwrap().is_empty());
    assert!(result["proposal"].is_null());
    assert!(result["evaluation"].is_null());
    assert!(timeline.contains("no_opportunity"));
    assert!(!timeline.contains("improvement_draft"));
    assert!(!timeline.contains("jev_or_agent_core_scout"));
}
