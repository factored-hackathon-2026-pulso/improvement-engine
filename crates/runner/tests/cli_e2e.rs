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
    e0_fixture_with_retries(root, discovery_status, replay_status, [Some(0), Some(0)]);
}

fn e0_fixture_with_retries(
    root: &Path,
    discovery_status: &str,
    replay_status: &str,
    retry_counts: [Option<i32>; 2],
) {
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
            Arc::new(Int32Array::from(retry_counts.to_vec())),
            Arc::new(Int64Array::from(vec![Some(5), Some(5)])),
        ],
    );
}

fn e0_recurrence_fixture(
    root: &Path,
    replay_signature: &str,
    replay_query_cases: usize,
    matching_replay_cases: usize,
) {
    assert!(matching_replay_cases <= replay_query_cases);
    assert!(replay_query_cases <= 21);
    e0_fixture(root, "ok", "ok");
    let data = root.join("datos");
    let case_ids = (1..=42)
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
                    (0..42)
                        .map(|offset| at + offset * 1_000_000)
                        .collect::<Vec<_>>(),
                )
                .with_timezone("UTC"),
            ),
            large_strings(vec!["chat"; 42]),
            large_strings(vec!["es"; 42]),
            large_strings(vec!["support"; 42]),
            large_strings(vec!["normal"; 42]),
        ],
    );
    let call_ids = (1..=42)
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
                    (0..42)
                        .map(|offset| at + offset * 1_000_000 + 10)
                        .collect::<Vec<_>>(),
                )
                .with_timezone("UTC"),
            ),
            large_strings(call_ids.iter().map(String::as_str).collect()),
            large_strings(vec!["tree"; 42]),
            large_strings(vec!["status_lookup"; 42]),
            large_strings(vec!["read"; 42]),
            large_strings(vec!["ok"; 42]),
            Arc::new(BooleanArray::from(vec![Some(true); 42])),
            large_strings(vec!["{}"; 42]),
            Arc::new(Int32Array::from(vec![Some(0); 42])),
            Arc::new(Int64Array::from(vec![Some(20); 42])),
        ],
    );
    let mut query_case_ids = (1..=20)
        .map(|ordinal| format!("private-case-{ordinal}"))
        .collect::<Vec<_>>();
    query_case_ids.extend(["private-case-1".into(), "private-case-21".into()]);
    query_case_ids.extend(
        (22..=42)
            .take(replay_query_cases)
            .map(|ordinal| format!("private-case-{ordinal}")),
    );
    let mut signatures = vec!["normalized-query-pattern"; 20];
    signatures.extend(["normalized-query-pattern", "unique-pattern"]);
    signatures.extend(vec!["normalized-query-pattern"; matching_replay_cases]);
    signatures.extend(vec![
        replay_signature;
        replay_query_cases - matching_replay_cases
    ]);
    let query_ids = (1..=query_case_ids.len())
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
                    (0..query_ids.len())
                        .map(|offset| at + offset as i64 * 1_000_000 + 20)
                        .collect::<Vec<_>>(),
                )
                .with_timezone("UTC"),
            ),
            large_strings(signatures),
            large_strings(vec!["tool:status_lookup"; query_ids.len()]),
        ],
    );
}

fn run_e0_cli(input: &Path, output: &Path) -> serde_json::Value {
    run_e0_cli_with_arranque(input, output, "21")
}

fn run_e0_cli_with_arranque(
    input: &Path,
    output: &Path,
    arranque_cases: &str,
) -> serde_json::Value {
    let completed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "e0",
            "--input",
        ])
        .arg(input)
        .args(["--output"])
        .arg(output)
        .args([
            "--tenant-id",
            "pulso_local",
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--arranque-cases",
            arranque_cases,
        ])
        .output()
        .expect("run E0 fixture through CLI");
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let run_dir = fs::read_dir(output)
        .expect("output directory")
        .next()
        .expect("run directory")
        .expect("read run directory")
        .path();
    serde_json::from_slice(&fs::read(run_dir.join("result.json")).expect("result file"))
        .expect("valid result JSON")
}

