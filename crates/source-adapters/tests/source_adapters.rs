use std::fs;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray, RecordBatch, StringArray, TimestampMicrosecondArray};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
#[cfg(feature = "local-simulation")]
use improvement_engine_source_adapters::prepare_e0_package_for_local_simulation;
use improvement_engine_source_adapters::{
    AdapterError, CasePhase, PreparationConfig, PreparedSource, SourceKind, evaluator,
    prepare_e0_package, prepare_original_bank, validate_original_contact_complaint_profile,
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

fn original_profile_fixture(root: &Path) {
    for (table, contract) in [
        (
            "call_center_interactions",
            include_str!("../../../contracts/sources/call_center_interactions.v1.json"),
        ),
        (
            "complaints",
            include_str!("../../../contracts/sources/complaints.v1.json"),
        ),
    ] {
        let columns: serde_json::Value = serde_json::from_str(contract).unwrap();
        let header = columns["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|column| column["name"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join(",");
        let table_dir = root.join(table).join("year=2025/month=01");
        fs::create_dir_all(&table_dir).unwrap();
        fs::write(table_dir.join("part.csv"), format!("{header}\n")).unwrap();
    }
}

#[test]
fn original_contact_complaint_profile_accepts_both_contract_headers_and_reports_bounded_scope() {
    let temp = TempDir::new().unwrap();
    original_profile_fixture(temp.path());

    let validated = validate_original_contact_complaint_profile(temp.path(), "1.0")
        .expect("both required profile tables validate");

    assert_eq!(validated.table_count(), 2);
    assert_eq!(validated.file_count(), 2);
}

#[test]
fn original_contact_complaint_profile_rejects_missing_pqr_table_and_version_mismatch() {
    let temp = TempDir::new().unwrap();
    let contacts = temp.path().join("call_center_interactions");
    fs::create_dir_all(&contacts).unwrap();
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/sources/call_center_interactions.v1.json"
    ))
    .unwrap();
    let header = contract["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| column["name"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join(",");
    fs::write(contacts.join("part.csv"), format!("{header}\n")).unwrap();

    assert!(validate_original_contact_complaint_profile(temp.path(), "1.0").is_err());
    original_profile_fixture(temp.path());
    assert!(validate_original_contact_complaint_profile(temp.path(), "2.0").is_err());
}

#[test]
fn original_contact_complaint_profile_rejects_wrong_header_and_malformed_rows() {
    let temp = TempDir::new().unwrap();
    original_profile_fixture(temp.path());
    let contacts = temp
        .path()
        .join("call_center_interactions/year=2025/month=01/part.csv");
    let original = fs::read_to_string(&contacts).unwrap();
    fs::write(
        &contacts,
        original.replace("interaction_id,", " interaction_id,"),
    )
    .unwrap();
    assert!(validate_original_contact_complaint_profile(temp.path(), "1.0").is_err());

    original_profile_fixture(temp.path());
    let original = fs::read_to_string(&contacts).unwrap();
    fs::write(&contacts, format!("{original}short,row\n")).unwrap();
    assert!(validate_original_contact_complaint_profile(temp.path(), "1.0").is_err());
}

#[test]
fn cutoff_rejects_fractional_seconds_instead_of_silently_truncating_them() {
    let result = PreparationConfig::new("demo-tenant", "2025-07-01T00:00:00.500Z", 10);

    assert!(result.is_err());
    assert_eq!(
        config_with_cutoff(10, "2025-07-01T00:00:00Z").observed_cutoff(),
        "2025-07-01T00:00:00Z"
    );
}

#[test]
fn original_contacts_expose_only_k_qualified_snapshot_aggregates() {
    let temp = TempDir::new().unwrap();
    let table = temp.path().join("call_center_interactions");
    fs::create_dir_all(&table).unwrap();
    fs::write(
        table.join("part-000.csv"),
        concat!(
            "interaction_id,customer_id,interaction_date,contact_reason,channel,was_resolved,requires_followup,agent_id,duration_seconds,wait_time_seconds\n",
            "INTERACTION-PII-SENTINEL-ROW-01-DO-NOT-SERIALIZE-6f11c9e8,CUSTOMER-PII-SENTINEL-ROW-01-DO-NOT-SERIALIZE-19bc3a7d,2025-01-01T10:00:00,Queja,Phone,true,false,AGENT-PII-SENTINEL-ROW-01-DO-NOT-SERIALIZE-7ad3f065,30,5\n",
            "INTERACTION-PII-SENTINEL-ROW-02-DO-NOT-SERIALIZE-41c87da5,CUSTOMER-PII-SENTINEL-ROW-02-DO-NOT-SERIALIZE-0b28f4a6,2025-01-02T10:00:00,Queja,Phone,true,false,AGENT-PII-SENTINEL-ROW-02-DO-NOT-SERIALIZE-38b0ac12,40,6\n",
            "INTERACTION-PII-SENTINEL-ROW-03-DO-NOT-SERIALIZE-8e05bca2,CUSTOMER-PII-SENTINEL-ROW-03-DO-NOT-SERIALIZE-746f190d,2025-01-03T10:00:00,Queja,Phone,false,true,AGENT-PII-SENTINEL-ROW-03-DO-NOT-SERIALIZE-b3a6015f,50,7\n",
            "INTERACTION-PII-SENTINEL-ROW-04-DO-NOT-SERIALIZE-a5096e31,CUSTOMER-PII-SENTINEL-ROW-04-DO-NOT-SERIALIZE-e7132f44,2025-01-04T10:00:00,Queja,Phone,true,false,AGENT-PII-SENTINEL-ROW-04-DO-NOT-SERIALIZE-5c8bd201,60,8\n",
            "INTERACTION-PII-SENTINEL-ROW-05-DO-NOT-SERIALIZE-c218f6a9,CUSTOMER-PII-SENTINEL-ROW-05-DO-NOT-SERIALIZE-9d07c351,2025-01-05T10:00:00,Queja,Phone,true,false,AGENT-PII-SENTINEL-ROW-05-DO-NOT-SERIALIZE-2fa19c73,70,9\n",
            "INTERACTION-PII-SENTINEL-ROW-06-DO-NOT-SERIALIZE-3d5f209a,CUSTOMER-PII-SENTINEL-ROW-06-DO-NOT-SERIALIZE-ec68240b,2099-01-06T10:00:00,person@example.test,Phone,true,false,AGENT-PII-SENTINEL-ROW-06-DO-NOT-SERIALIZE-98a61e0d,80,10\n",
            "INTERACTION-PII-SENTINEL-ROW-07-DO-NOT-SERIALIZE-d0512a86,CUSTOMER-PII-SENTINEL-ROW-07-DO-NOT-SERIALIZE-0f23bd74,2025-01-07T10:00:00,Queja,,true,false,AGENT-PII-SENTINEL-ROW-07-DO-NOT-SERIALIZE-a67c3d18,80,10\n",
        ),
    )
    .unwrap();

    let prepared = prepare_original_bank(temp.path(), &config(10)).unwrap();
    let volumes = prepared.agent_inputs().contact_volumes();

    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0].reason(), "complaint");
    assert_eq!(volumes[0].channel(), "phone");
    assert_eq!(volumes[0].record_count(), 5);
    let summary = prepared.agent_inputs().contact_projection().unwrap();
    assert_eq!(
        summary.semantics(),
        improvement_engine_source_adapters::ContactProjectionSemantics::SnapshotExtractCounts
    );
    assert_eq!(summary.included_record_count(), 5);
    let serialized = serde_json::to_string(&prepared).unwrap();
    for forbidden in [
        "INTERACTION-PII-SENTINEL-ROW-01-DO-NOT-SERIALIZE-6f11c9e8",
        "CUSTOMER-PII-SENTINEL-ROW-01-DO-NOT-SERIALIZE-19bc3a7d",
        "AGENT-PII-SENTINEL-ROW-01-DO-NOT-SERIALIZE-7ad3f065",
        "INTERACTION-PII-SENTINEL-ROW-02-DO-NOT-SERIALIZE-41c87da5",
        "CUSTOMER-PII-SENTINEL-ROW-02-DO-NOT-SERIALIZE-0b28f4a6",
        "AGENT-PII-SENTINEL-ROW-02-DO-NOT-SERIALIZE-38b0ac12",
        "INTERACTION-PII-SENTINEL-ROW-03-DO-NOT-SERIALIZE-8e05bca2",
        "CUSTOMER-PII-SENTINEL-ROW-03-DO-NOT-SERIALIZE-746f190d",
        "AGENT-PII-SENTINEL-ROW-03-DO-NOT-SERIALIZE-b3a6015f",
        "INTERACTION-PII-SENTINEL-ROW-04-DO-NOT-SERIALIZE-a5096e31",
        "CUSTOMER-PII-SENTINEL-ROW-04-DO-NOT-SERIALIZE-e7132f44",
        "AGENT-PII-SENTINEL-ROW-04-DO-NOT-SERIALIZE-5c8bd201",
        "INTERACTION-PII-SENTINEL-ROW-05-DO-NOT-SERIALIZE-c218f6a9",
        "CUSTOMER-PII-SENTINEL-ROW-05-DO-NOT-SERIALIZE-9d07c351",
        "AGENT-PII-SENTINEL-ROW-05-DO-NOT-SERIALIZE-2fa19c73",
        "INTERACTION-PII-SENTINEL-ROW-06-DO-NOT-SERIALIZE-3d5f209a",
        "CUSTOMER-PII-SENTINEL-ROW-06-DO-NOT-SERIALIZE-ec68240b",
        "AGENT-PII-SENTINEL-ROW-06-DO-NOT-SERIALIZE-98a61e0d",
        "INTERACTION-PII-SENTINEL-ROW-07-DO-NOT-SERIALIZE-d0512a86",
        "CUSTOMER-PII-SENTINEL-ROW-07-DO-NOT-SERIALIZE-0f23bd74",
        "AGENT-PII-SENTINEL-ROW-07-DO-NOT-SERIALIZE-a67c3d18",
        "person@example.test",
        "2025-01-01",
        "2099-01-06T10:00:00",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "serialized source sentinel"
        );
    }
    for forbidden in ["person@example.test", "Queja", "Phone"] {
        assert!(!serialized.contains(forbidden));
    }
    for day in 1..=7 {
        let year = if day == 6 { 2099 } else { 2025 };
        assert!(!serialized.contains(&format!("{year}-01-{day:02}T10:00:00")));
    }
    let serialized_json: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    fn contains_key(value: &serde_json::Value, expected: &str) -> bool {
        match value {
            serde_json::Value::Object(object) => {
                object.contains_key(expected)
                    || object.values().any(|child| contains_key(child, expected))
            }
            serde_json::Value::Array(items) => {
                items.iter().any(|child| contains_key(child, expected))
            }
            _ => false,
        }
    }
    for forbidden_key in [
        "interaction_id",
        "customer_id",
        "agent_id",
        "was_resolved",
        "requires_followup",
        "duration_seconds",
        "wait_time_seconds",
    ] {
        assert!(!contains_key(&serialized_json, forbidden_key));
    }
    assert!(!serialized.contains("true"));
    assert!(!serialized.contains("rejected_rows"));
    assert!(!serialized.contains("suppressed_cells"));
}

