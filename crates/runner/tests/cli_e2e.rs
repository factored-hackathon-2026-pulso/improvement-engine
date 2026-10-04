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
fn original_cli_reports_sanitized_source_preparation_progress_without_changing_manifest() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("original-progress-input");
    let interactions = input.join("call_center_interactions");
    fs::create_dir_all(&interactions).expect("create original input");
    let interaction_csv = interactions.join("part-000.csv");
    fs::write(
        &interaction_csv,
        concat!(
            "interaction_id,customer_id,interaction_date,contact_reason,channel\n",
            "private-interaction-sentinel,private-customer-sentinel,2025-01-01T10:00:00,Complaint,Phone\n",
        ),
    )
    .expect("write synthetic contact source");
    let auxiliary = input.join("transactions");
    fs::create_dir_all(&auxiliary).expect("create auxiliary table");
    fs::write(
        auxiliary.join("part-001.csv"),
        "id,value\nprivate-row-sentinel,1\n",
    )
    .expect("write synthetic auxiliary source");

    let run = |name: &str, progress: bool| {
        let output = temp.path().join(name);
        let mut command = Command::new(env!("CARGO_BIN_EXE_improvement-engine"));
        command.args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "original",
            "--input",
        ]);
        command.arg(&input).args(["--output"]).arg(&output).args([
            "--tenant-id",
            "pulso_local",
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
        ]);
        if progress {
            command.arg("--progress-jsonl");
        }
        let completed = command.output().expect("run original source through CLI");
        assert!(
            completed.status.success(),
            "{}",
            String::from_utf8_lossy(&completed.stderr)
        );
        let run_dir = fs::read_dir(&output)
            .expect("read output root")
            .next()
            .expect("one output run")
            .expect("output run entry")
            .path();
        let result: serde_json::Value = serde_json::from_slice(
            &fs::read(run_dir.join("result.json")).expect("read result envelope"),
        )
        .expect("parse result envelope");
        (completed, result)
    };

    let (completed, progressed_result) = run("progress-output", true);
    let (_, baseline_result) = run("baseline-output", false);
    let stderr = String::from_utf8(completed.stderr).expect("UTF-8 progress output");
    let progress = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("JSONL progress"))
        .collect::<Vec<_>>();

    for stage in ["inventory", "manifest_scan", "contact_projection"] {
        assert!(
            progress
                .iter()
                .any(|event| event["stage"] == stage && event["status"] == "completed"),
            "missing completed progress for {stage}: {stderr}"
        );
    }
    let completed_stages = progress
        .iter()
        .filter(|event| event.get("stage").is_some() && event["status"] == "completed")
        .map(|event| event["stage"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        completed_stages,
        ["inventory", "manifest_scan", "contact_projection"]
    );
    for event in progress.iter().filter(|event| event.get("stage").is_some()) {
        assert_eq!(event["phase"], "source_preparation");
        assert!(event["files_completed"].as_u64().is_some());
        assert!(event["files_total"].as_u64().is_some());
        assert!(event["bytes_completed"].as_u64().is_some());
        assert!(event["bytes_total"].as_u64().is_some());
        assert!(event["elapsed_ms"].as_u64().is_some());
        assert!(
            event["files_completed"].as_u64().unwrap() <= event["files_total"].as_u64().unwrap()
        );
        assert!(
            event["bytes_completed"].as_u64().unwrap() <= event["bytes_total"].as_u64().unwrap()
        );
    }
    for (stage, expected_files) in [
        ("inventory", 2),
        ("manifest_scan", 2),
        ("contact_projection", 1),
    ] {
        let completed = progress
            .iter()
            .find(|event| event["stage"] == stage && event["status"] == "completed")
            .expect("completed stage event");
        assert_eq!(completed["files_completed"], expected_files);
        assert_eq!(completed["files_total"], expected_files);
        assert!(completed["bytes_total"].as_u64().unwrap() > 0);
        assert_eq!(completed["bytes_completed"], completed["bytes_total"]);
    }
    for forbidden in [
        input.to_string_lossy().as_ref(),
        "private-interaction-sentinel",
        "private-customer-sentinel",
        "private-row-sentinel",
        "part-000.csv",
        "sha256:",
    ] {
        assert!(!stderr.contains(forbidden), "progress exposed {forbidden}");
    }
    assert_eq!(
        progressed_result["manifest_digest"],
        baseline_result["manifest_digest"]
    );
    assert_eq!(
        progressed_result["snapshot_ref"]["digest"],
        baseline_result["snapshot_ref"]["digest"]
    );
}

