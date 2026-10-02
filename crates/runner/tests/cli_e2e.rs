use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Int32Array, Int64Array, RecordBatch, StringArray,
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

fn e0_fixture(root: &Path) {
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
            Arc::new(StringArray::from(vec!["private-case-id"])),
            Arc::new(TimestampMicrosecondArray::from(vec![at]).with_timezone("UTC")),
            Arc::new(StringArray::from(vec!["chat"])),
            Arc::new(StringArray::from(vec!["es"])),
            Arc::new(StringArray::from(vec!["payments"])),
            Arc::new(StringArray::from(vec!["normal"])),
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
            Field::new("state_change", DataType::Boolean, true),
            Field::new("retry_count", DataType::Int32, true),
            Field::new("latency_ms", DataType::Int64, true),
        ]),
        vec![
            Arc::new(StringArray::from(vec!["private-case-id"])),
            Arc::new(TimestampMicrosecondArray::from(vec![at + 1]).with_timezone("UTC")),
            Arc::new(StringArray::from(vec!["private-tool-call-id"])),
            Arc::new(StringArray::from(vec!["tree"])),
            Arc::new(StringArray::from(vec!["status_lookup"])),
            Arc::new(StringArray::from(vec!["read"])),
            Arc::new(StringArray::from(vec!["error"])),
            Arc::new(BooleanArray::from(vec![Some(false)])),
            Arc::new(BooleanArray::from(vec![None])),
            Arc::new(Int32Array::from(vec![Some(0)])),
            Arc::new(Int64Array::from(vec![Some(5)])),
        ],
    );
}

#[test]
fn binary_persists_simulated_result_and_timeline_without_source_identifiers() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("e0");
    let output = temp.path().join("runs");
    e0_fixture(&input);

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
    assert_eq!(result["evaluation"]["status"], "simulated");
    assert_eq!(result["proposal"]["status"], "simulated_unverified");
    assert_eq!(result["formal_route"], "do_nothing");
    assert!(timeline.contains("improvement_draft"));
    assert!(!serialized.contains("private-case-id"));
    assert!(!serialized.contains("private-tool-call-id"));
    assert!(!serialized.contains("\"label\""));
    assert!(!serialized.contains("final_sla_breached"));
}