#[test]
fn original_contacts_expose_literal_month_snapshot_projection_without_cutoff_or_raw_values() {
    let temp = TempDir::new().unwrap();
    let table = temp.path().join("call_center_interactions");
    fs::create_dir_all(&table).unwrap();
    fs::write(
        table.join("part-000.csv"),
        concat!(
            "interaction_id,customer_id,interaction_date,contact_reason,channel\n",
            "id-1,c-1,2027-03-31 23:59:59,Queja,Phone\n",
            "id-2,c-2,2027-03-31 23:59:59,Queja,Phone\n",
            "id-3,c-3,2027-03-31 23:59:59,Queja,Phone\n",
            "id-4,c-4,2027-03-31 23:59:59,Queja,Phone\n",
            "id-5,c-5,2027-03-31 23:59:59,Queja,Phone\n",
            "id-6,c-6,2027-04-01 00:00:00,Queja,Phone\n",
            "id-7,c-7,2027-04-01 00:00:00,Queja,Phone\n",
            "id-8,c-8,2027-04-01 00:00:00,Queja,Phone\n",
            "id-9,c-9,2027-04-01 00:00:00,Queja,Phone\n",
            "id-10,c-10,2027-04-01 00:00:00,Queja,Phone\n",
        ),
    )
    .unwrap();

    let prepared = prepare_original_bank(temp.path(), &config(10)).unwrap();
    let projection = prepared
        .agent_inputs()
        .descriptive_contact_projection()
        .expect("snapshot descriptive projection");

    assert_eq!(
        projection.temporal_basis(),
        "literal_source_wall_clock_month"
    );
    assert_eq!(projection.value_semantics(), "final_extract_facts_only");
    assert_eq!(projection.coverage(), "partial");
    assert_eq!(projection.minimum_cell_count(), 5);
    assert_eq!(projection.aggregates().len(), 2);
    assert_eq!(projection.aggregates()[0].period(), "2027-03");
    assert_eq!(projection.aggregates()[0].contact_count(), 5);
    assert_eq!(projection.aggregates()[1].period(), "2027-04");
    assert_eq!(projection.aggregates()[1].contact_count(), 5);

    let serialized = serde_json::to_string(projection).unwrap();
    for forbidden in ["id-1", "c-1", "interaction_date", "observed_cutoff"] {
        assert!(!serialized.contains(forbidden));
    }
}

