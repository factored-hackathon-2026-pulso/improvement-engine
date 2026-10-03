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

fn strings(values: Vec<&str>) -> ArrayRef {
    Arc::new(LargeStringArray::from(values))
}

fn e0_fixture(root: &Path) {
    let data = root.join("datos");
    fs::create_dir_all(&data).expect("create data directory");
    fs::create_dir_all(root.join("contratos")).expect("create contracts directory");
    fs::write(
        root.join("contratos/platform_history.json"),
        "{\"fixture\":\"synthetic\"}",
    )
    .expect("write synthetic source contract");
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
            strings(vec!["private-case-1", "private-case-2"]),
            Arc::new(TimestampMicrosecondArray::from(vec![at, at + 10]).with_timezone("UTC")),
            strings(vec!["chat", "chat"]),
            strings(vec!["es", "es"]),
            strings(vec!["payments", "payments"]),
            strings(vec!["normal", "normal"]),
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
            strings(vec!["private-case-1", "private-case-2"]),
            Arc::new(TimestampMicrosecondArray::from(vec![at + 1, at + 11]).with_timezone("UTC")),
            strings(vec!["private-call-1", "private-call-2"]),
            strings(vec!["tree", "tree"]),
            strings(vec!["status_lookup", "status_lookup"]),
            strings(vec!["read", "read"]),
            strings(vec!["error", "error"]),
            Arc::new(BooleanArray::from(vec![Some(false), Some(false)])),
            strings(vec!["{}", "{}"]),
            Arc::new(Int32Array::from(vec![Some(2), Some(1)])),
            Arc::new(Int64Array::from(vec![Some(5), Some(5)])),
        ],
    );
}

fn persisted_result(output: &Path) -> serde_json::Value {
    let run_dir = persisted_run_dir(output);
    serde_json::from_slice(&fs::read(run_dir.join("result.json")).expect("result file"))
        .expect("valid result JSON")
}

fn persisted_events(output: &Path) -> Vec<serde_json::Value> {
    let run_dir = persisted_run_dir(output);
    fs::read_to_string(run_dir.join("events.ndjson"))
        .expect("events file")
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid event JSON"))
        .collect()
}

fn persisted_run_dir(output: &Path) -> std::path::PathBuf {
    fs::read_dir(output)
        .expect("output directory")
        .next()
        .expect("run directory")
        .expect("read run directory")
        .path()
}

fn run_cli(source: &str, input: &Path, output: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_improvement-engine"))
        .args([
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            source,
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
            "2",
        ])
        .output()
        .expect("run public CLI")
}