#[test]
fn e0_cli_surfaces_retry_case_rate_with_known_denominator_and_missing_cases() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("e0-retries");
    let output = temp.path().join("runs-retries");
    e0_fixture_with_retries(&input, "ok", "ok", [Some(2), None]);

    let result = run_e0_cli_with_arranque(&input, &output, "2");
    let retry = result["signals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|signal| signal["metric_id"] == "e0_tool_retry_case_rate")
        .expect("retry signal is persisted");
    assert_eq!(retry["numerator"], 1);
    assert_eq!(retry["denominator"], 1);
    assert_eq!(retry["missing"], 1);
    assert_eq!(result["signal"]["metric_id"], "e0_tool_retry_case_rate");
    assert_eq!(result["proposal"]["status"], "simulated_unverified");
    assert_eq!(result["proposal"]["execution_status"], "not_executed");
    assert_eq!(
        result["proposal"]["proposed_artifact"]["observed_evidence"]["retry_cases"],
        1
    );
    assert_eq!(
        result["proposal"]["proposed_artifact"]["observed_evidence"]["retry_denominator_known_cases"],
        1
    );
    assert_eq!(
        result["proposal"]["proposed_artifact"]["observed_evidence"]["retry_cases_missing"],
        1
    );
    let overlap =
        &result["proposal"]["proposed_artifact"]["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["policy_id"], "e0_retry_error_overlap_k_v2");
    assert_eq!(overlap["status"], "insufficient_retry_status_coverage");
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert!(
        result
            .to_string()
            .contains("does not establish cause or savings")
    );
    assert!(!result.to_string().contains("private-case-id"));
}

#[test]
fn e0_cli_emits_opt_in_sanitized_live_progress_for_each_execution_phase() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("e0-progress");
    let output = temp.path().join("runs-progress");
    e0_fixture_with_retries(&input, "ok", "ok", [Some(2), None]);

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
            "2",
            "--progress-jsonl",
        ])
        .output()
        .expect("run E0 fixture through CLI with progress enabled");
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );

    let stderr = String::from_utf8(completed.stderr).expect("UTF-8 progress output");
    let progress = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("JSONL progress"))
        .collect::<Vec<_>>();
    let transitions = progress
        .iter()
        .map(|event| {
            (
                event["phase"].as_str().unwrap(),
                event["status"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        transitions,
        [
            ("source_preparation", "started"),
            ("source_preparation", "completed"),
            ("detection", "started"),
            ("detection", "completed"),
            ("post_selection_holdout", "started"),
            ("post_selection_holdout", "skipped"),
            ("persist_outputs", "started"),
            ("persist_outputs", "completed"),
        ]
    );
    assert!(progress.windows(2).all(|pair| {
        pair[0]["elapsed_ms"].as_u64().unwrap() <= pair[1]["elapsed_ms"].as_u64().unwrap()
    }));
    let phases = progress
        .iter()
        .filter(|event| event["status"] == "started")
        .map(|event| event["phase"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [
            "source_preparation",
            "detection",
            "post_selection_holdout",
            "persist_outputs"
        ]
    );
    assert!(progress.iter().all(|event| {
        event["schema_version"] == 1
            && event["event"] == "run_progress"
            && event["elapsed_ms"].is_number()
            && matches!(
                event["status"].as_str(),
                Some("started" | "completed" | "skipped")
            )
            && matches!(
                event["phase"].as_str(),
                Some(
                    "source_preparation"
                        | "detection"
                        | "post_selection_holdout"
                        | "persist_outputs"
                )
            )
    }));
    assert!(!stderr.contains(&input.display().to_string()));
    assert!(!stderr.contains("private-case-id"));
    assert!(!stderr.contains("private-replay-case-id"));
    assert!(progress.iter().any(|event| {
        event["phase"] == "post_selection_holdout" && event["status"] == "skipped"
    }));
}

#[test]
fn e0_cli_emits_failed_phase_without_continuing_when_source_preparation_fails() {
    let temp = TempDir::new().expect("temp directory");
    let missing_input = temp.path().join("missing-e0-source");
    let output = temp.path().join("runs-failed-progress");
    let completed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "e0",
            "--input",
        ])
        .arg(&missing_input)
        .args(["--output"])
        .arg(&output)
        .args([
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--progress-jsonl",
        ])
        .output()
        .expect("run CLI with missing source");
    assert!(!completed.status.success());

    let stderr = String::from_utf8(completed.stderr).expect("UTF-8 progress output");
    let progress = stderr
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("JSONL progress"))
        .collect::<Vec<_>>();
    assert_eq!(progress.len(), 2);
    assert_eq!(progress[0]["phase"], "source_preparation");
    assert_eq!(progress[0]["status"], "started");
    assert_eq!(progress[1]["phase"], "source_preparation");
    assert_eq!(progress[1]["status"], "failed");
    assert!(
        progress
            .iter()
            .all(|event| event["event"] == "run_progress" && event["schema_version"] == 1)
    );
    assert!(!stderr.contains("missing-e0-source"));
}