#[test]
fn descriptive_projection_omits_exact_rejection_and_suppression_counts() {
    let temp = TempDir::new().unwrap();
    let table = temp.path().join("call_center_interactions");
    fs::create_dir_all(&table).unwrap();
    fs::write(
        table.join("part-000.csv"),
        concat!(
            "interaction_id,interaction_date,contact_reason,channel\n",
            "id-1,2027-03-01 10:00:00,Queja,Phone\n",
            "id-2,2027-03-02 10:00:00,Queja,Phone\n",
            "id-3,2027-03-03 10:00:00,Queja,Phone\n",
            "id-4,2027-03-04 10:00:00,Queja,Phone\n",
            "id-5,2027-03-05 10:00:00,Queja,Phone\n",
            "id-6,2027-03-06 10:00:00,Queja,\n",
            "id-7,not-a-date,Queja,Phone\n",
            "id-8,2027-04-01 10:00:00,Técnico,Chat\n",
            "id-9,2027-04-02 10:00:00,Técnico,Chat\n",
            "id-10,2027-04-03 10:00:00,Técnico,Chat\n",
            "id-11,2027-04-04 10:00:00,Técnico,Chat\n",
        ),
    )
    .unwrap();

    let prepared = prepare_original_bank(temp.path(), &config(10)).unwrap();
    let projection = prepared
        .agent_inputs()
        .descriptive_contact_projection()
        .unwrap();
    assert_eq!(projection.included_contact_count(), 5);
    assert_eq!(projection.aggregates().len(), 1);
    let serialized_inputs = serde_json::to_string(prepared.agent_inputs()).unwrap();
    for forbidden in ["rejected_rows", "suppressed_cells"] {
        assert!(
            !serialized_inputs.contains(forbidden),
            "unexpected disclosure field or label: {forbidden}"
        );
    }
}