#[test]
fn e0_cli_persists_every_qualifying_proposal_seed_with_truthful_provenance() {
    let temp = TempDir::new().expect("temporary directory");
    let input = temp.path().join("e0-input");
    let output = temp.path().join("e0-runs");
    e0_fixture(&input);

    let completed = run_cli("e0", &input, &output);
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let result = persisted_result(&output);
    // This fixture has no recurring-query candidate, so no route-bound
    // investigation plan may be fabricated from unrelated metric candidates.
    assert!(result.get("e0_investigation_proposal_plan").is_none());
    let assembly = result
        .get("proposal_assembly")
        .expect("persisted result should include E0 proposal assembly");
    assert_eq!(assembly["status"], "candidates_ready");
    assert_eq!(assembly["source_family"], "e0");
    assert_eq!(assembly["authority"], "simulator_only");
    assert_eq!(assembly["source_run_id"], result["run_id"]);
    assert_eq!(assembly["business_lift"], serde_json::Value::Null);

    let candidates = assembly["candidates"].as_array().expect("candidate list");
    let metrics = candidates
        .iter()
        .map(|candidate| candidate["metric_id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        metrics,
        ["e0_technical_error_rate", "e0_tool_retry_case_rate"]
            .into_iter()
            .collect()
    );
    assert_eq!(
        candidates.len(),
        metrics.len(),
        "no candidate is duplicated"
    );
    for candidate in candidates {
        assert_eq!(candidate["claim_level"], "descriptive_only");
        assert_eq!(candidate["route_status"], "unlinked");
        assert_eq!(candidate["evaluation_status"], "not_evaluated");
        assert_eq!(candidate["business_lift"], serde_json::Value::Null);
        assert_eq!(candidate["native_agent_core_status"], "not_connected");
        assert_eq!(candidate["source_run_id"], result["run_id"]);
        assert_eq!(
            candidate["source_snapshot_ref"],
            assembly["source_snapshot_ref"]
        );
        assert!(
            candidate["signal_digest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            candidate["summary_commitment"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
    }
    assert!(!assembly.to_string().contains("private-case-"));
    assert!(!assembly.to_string().contains("private-call-"));
    assert!(!assembly.to_string().contains("pulso_local"));
    assert!(!result.to_string().contains("private-case-"));
    assert!(!result.to_string().contains("private-call-"));

    let timeline = result["events"].as_array().expect("result timeline");
    let proposal_events = timeline
        .iter()
        .filter(|event| event["stage"] == "proposal_assembly")
        .collect::<Vec<_>>();
    assert_eq!(proposal_events.len(), 1, "one proposal activity event");
    let proposal_event = proposal_events[0];
    assert_eq!(proposal_event["status"], assembly["status"]);
    assert_eq!(
        proposal_event["detail"],
        "candidate_count=2; disposition_count=3"
    );
    assert_eq!(
        proposal_event["observed_cutoff_rfc3339"],
        result["observed_cutoff_rfc3339"]
    );
    let previous_sequence = timeline
        .iter()
        .filter(|event| event["stage"] != "proposal_assembly")
        .filter_map(|event| event["sequence"].as_u64())
        .max()
        .unwrap_or(0);
    assert_eq!(
        proposal_event["sequence"].as_u64(),
        previous_sequence.checked_add(1),
        "proposal activity follows the prior persisted timeline/holdout event"
    );
    assert_eq!(timeline.last(), Some(proposal_event));
    assert!(
        timeline
            .iter()
            .all(|event| event["stage"] != "e0_investigation_proposal_plan")
    );
    let event_fields = proposal_event
        .as_object()
        .expect("RunEvent object")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        event_fields,
        [
            "sequence",
            "stage",
            "status",
            "detail",
            "observed_cutoff_rfc3339"
        ]
        .into_iter()
        .collect()
    );
    let ndjson_events = persisted_events(&output);
    assert_eq!(
        ndjson_events.as_slice(),
        timeline.as_slice(),
        "persisted event views are identical"
    );
    let ndjson_proposal_events = ndjson_events
        .iter()
        .filter(|event| event["stage"] == "proposal_assembly")
        .collect::<Vec<_>>();
    assert_eq!(ndjson_proposal_events.len(), 1);
    assert_eq!(ndjson_proposal_events[0], proposal_event);
    assert_eq!(ndjson_events.last(), Some(proposal_event));
    assert!(!proposal_event.to_string().contains("private-case-"));
    assert!(!proposal_event.to_string().contains("private-call-"));
    assert!(!proposal_event.to_string().contains("pulso_local"));
    assert!(!proposal_event.to_string().contains("sha256"));
}

#[test]
fn original_bank_cli_never_persists_an_e0_proposal_assembly() {
    let temp = TempDir::new().expect("temporary directory");
    let input = temp.path().join("original-input");
    let contacts = input.join("call_center_interactions");
    let output = temp.path().join("original-runs");
    fs::create_dir_all(&contacts).expect("create contacts table directory");
    let mut csv =
        String::from("interaction_id,customer_id,interaction_date,contact_reason,channel\n");
    for (month, day) in [("2027-03", 1), ("2027-04", 1)] {
        for offset in 0..5 {
            csv.push_str(&format!(
                "private-interaction-{month}-{offset},private-customer-{month}-{offset},{month}-{day:02} 10:00:00,Complaint,Phone\n"
            ));
        }
    }
    fs::write(contacts.join("part-000.csv"), csv).expect("write synthetic contacts");

    let completed = run_cli("original", &input, &output);
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let result = persisted_result(&output);
    assert_eq!(result["source_kind"], "original_bank");
    assert!(result.get("proposal_assembly").is_none());
    assert!(result.get("e0_mechanism_resolution").is_none());
    assert!(result.get("e0_investigation_proposal_plan").is_none());
    assert!(result.get("local_simulation_portfolio").is_none());
    assert!(!result.to_string().contains("candidates_ready"));
    assert!(!result.to_string().contains("private-interaction-"));
    assert!(!result.to_string().contains("private-customer-"));
    assert!(
        !result["events"]
            .as_array()
            .expect("OriginalBank timeline")
            .iter()
            .any(|event| event["stage"] == "proposal_assembly")
    );
    assert!(
        !result["events"]
            .as_array()
            .expect("OriginalBank timeline")
            .iter()
            .any(|event| event["stage"] == "e0_mechanism_resolution")
    );
    assert!(
        !result["events"]
            .as_array()
            .expect("OriginalBank timeline")
            .iter()
            .any(|event| event["stage"] == "e0_investigation_proposal_plan")
    );
    assert!(
        !persisted_events(&output)
            .iter()
            .any(|event| event["stage"] == "proposal_assembly")
    );
    assert!(
        !persisted_events(&output)
            .iter()
            .any(|event| event["stage"] == "e0_mechanism_resolution")
    );
}