#[test]
fn e0_cli_reports_failed_output_persistence_after_detection_without_false_completion() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("e0-persist-failure");
    let output_file = temp.path().join("not-a-directory");
    e0_fixture_with_retries(&input, "ok", "ok", [Some(2), None]);
    fs::write(&output_file, "occupied").expect("create non-directory output target");

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
        .arg(&output_file)
        .args([
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--arranque-cases",
            "2",
            "--progress-jsonl",
        ])
        .output()
        .expect("run CLI with invalid output target");
    assert!(!completed.status.success());

    let stderr = String::from_utf8(completed.stderr).expect("UTF-8 progress output");
    let progress = stderr
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("JSONL progress"))
        .collect::<Vec<_>>();
    let transitions = progress
        .iter()
        .map(|event| {
            (
                event["phase"].as_str().unwrap(),
                event["status"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        transitions,
        [
            ("source_preparation", "started"),
            ("source_preparation", "completed"),
            ("detection", "started"),
            ("detection", "completed"),
            ("post_selection_holdout", "started"),
            ("post_selection_holdout", "skipped"),
            ("persist_outputs", "started"),
            ("persist_outputs", "failed"),
        ]
    );
    assert!(!stderr.contains(&output_file.display().to_string()));
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
    assert!(result["excluded_replay_case_count"].is_null());
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
    assert!(result["excluded_replay_case_count"].is_null());
    assert_eq!(result["terminal_status"], "complete_no_opportunity");
    assert!(result["candidates"].as_array().unwrap().is_empty());
    assert!(result["proposal"].is_null());
    assert!(result["evaluation"].is_null());
    assert!(result["e0_recurrence_holdout"].is_null());
    assert!(timeline.contains("no_opportunity"));
    assert!(!timeline.contains("improvement_draft"));
    assert!(!timeline.contains("jev_or_agent_core_scout"));
}

#[test]
fn binary_detects_recurring_copilot_query_from_arranque_without_using_replay() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("e0-recurrence");
    let output = temp.path().join("runs-recurrence");
    e0_recurrence_fixture(&input, "normalized-query-pattern", 21, 21);

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
    let timeline: Vec<serde_json::Value> = fs::read_to_string(run_dir.join("events.ndjson"))
        .expect("timeline file")
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid timeline event"))
        .collect();

    assert_eq!(result["terminal_status"], "complete_simulated");
    assert!(result["excluded_replay_case_count"].is_null());
    assert_eq!(result["primary_signal_policy"], "local_primary_signal_v3");
    assert_eq!(
        result["signal"]["metric_id"],
        "e0_recurring_copilot_query_cases"
    );
    assert_eq!(result["signal"]["numerator"], 20);
    assert_eq!(result["signal"]["denominator"], 21);
    assert_eq!(result["signal"]["minimum_support"], 20);
    assert_eq!(result["e0_recurrence_holdout"]["status"], "replicated");
    assert_eq!(
        result["e0_recurrence_holdout"]["reproduction_case_count"],
        21
    );
    assert_eq!(result["e0_recurrence_holdout"]["queried_case_count"], 21);
    assert_eq!(result["e0_recurrence_holdout"]["matching_case_count"], 21);
    assert_eq!(
        result["e0_recurrence_holdout"]["interpretation"],
        "descriptive_recurrence_only_no_causal_or_outcome_claim"
    );
    assert_eq!(
        result["events"].as_array().unwrap().last().unwrap()["stage"],
        "e0_recurrence_holdout"
    );
    assert_eq!(timeline.last().unwrap()["stage"], "e0_recurrence_holdout");
    assert!(
        timeline.last().unwrap()["detail"]
            .as_str()
            .unwrap()
            .contains("no causal or outcome claim")
    );
    assert_eq!(result["proposal"]["status"], "simulated_unverified");
    assert_eq!(result["proposal"]["execution_status"], "not_executed");
    assert_eq!(result["formal_route"], "do_nothing");
    assert_eq!(result["signals"].as_array().unwrap().len(), 3);
    assert!(result["signal"]["pattern_ref"].as_str().is_some());
    assert!(!serialized.contains("private-case-"));
    assert!(!serialized.contains("private-query-"));
    assert!(!serialized.contains("normalized-query-pattern"));

    let changed_holdout_input = temp.path().join("e0-recurrence-changed-holdout");
    let changed_holdout_output = temp.path().join("runs-recurrence-changed-holdout");
    e0_recurrence_fixture(&changed_holdout_input, "different-replay-pattern", 21, 0);
    let changed_holdout = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "e0",
            "--input",
        ])
        .arg(&changed_holdout_input)
        .args(["--output"])
        .arg(&changed_holdout_output)
        .args([
            "--tenant-id",
            "pulso_local",
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--arranque-cases",
            "21",
        ])
        .output()
        .expect("run with changed Reproduccion-only query signature");
    assert!(
        changed_holdout.status.success(),
        "{}",
        String::from_utf8_lossy(&changed_holdout.stderr)
    );
    let changed_holdout_dir = fs::read_dir(&changed_holdout_output)
        .expect("changed holdout output directory")
        .next()
        .expect("changed holdout run")
        .expect("read changed holdout run")
        .path();
    let changed_holdout_result: serde_json::Value = serde_json::from_slice(
        &fs::read(changed_holdout_dir.join("result.json")).expect("changed holdout result"),
    )
    .expect("valid changed holdout result");
    assert_eq!(
        changed_holdout_result["signal"]["numerator"],
        result["signal"]["numerator"]
    );
    assert_eq!(
        changed_holdout_result["signal"]["denominator"],
        result["signal"]["denominator"]
    );
    assert_eq!(
        changed_holdout_result["proposal"]["hypothesis"],
        result["proposal"]["hypothesis"]
    );
    assert_eq!(
        changed_holdout_result["candidates"]
            .as_array()
            .unwrap()
            .len(),
        result["candidates"].as_array().unwrap().len()
    );
    assert_eq!(
        changed_holdout_result["e0_recurrence_holdout"]["matching_case_count"],
        0
    );
    assert_eq!(
        changed_holdout_result["e0_recurrence_holdout"]["status"],
        "not_observed"
    );

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