#[test]
fn original_contact_projection_fails_closed_on_truncated_or_duplicate_headers() {
    for contents in [
        "interaction_id,contact_reason,channel\nid-1,Complaint\n",
        "interaction_id,contact_reason,channel,channel\nid-1,Complaint,Phone,Web\n",
    ] {
        let temp = TempDir::new().unwrap();
        let table = temp.path().join("call_center_interactions");
        fs::create_dir_all(&table).unwrap();
        fs::write(table.join("part-000.csv"), contents).unwrap();

        assert!(prepare_original_bank(temp.path(), &config(10)).is_err());
    }
}

#[test]
fn snapshot_contact_suppression_floor_is_fixed_by_policy_v1() {
    let temp = TempDir::new().unwrap();
    let table = temp.path().join("call_center_interactions");
    fs::create_dir_all(&table).unwrap();
    fs::write(
        table.join("part-000.csv"),
        concat!(
            "interaction_id,contact_reason,channel\n",
            "id-1,Complaint,Phone\n",
            "id-2,Complaint,Phone\n",
            "id-3,Complaint,Phone\n",
            "id-4,Complaint,Phone\n",
        ),
    )
    .unwrap();
    let prepared = prepare_original_bank(temp.path(), &config(10)).unwrap();
    assert!(prepared.agent_inputs().contact_volumes().is_empty());
    let summary = prepared.agent_inputs().contact_projection().unwrap();
    assert_eq!(summary.minimum_cell_count(), 5);
    assert_eq!(summary.included_record_count(), 0);
}