#[test]
fn original_cli_bounds_manifest_progress_for_many_files_and_preserves_exact_totals() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("many-original-csvs");
    let transactions = input.join("transactions");
    let contacts = input.join("call_center_interactions");
    fs::create_dir_all(&transactions).expect("create transactions");
    fs::create_dir_all(&contacts).expect("create contacts");
    let transaction_csv = "id,value\nrow,1\n";
    for index in 0..205 {
        fs::write(
            transactions.join(format!("part-{index:03}.csv")),
            transaction_csv,
        )
        .expect("write synthetic transaction file");
    }
    let contact_csv = "interaction_id,customer_id,interaction_date,contact_reason,channel\nprivate-id,private-customer,2025-01-01,complaint,phone\n";
    for index in 0..205 {
        fs::write(contacts.join(format!("part-{index:03}.csv")), contact_csv)
            .expect("write synthetic contact file");
    }

    let output = temp.path().join("many-original-output");
    let completed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "original",
            "--input",
        ])
        .arg(&input)
        .args(["--output"])
        .arg(&output)
        .args([
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--progress-jsonl",
        ])
        .output()
        .expect("run original source through CLI");
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
    let manifest_events = progress
        .iter()
        .filter(|event| event["stage"] == "manifest_scan")
        .collect::<Vec<_>>();
    assert!(
        manifest_events.len() <= 102,
        "too many manifest progress events: {}",
        manifest_events.len()
    );
    let completed_manifest = manifest_events
        .iter()
        .find(|event| event["status"] == "completed")
        .expect("completed manifest scan event");
    assert_eq!(completed_manifest["files_completed"], 410);
    assert_eq!(completed_manifest["files_total"], 410);
    let expected_contact_bytes = contact_csv.len() as u64 * 205;
    let expected_bytes = transaction_csv.len() as u64 * 205 + expected_contact_bytes;
    assert_eq!(completed_manifest["bytes_completed"], expected_bytes);
    assert_eq!(completed_manifest["bytes_total"], expected_bytes);
    let contact_events = progress
        .iter()
        .filter(|event| event["stage"] == "contact_projection")
        .collect::<Vec<_>>();
    assert!(
        contact_events.len() <= 102,
        "too many contact progress events: {}",
        contact_events.len()
    );
    let completed_contact = contact_events
        .iter()
        .find(|event| event["status"] == "completed")
        .expect("completed contact projection event");
    assert_eq!(completed_contact["files_completed"], 205);
    assert_eq!(completed_contact["files_total"], 205);
    assert_eq!(completed_contact["bytes_completed"], expected_contact_bytes);
    assert_eq!(completed_contact["bytes_total"], expected_contact_bytes);
    assert!(!stderr.contains("private-customer"));
}