#[test]
fn binary_suppresses_holdout_counts_for_one_through_four_matching_cases() {
    for matching_cases in 1..=4 {
        let temp = TempDir::new().expect("temp directory");
        let input = temp.path().join("e0-low-support");
        let output = temp.path().join("runs-low-support");
        e0_recurrence_fixture(&input, "different-replay-pattern", 21, matching_cases);

        let result = run_e0_cli(&input, &output);
        assert_eq!(
            result["e0_recurrence_holdout"]["status"],
            "insufficient_support"
        );
        assert!(result["excluded_replay_case_count"].is_null());
        assert!(result["e0_recurrence_holdout"]["reproduction_case_count"].is_null());
        assert!(result["e0_recurrence_holdout"]["queried_case_count"].is_null());
        assert!(result["e0_recurrence_holdout"]["matching_case_count"].is_null());
        assert!(result["e0_recurrence_holdout"]["recurrence_rate_basis_points"].is_null());
        let event = result["events"].as_array().unwrap().last().unwrap();
        assert_eq!(event["status"], "insufficient_support");
        assert!(
            event["detail"]
                .as_str()
                .unwrap()
                .contains("exact counts suppressed")
        );
    }
}

#[test]
fn binary_suppresses_replay_count_when_holdout_denominator_is_below_floor() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("e0-low-denominator");
    let output = temp.path().join("runs-low-denominator");
    e0_recurrence_fixture(&input, "different-replay-pattern", 4, 0);

    let result = run_e0_cli(&input, &output);
    assert_eq!(
        result["e0_recurrence_holdout"]["status"],
        "insufficient_support"
    );
    assert!(result["e0_recurrence_holdout"]["queried_case_count"].is_null());
    assert!(result["excluded_replay_case_count"].is_null());
}