#[test]
fn original_contact_counts_are_explicit_record_counts_and_blank_primary_reason_falls_back() {
    let temp = TempDir::new().unwrap();
    let table = temp.path().join("call_center_interactions");
    fs::create_dir_all(&table).unwrap();
    fs::write(
        table.join("part-000.csv"),
        concat!(
            "interaction_id,reason_category,contact_reason,channel\n",
            "same-id,,Complaint,Phone\n",
            "same-id,,Complaint,Phone\n",
            "same-id,,Complaint,Phone\n",
            "same-id,,Complaint,Phone\n",
            "same-id,,Complaint,Phone\n",
        ),
    )
    .unwrap();

    let prepared = prepare_original_bank(temp.path(), &config(10)).unwrap();
    let volume = &prepared.agent_inputs().contact_volumes()[0];

    assert_eq!(volume.reason(), "complaint");
    assert_eq!(volume.record_count(), 5);
    assert_eq!(
        prepared
            .agent_inputs()
            .contact_projection()
            .unwrap()
            .included_record_count(),
        5
    );
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
    e0_fixture_with_retry_counts(root, vec![Some(0), Some(0), Some(1), None]);
}

#[cfg(feature = "local-simulation")]
fn install_local_u12_contract(root: &Path) {
    let contract = serde_json::json!({
        "name": "platform_history",
        "version": "test-v1",
        "entities": {
            "case": { "fields": {
                "case_id": { "type": "VARCHAR", "required": true },
                "opened_at": { "type": "TIMESTAMP", "required": true }
            }},
            "tool_call": { "fields": {
                "case_id": { "type": "VARCHAR", "required": true },
                "event_time": { "type": "TIMESTAMP", "required": true },
                "status": { "type": "VARCHAR", "required": true,
                    "domain": ["ok", "error", "timeout", "denied"] }
            }}
        }
    });
    fs::write(
        root.join("contratos/platform_history.json"),
        contract.to_string(),
    )
    .expect("write U12-compatible local contract");
}