#[test]
fn original_cli_reports_one_failed_event_for_each_source_preparation_stage_error() {
    let temp = TempDir::new().expect("temp directory");
    let manifest_input = temp.path().join("invalid-manifest");
    let transaction_dir = manifest_input.join("transactions");
    fs::create_dir_all(&transaction_dir).expect("create transaction directory");
    fs::write(transaction_dir.join("bad.csv"), "id,value\nrow\n")
        .expect("write malformed transaction CSV");
    let contact_input = temp.path().join("invalid-contact-projection");
    let contact_dir = contact_input.join("call_center_interactions");
    fs::create_dir_all(&contact_dir).expect("create contact directory");
    fs::write(
        contact_dir.join("bad.csv"),
        "interaction_id,channel,channel\nprivate-id,phone,app\n",
    )
    .expect("write duplicate-header contact CSV");

    for (name, input, failed_stage) in [
        ("manifest", manifest_input, "manifest_scan"),
        ("contact", contact_input, "contact_projection"),
    ] {
        let output = temp.path().join(format!("failed-{name}-output"));
        let completed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
            .args([
                "local-sim",
                "--mode",
                "local-simulation",
                "--source",
                "original",
                "--input",
            ])
            .arg(&input)
            .args(["--output"])
            .arg(&output)
            .args([
                "--observed-cutoff",
                "2025-07-01T00:00:00Z",
                "--progress-jsonl",
            ])
            .output()
            .expect("run invalid original source through CLI");
        assert!(!completed.status.success());
        let stderr = String::from_utf8(completed.stderr).expect("UTF-8 progress output");
        let progress = stderr
            .lines()
            .filter(|line| line.starts_with('{'))
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("JSONL progress"))
            .filter(|event| event["stage"] == failed_stage)
            .collect::<Vec<_>>();
        assert_eq!(
            progress
                .iter()
                .filter(|event| event["status"] == "failed")
                .count(),
            1,
            "expected exactly one failed event for {failed_stage}: {stderr}"
        );
        assert!(!progress.iter().any(|event| event["status"] == "completed"));
        assert!(!stderr.contains("private-id"));
    }
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
fn original_bank_cli_emits_snapshot_descriptive_opportunity_without_publishing_candidate() {
    let temp = TempDir::new().expect("temp directory");
    let input = temp.path().join("original-bank");
    let contacts = input.join("call_center_interactions");
    let output = temp.path().join("runs-original");
    fs::create_dir_all(&contacts).expect("contact table directory");
    let mut csv =
        String::from("interaction_id,customer_id,interaction_date,contact_reason,channel\n");
    for (month, day) in [("2027-03", 1), ("2027-04", 1)] {
        for offset in 0..5 {
            csv.push_str(&format!(
                "private-interaction-{month}-{offset},private-customer-{month}-{offset},{month}-{day:02} 10:00:00,Complaint,Phone\n"
            ));
        }
    }
    for offset in 0..4 {
        csv.push_str(&format!(
            "private-suppressed-{offset},private-suppressed-customer-{offset},2027-05-01 10:00:00,Technical,Chat\n"
        ));
    }
    csv.push_str(
        "private-rejected-channel,private-customer-rejected-channel,2027-06-01 10:00:00,Complaint,\n",
    );
    csv.push_str(
        "private-rejected-timestamp,private-customer-rejected-timestamp,not-a-date,Complaint,Phone\n",
    );
    fs::write(contacts.join("part-000.csv"), csv).expect("write synthetic contacts");

    let completed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "original",
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
        .expect("run original source CLI");
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
    let envelope = &result["snapshot_descriptive_envelope"];
    assert!(!serialized.contains("\"rejected_rows\""));
    assert!(!serialized.contains("\"suppressed_cells\""));
    assert_eq!(
        result["terminal_status"],
        "snapshot_descriptive_finding_ready"
    );
    assert_eq!(result["formal_route"], "do_nothing");
    assert_eq!(
        envelope["finding"]["literal_months"],
        serde_json::json!(["2027-03", "2027-04"])
    );
    assert_eq!(
        envelope["finding"]["temporal_basis"],
        "literal_source_wall_clock_month"
    );
    assert_eq!(
        envelope["finding"]["value_semantics"],
        "final_extract_facts_only"
    );
    assert_eq!(envelope["finding"]["coverage"], "partial");
    assert_eq!(envelope["finding"]["minimum_cell_count"], 5);
    assert!(envelope["finding"].get("rejected_rows").is_none());
    assert!(envelope["finding"].get("suppressed_cells").is_none());
    assert_eq!(envelope["finding"]["complaint_contact_count"], 10);
    assert_eq!(
        envelope["agent_core_candidate"],
        "dependency_blocked_snapshot_semantics"
    );
    assert_eq!(envelope["proposal"]["status"], "simulated_unverified");
    assert_eq!(envelope["proposal"]["execution_status"], "not_executed");
    assert_eq!(envelope["proposal"]["publication_eligible"], false);
    for forbidden in [
        "private-interaction",
        "private-customer",
        "private-suppressed",
        "technical",
        "chat",
        "rejected_rows",
        "suppressed_cells",
        "observed_cutoff",
    ] {
        assert!(!envelope.to_string().contains(forbidden));
        if forbidden != "observed_cutoff" {
            assert!(
                !serialized.contains(forbidden),
                "unexpected disclosure in result.json: {forbidden}"
            );
        }
    }
    assert!(!envelope.to_string().contains("2025-07-01T00:00:00Z"));
}