#[cfg(feature = "local-simulation")]
#[test]
fn local_simulation_issuer_returns_only_u08_verified_e0_rows_from_e0_package() {
    let temp = TempDir::new().expect("temporary source directory");
    e0_fixture(temp.path());
    install_local_u12_contract(temp.path());

    let local_config = PreparationConfig::new("pulso_local", "2025-07-01T00:00:00Z", 2)
        .expect("valid local simulation config");
    let (prepared, evidence) = prepare_e0_package_for_local_simulation(temp.path(), &local_config)
        .expect("prepare E0 with local U02/U04/U08 evidence");
    let evidence = evidence.expect("E0 package should issue verified local query evidence");

    assert_eq!(prepared.source_kind(), SourceKind::E0);
    assert_eq!(
        evidence.result().receipt().queried_table,
        "tool_call",
        "the receipt must come from the U08 tool_call read"
    );
    assert!(evidence.result().receipt().has_valid_digest());
    assert_eq!(evidence.result().rows().len(), 3);
    for row in evidence.result().rows() {
        assert_eq!(row.len(), 2);
        assert!(row.contains_key("event_time"));
        assert!(row.contains_key("technical_error"));
        let serialized = serde_json::to_string(row).expect("safe row JSON");
        assert!(!serialized.contains("synthetic-customer"));
        assert!(!serialized.contains("person@example.test"));
        assert!(!serialized.contains("call-"));
        assert!(!serialized.contains("read_txn"));
    }

    let one_case_config = PreparationConfig::new("pulso_local", "2025-07-01T00:00:00Z", 1)
        .expect("valid one-case cohort config");
    let (_, one_case_evidence) =
        prepare_e0_package_for_local_simulation(temp.path(), &one_case_config)
            .expect("prepare one-case cohort");
    let one_case_evidence = one_case_evidence.expect("verified one-case evidence");
    assert_eq!(one_case_evidence.result().receipt().row_count, 2);
    assert_ne!(
        evidence.result().receipt().transform_digest,
        one_case_evidence.result().receipt().transform_digest,
        "the selected Arranque count is committed by U04/U08 provenance"
    );

    write_parquet(
        &temp.path().join("datos").join("case.parquet"),
        Schema::new(vec![
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
        ]),
        vec![
            Arc::new(StringArray::from(vec!["case-b", "case-a", "case-c"])),
            Arc::new(
                TimestampMicrosecondArray::from(vec![5_000_000_i64, 10_000_000, 30_000_000])
                    .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec![
                "customer-b",
                "customer-a",
                "customer-c",
            ])),
            Arc::new(StringArray::from(vec!["chat", "phone", "web"])),
            Arc::new(StringArray::from(vec!["es", "es", "es"])),
            Arc::new(StringArray::from(vec!["payments", "payments", "account"])),
            Arc::new(StringArray::from(vec!["normal", "high", "normal"])),
        ],
    );
    let (_, changed_case_evidence) =
        prepare_e0_package_for_local_simulation(temp.path(), &one_case_config)
            .expect("prepare changed case cohort");
    let changed_case_evidence = changed_case_evidence.expect("verified changed-case evidence");
    assert_eq!(changed_case_evidence.result().receipt().row_count, 1);
    assert_ne!(
        one_case_evidence
            .result()
            .receipt()
            .source_snapshot_ref
            .digest,
        changed_case_evidence
            .result()
            .receipt()
            .source_snapshot_ref
            .digest,
        "case.parquet bytes are part of the U02 snapshot that U08 rereads"
    );
}

fn e0_fixture_with_retry_counts(root: &Path, retry_counts: Vec<Option<i64>>) {
    e0_fixture_with_statuses(root, retry_counts, ["timeout", "ok", "error", "denied"]);
}