#[test]
fn original_cli_does_not_allow_per_run_contact_k_override() {
    let temp = TempDir::new().expect("temp directory");
    let completed = Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "original",
            "--input",
        ])
        .arg(temp.path().join("source-does-not-need-to-exist"))
        .args(["--output"])
        .arg(temp.path().join("out"))
        .args([
            "--tenant-id",
            "pulso_local",
            "--observed-cutoff",
            "2025-07-01T00:00:00Z",
            "--arranque-cases",
            "1",
            "--min-contact-cell-count",
            "10",
        ])
        .output()
        .expect("run CLI with unsupported k override");
    assert!(!completed.status.success());
    assert!(
        String::from_utf8_lossy(&completed.stderr)
            .contains("unknown option --min-contact-cell-count")
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
    assert_eq!(
        result["e0_builder_input_preparation"]["status"],
        "not_applicable"
    );
    assert_eq!(
        result["e0_builder_input_preparation"]["reason"],
        "no_qualifying_candidate"
    );
    assert_eq!(result["e0_builder_input_preparation"]["candidate_count"], 0);
    assert!(result["e0_recurrence_holdout"].is_null());
    assert!(result.get("e0_mechanism_resolution").is_none());
    assert!(result.get("e0_investigation_proposal_plan").is_none());
    assert!(
        result["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["stage"] != "e0_mechanism_resolution")
    );
    assert!(
        result["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["stage"] != "e0_investigation_proposal_plan")
    );
    assert!(
        result["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["stage"] == "e0_builder_input_preparation"
                && event["status"] == "not_applicable")
    );
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
    let ndjson_timeline: Vec<serde_json::Value> = fs::read_to_string(run_dir.join("events.ndjson"))
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
    let mechanism = result
        .get("e0_mechanism_resolution")
        .expect("E0 recurrence result includes explicit mechanism resolution");
    assert_eq!(mechanism["evidence_origin"], "e0_local_run");
    assert_eq!(
        mechanism["catalog_origin"],
        "team_generated_empty_local_catalog_fixture"
    );
    assert_eq!(mechanism["catalog_durability"], "ephemeral");
    assert_eq!(
        mechanism["resolution"]["status"], "unlinked",
        "the runner's empty fixture catalog cannot invent a Core route"
    );
    assert_eq!(
        mechanism["resolution"]["reason"],
        "no_exact_supported_flow_mapping"
    );
    assert_eq!(
        mechanism["resolution"]["catalog_ref"]["id"],
        "0199b21c-7eab-7000-8000-000000000205"
    );
    assert_eq!(mechanism["resolution"]["catalog_ref"]["revision"], 1);
    assert_eq!(
        mechanism["resolution"]["catalog_ref"]["digest"],
        "sha256:ebf1ae456ab9bfa6e06bea342f2c1d377783ce04a33a19e4c66cf6515a21320a"
    );
    assert_eq!(
        mechanism["evidence_packet"]["metric_id"],
        "e0_recurring_copilot_query_cases"
    );
    let recurrence_candidate = result["proposal_assembly"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["metric_id"] == "e0_recurring_copilot_query_cases")
        .expect("recurrence candidate from the same assembly");
    assert_eq!(
        mechanism["evidence_packet"]["signal_digest"],
        recurrence_candidate["signal_digest"]
    );
    assert_eq!(
        mechanism["evidence_packet"]["summary_commitment"],
        recurrence_candidate["summary_commitment"]
    );
    assert_eq!(
        mechanism["evidence_packet"]["source_run_id"],
        result["run_id"]
    );
    assert_eq!(
        mechanism["evidence_packet"]["observed_cutoff_rfc3339"],
        result["observed_cutoff_rfc3339"]
    );
    assert_eq!(mechanism["evidence_packet"]["numerator"], 20);
    assert_eq!(mechanism["evidence_packet"]["denominator"], 21);
    assert_eq!(
        mechanism["evidence_packet"]["claim_level"],
        "descriptive_only"
    );
    let investigation_plan = result
        .get("e0_investigation_proposal_plan")
        .expect("E0 candidate has a typed read-only investigation plan");
    assert_eq!(
        investigation_plan["artifact_kind"],
        "e0_read_only_investigation_plan_not_agent_core_proposal"
    );
    assert!(
        investigation_plan["plan_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(
        investigation_plan["proposal_ref"],
        recurrence_candidate["proposal_ref"]
    );
    assert_eq!(
        investigation_plan["evidence_packet"],
        mechanism["evidence_packet"]
    );
    assert_eq!(
        investigation_plan["decision"]["recommended_option"],
        "investigate_mapping"
    );
    assert_eq!(
        investigation_plan["decision"]["available_options"],
        serde_json::json!(["investigate_mapping", "do_nothing"])
    );
    assert_eq!(investigation_plan["decision"]["authority"], "none");
    assert_eq!(
        investigation_plan["decision"]["execution_state"],
        "not_executable"
    );
    assert_eq!(investigation_plan["business_lift"], serde_json::Value::Null);
    assert!(!investigation_plan.to_string().contains("pulso_local"));
    assert!(!investigation_plan.to_string().contains("private-query-"));
    assert!(mechanism["evidence_packet"].get("tenant_scope").is_none());
    assert!(!mechanism.to_string().contains("pulso_local"));
    assert!(!mechanism.to_string().contains("normalized-query-pattern"));
    assert!(!mechanism.to_string().contains("private-query-"));
    assert!(!mechanism.to_string().contains("private-case-"));
    assert!(mechanism["resolution"].get("flow_ref").is_none());
    assert!(mechanism.get("business_lift").is_none());
    assert!(mechanism.get("execution_status").is_none());
    assert!(mechanism["resolution"].get("may_compile").is_none());
    assert!(
        mechanism["resolution"]
            .get("may_start_sandbox_trial")
            .is_none()
    );
    let holdout_event = result["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["stage"] == "e0_recurrence_holdout")
        .expect("persisted holdout event");
    assert_eq!(
        ndjson_timeline
            .iter()
            .find(|event| event["stage"] == "e0_recurrence_holdout"),
        Some(holdout_event)
    );
    let result_timeline = result["events"].as_array().unwrap();
    let proposal_event = result_timeline
        .iter()
        .find(|event| event["stage"] == "proposal_assembly")
        .expect("persisted proposal activity event");
    assert_eq!(
        proposal_event["sequence"].as_u64(),
        holdout_event["sequence"].as_u64().map(|value| value + 1)
    );
    let builder_input = result
        .get("e0_builder_input_preparation")
        .expect("persisted E0 builder readiness boundary");
    assert_eq!(builder_input["status"], "dependency_blocked");
    assert_eq!(builder_input["evidence_binding"], "bound");
    assert_eq!(
        builder_input["candidate_count"],
        result["proposal_assembly"]["candidates"]
            .as_array()
            .unwrap()
            .len()
    );
    assert_eq!(
        builder_input["readiness"]["u20_plan"],
        "unavailable_in_local_simulation"
    );
    let builder_event = result_timeline
        .iter()
        .find(|event| event["stage"] == "e0_builder_input_preparation")
        .expect("persisted E0 builder-input event");
    assert_eq!(builder_event["status"], "dependency_blocked");
    assert_eq!(
        builder_event["sequence"].as_u64(),
        proposal_event["sequence"].as_u64().map(|value| value + 1)
    );
    let mechanism_events = result_timeline
        .iter()
        .filter(|event| event["stage"] == "e0_mechanism_resolution")
        .collect::<Vec<_>>();
    assert_eq!(mechanism_events.len(), 1);
    let mechanism_event = mechanism_events[0];
    assert_eq!(mechanism_event["status"], "unlinked");
    assert_eq!(
        mechanism_event["detail"],
        "catalog_origin=team_generated_empty_local_catalog_fixture; catalog_durability=ephemeral; resolution_count=1; mapped_count=0; unlinked_count=1; reason=no_exact_supported_flow_mapping"
    );
    assert_eq!(
        mechanism_event["sequence"].as_u64(),
        builder_event["sequence"].as_u64().map(|value| value + 1)
    );
    let investigation_plan_event = result_timeline
        .last()
        .filter(|event| event["stage"] == "e0_investigation_proposal_plan")
        .expect("investigation plan event follows mechanism resolution");
    assert_eq!(investigation_plan_event["status"], "pending_review");
    assert_eq!(
        investigation_plan_event["sequence"].as_u64(),
        mechanism_event["sequence"].as_u64().map(|value| value + 1)
    );
    assert!(!investigation_plan_event.to_string().contains("pulso_local"));
    assert!(
        !investigation_plan_event
            .to_string()
            .contains("private-query-")
    );
    assert!(!investigation_plan_event.to_string().contains("sha256"));
    assert_eq!(ndjson_timeline.last().unwrap(), investigation_plan_event);
    assert!(!mechanism_event.to_string().contains("pulso_local"));
    assert!(!mechanism_event.to_string().contains("private-case-"));
    assert!(!mechanism_event.to_string().contains("private-query-"));
    assert!(!mechanism_event.to_string().contains("sha256"));
    assert_eq!(
        result_timeline,
        ndjson_timeline.as_slice(),
        "JSON and NDJSON timelines remain identical"
    );
    assert!(
        holdout_event["detail"]
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
        let event = result["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["stage"] == "e0_recurrence_holdout")
            .expect("holdout event");
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