fn e0_fixture_with_statuses(root: &Path, retry_counts: Vec<Option<i64>>, statuses: [&str; 4]) {
    assert_eq!(retry_counts.len(), 4);
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
        Field::new("retry_count", DataType::Int64, true),
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
            Arc::new(StringArray::from(statuses.to_vec())),
            Arc::new(BooleanArray::from(vec![true, true, true, false])),
            Arc::new(arrow_array::StringArray::from(vec![
                None,
                None,
                Some("{\"status\":\"updated\"}"),
                None,
            ])),
            Arc::new(arrow_array::Int64Array::from(retry_counts)),
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

    let identity_checks = Schema::new(vec![
        Field::new("check_id", DataType::Utf8, false),
        Field::new("case_id", DataType::Utf8, false),
        Field::new(
            "started_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("actor_role", DataType::Utf8, false),
        Field::new("result", DataType::Utf8, false),
        Field::new("correct", DataType::Int32, true),
    ]);
    write_parquet(
        &data.join("identity_check.parquet"),
        identity_checks,
        vec![
            Arc::new(StringArray::from(vec![
                "identity-row-1",
                "identity-row-2",
                "identity-row-3",
            ])),
            Arc::new(StringArray::from(vec!["case-b", "case-c", "case-a"])),
            Arc::new(
                TimestampMicrosecondArray::from(vec![10_500_000_i64, 10_600_000, 12_500_000])
                    .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec!["analyst", "analyst", "analyst"])),
            Arc::new(StringArray::from(vec!["verified", "verified", "failed"])),
            Arc::new(arrow_array::Int32Array::from(vec![
                Some(2_i32),
                None,
                Some(1),
            ])),
        ],
    );

    let copilot_queries = Schema::new(vec![
        Field::new("query_id", DataType::Utf8, false),
        Field::new("case_id", DataType::Utf8, false),
        Field::new(
            "event_time",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("query_signature", DataType::Utf8, false),
        Field::new("answered_by", DataType::Utf8, false),
        Field::new("tables_read", DataType::LargeUtf8, true),
        Field::new("columns_read", DataType::LargeUtf8, true),
        Field::new("sent_to_chat", DataType::Boolean, true),
    ]);
    write_parquet(
        &data.join("copilot_query.parquet"),
        copilot_queries,
        vec![
            Arc::new(StringArray::from(vec!["query-1"])),
            Arc::new(StringArray::from(vec!["case-a"])),
            Arc::new(TimestampMicrosecondArray::from(vec![12_800_000_i64]).with_timezone("UTC")),
            Arc::new(StringArray::from(vec!["safe-signature"])),
            Arc::new(StringArray::from(vec!["tool:lookup"])),
            Arc::new(arrow_array::LargeStringArray::from(vec![Some(
                "[\"customers\",\"transactions\"]",
            )])),
            Arc::new(arrow_array::LargeStringArray::from(vec![Some(
                "[\"account_status\"]",
            )])),
            Arc::new(BooleanArray::from(vec![Some(false)])),
        ],
    );

    let approvals = Schema::new(vec![
        Field::new("approval_id", DataType::Utf8, false),
        Field::new("case_id", DataType::Utf8, false),
        Field::new(
            "requested_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("requested_by_role", DataType::Utf8, false),
        Field::new("tool_id", DataType::Utf8, false),
        Field::new("executed_call_id", DataType::Utf8, true),
        Field::new("reason_code", DataType::Null, true),
        Field::new("policy_rule_id", DataType::Null, true),
        Field::new(
            "decided_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            true,
        ),
        Field::new("decision", DataType::Utf8, true),
    ]);
    write_parquet(
        &data.join("approval.parquet"),
        approvals,
        vec![
            Arc::new(StringArray::from(vec![
                "approval-missing",
                "approval-before",
                "approval-after",
            ])),
            Arc::new(StringArray::from(vec!["case-a", "case-a", "case-a"])),
            Arc::new(
                TimestampMicrosecondArray::from(vec![10_700_000_i64, 10_800_000, 10_900_000])
                    .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec!["analyst", "analyst", "analyst"])),
            Arc::new(StringArray::from(vec!["mutate_a", "mutate_b", "mutate_c"])),
            Arc::new(StringArray::from(vec![
                Some("call-2"),
                Some("call-3"),
                None,
            ])),
            Arc::new(arrow_array::NullArray::new(3)),
            Arc::new(arrow_array::NullArray::new(3)),
            Arc::new(
                TimestampMicrosecondArray::from(vec![None, Some(19_000_000), Some(21_000_000)])
                    .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec![
                Some("approved"),
                Some("approved"),
                Some("approved"),
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
    assert_eq!(cases[0].events().len(), 8);
    let tool_events = cases[0]
        .events()
        .iter()
        .filter(|event| event.event_kind() == "tool_call")
        .collect::<Vec<_>>();
    assert_eq!(tool_events.len(), 2);
    assert_eq!(tool_events[0].technical_error(), Some(false));
    assert_eq!(tool_events[1].technical_error(), Some(true));
    assert_eq!(tool_events[0].retry_count(), Some(0));
    assert_eq!(tool_events[1].retry_count(), Some(1));
    assert_eq!(
        cases[2]
            .events()
            .iter()
            .find(|event| event.event_kind() == "tool_call")
            .unwrap()
            .retry_count(),
        None,
        "null retry count remains unknown rather than becoming zero"
    );
    assert_eq!(tool_events[0].actor_role(), Some("tree"));
    assert!(
        tool_events[0]
            .tool_code()
            .is_some_and(|code| code.starts_with("sha256_") && code.len() <= 64)
    );
    let approval_event = cases[0]
        .events()
        .iter()
        .find(|event| event.event_kind() == "approval")
        .expect("approval should appear in safe event timeline");
    assert_eq!(approval_event.parent_event_ordinal(), None);
    let linked_tool_event = tool_events[1];
    assert_eq!(linked_tool_event.event_kind(), "tool_call");
    assert_eq!(linked_tool_event.parent_event_ordinal(), Some(2));
    assert!(
        cases[0]
            .events()
            .iter()
            .any(|event| event.event_kind() == "identity_check")
    );
    assert!(e0.agent_inputs().facts().iter().any(|fact| matches!(
        fact,
        improvement_engine_source_adapters::E0Fact::IdentityCheck {
            case_ordinal: 1,
            correct: Some(1),
            ..
        }
    )));
    assert!(e0.agent_inputs().facts().iter().any(|fact| matches!(
        fact,
        improvement_engine_source_adapters::E0Fact::CopilotQuery {
            tables_read,
            columns_read,
            answered_by,
            ..
        } if tables_read.len() == 2
            && columns_read.len() == 1
            && tables_read.iter().all(|value| value.starts_with("sha256_") && value.len() <= 64)
            && columns_read[0].starts_with("sha256_")
            && answered_by == "tool"
    )));
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
fn e0_retry_count_above_supported_u32_range_is_rejected_instead_of_wrapping() {
    let temp = TempDir::new().expect("temp dir");
    e0_fixture_with_retry_counts(
        temp.path(),
        vec![Some(i64::from(u32::MAX) + 1), Some(0), Some(1), None],
    );

    let error = match prepare_e0_package(temp.path(), &config(2)) {
        Err(error) => error,
        Ok(_) => panic!("oversized retry count should be rejected"),
    };
    assert!(matches!(
        error,
        AdapterError::InvalidInput("optional E0 retry count exceeds supported range")
    ));
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
        improvement_engine_source_adapters::E0Fact::IdentityCheck { event_time, .. }
        | improvement_engine_source_adapters::E0Fact::Turn { event_time, .. }
        | improvement_engine_source_adapters::E0Fact::RoutingStep { event_time, .. }
        | improvement_engine_source_adapters::E0Fact::CopilotQuery { event_time, .. } =>
            event_time.as_str() <= "1970-01-01T00:00:20Z",
        improvement_engine_source_adapters::E0Fact::Approval { requested_at, .. } =>
            requested_at.as_str() <= "1970-01-01T00:00:20Z",
    }));
    assert!(
        inputs.cases()[1]
            .events()
            .iter()
            .all(|event| event.event_time_unix_micros() <= 20_000_000)
    );
    let approvals = inputs
        .facts()
        .iter()
        .filter_map(|fact| match fact {
            improvement_engine_source_adapters::E0Fact::Approval {
                requested_at_unix_micros,
                decided_at_unix_micros,
                decision,
                ..
            } => Some((
                *requested_at_unix_micros,
                *decided_at_unix_micros,
                decision.as_deref(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(approvals.len(), 3);
    assert_eq!(
        approvals
            .iter()
            .filter(|(_, decided, decision)| decided.is_none() && decision.is_none())
            .count(),
        2,
        "missing timestamps and post-cutoff decisions must both be redacted"
    );
    assert!(approvals.iter().any(
        |(_, decided, decision)| *decided == Some(19_000_000) && *decision == Some("approved")
    ));
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
