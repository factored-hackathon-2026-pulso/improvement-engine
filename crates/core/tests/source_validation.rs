use improvement_engine_core::source_validation::{
    QualityFindingKind, SourceContract, SourceSnapshot, load_canonical_contracts,
    validate_source_file,
};
use std::path::Path;

const CONTRACT: &str = include_str!("../../../contracts/sources/call_center_interactions.v1.json");
const SNAPSHOT: &str =
    include_str!("../../../contracts/fixtures/sources-v1/validation/source-snapshot.json");
const SOURCE: &[u8] = include_bytes!(
    "../../../contracts/fixtures/sources-v1/validation/call_center_interactions.csv"
);

fn contract() -> SourceContract {
    SourceContract::from_json(CONTRACT).expect("canonical contract is valid")
}

fn snapshot() -> SourceSnapshot {
    SourceSnapshot::from_json(SNAPSHOT).expect("synthetic snapshot is valid")
}

#[test]
fn loads_canonical_json_contracts_in_deterministic_filename_order() {
    let contracts = load_canonical_contracts(Path::new("../../contracts/sources"))
        .expect("canonical contracts are readable");

    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].table, "call_center_interactions");
}

#[test]
fn accepts_a_golden_synthetic_source_and_preserves_snapshot_provenance() {
    assert_eq!(
        snapshot()
            .source_file_seal("call_center_interactions")
            .unwrap()
            .partition_inventory_digest(),
        None
    );
    let report = validate_source_file(&contract(), &snapshot(), SOURCE)
        .expect("fixture has a matching source entry");

    assert!(report.findings.is_empty());
    assert_eq!(report.provenance.tenant_id, "demo");
    assert_eq!(report.provenance.source_namespace, "bank_history");
    assert_eq!(report.provenance.world_ref, "supplied-synthetic-v1");
    assert_eq!(report.provenance.observed_cutoff, "2026-09-01T00:00:00Z");
}

#[test]
fn optional_partition_inventory_seal_is_distinct_from_original_file_digest() {
    let mut snapshot_json = serde_json::from_str::<serde_json::Value>(SNAPSHOT).unwrap();
    let source_file_digest = snapshot_json["sources"][0]["file_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    snapshot_json["sources"][0]["partition_inventory_digest"] =
        serde_json::Value::String(format!("sha256:{}", "a".repeat(64)));
    let snapshot = SourceSnapshot::from_json(&snapshot_json.to_string()).unwrap();
    let seal = snapshot
        .source_file_seal("call_center_interactions")
        .unwrap();
    assert_eq!(seal.file_digest(), source_file_digest);
    assert_eq!(
        seal.partition_inventory_digest(),
        Some(format!("sha256:{}", "a".repeat(64))).as_deref()
    );
}

#[test]
fn rejects_invalid_optional_partition_inventory_digest() {
    for digest in [
        serde_json::Value::String("not-a-digest".into()),
        serde_json::Value::Null,
    ] {
        let mut snapshot_json = serde_json::from_str::<serde_json::Value>(SNAPSHOT).unwrap();
        snapshot_json["sources"][0]["partition_inventory_digest"] = digest;
        assert!(SourceSnapshot::from_json(&snapshot_json.to_string()).is_err());
    }
}

#[test]
fn reports_header_drift_deterministically() {
    let source = String::from_utf8_lossy(SOURCE).replace("contact_reason", "contact_reasons");
    let report = validate_source_file(&contract(), &snapshot(), source.as_bytes())
        .expect("fixture has a matching source entry");

    assert_eq!(report.findings.len(), 3);
    assert_eq!(
        report.findings[0].kind,
        QualityFindingKind::HeaderColumnsMismatch
    );
    assert_eq!(
        report.findings[1].kind,
        QualityFindingKind::HeaderDigestMismatch
    );
    assert_eq!(
        report.findings[2].kind,
        QualityFindingKind::FileDigestMismatch
    );
}

#[test]
fn reports_file_digest_mismatch_when_the_header_is_unchanged() {
    let mut source = SOURCE.to_vec();
    source.extend_from_slice(b"synthetic-002,customer-002,2026-08-31T09:00:00Z,technical,chat,true,false,agent-002,30,5\n");
    let report = validate_source_file(&contract(), &snapshot(), &source)
        .expect("fixture has a matching source entry");

    assert_eq!(report.findings.len(), 1);
    assert_eq!(
        report.findings[0].kind,
        QualityFindingKind::FileDigestMismatch
    );
}

#[test]
fn reports_disallowed_policy_classifications_in_stable_column_order() {
    let invalid_policy_contract = CONTRACT
        .replace(
            "\"classification\": \"internal\"}\n  ]",
            "\"classification\": \"aggregated\"}\n  ]",
        )
        .replace(
            "\"permitted_classifications\": [\"internal\", \"pseudonymized\", \"aggregated\"]",
            "\"permitted_classifications\": [\"internal\", \"pseudonymized\"]",
        );
    let report = validate_source_file(
        &SourceContract::from_json(&invalid_policy_contract).expect("shape remains valid"),
        &snapshot(),
        SOURCE,
    )
    .expect("fixture has a matching source entry");

    assert_eq!(report.findings.len(), 2);
    assert_eq!(
        report.findings[0].kind,
        QualityFindingKind::SourceContractDigestMismatch
    );
    assert_eq!(
        report.findings[1].kind,
        QualityFindingKind::PolicyClassificationViolation
    );
    assert_eq!(
        report.findings[1].column.as_deref(),
        Some("wait_time_seconds")
    );
}

#[test]
fn reports_a_contract_digest_mismatch_before_file_findings() {
    let different_contract = CONTRACT.replace('\n', "\n\n");
    let report = validate_source_file(
        &SourceContract::from_json(&different_contract).expect("shape remains valid"),
        &snapshot(),
        SOURCE,
    )
    .expect("snapshot source is selected by the expected table");

    assert_eq!(
        report.findings[0].kind,
        QualityFindingKind::SourceContractDigestMismatch
    );
}

#[test]
fn rejects_contracts_that_break_readonly_or_schema_invariants() {
    let writable = CONTRACT.replace("\"read_only\": true", "\"read_only\": false");
    assert!(SourceContract::from_json(&writable).is_err());

    let unknown_field = CONTRACT.replacen('{', "{\"unexpected\":true,", 1);
    assert!(SourceContract::from_json(&unknown_field).is_err());

    let unknown_classification = CONTRACT.replace("\"internal\"", "\"restricted\"");
    assert!(SourceContract::from_json(&unknown_classification).is_err());
}

#[test]
fn rejects_snapshots_with_duplicate_tables_or_unknown_metadata() {
    let mut duplicate = serde_json::from_str::<serde_json::Value>(SNAPSHOT).expect("fixture JSON");
    let source = duplicate["sources"][0].clone();
    duplicate["sources"]
        .as_array_mut()
        .expect("sources array")
        .push(source);
    assert!(SourceSnapshot::from_json(&duplicate.to_string()).is_err());

    let unknown_field = SNAPSHOT.replacen('{', "{\"unexpected\":true,", 1);
    assert!(SourceSnapshot::from_json(&unknown_field).is_err());
}

#[test]
fn rejects_a_snapshot_reference_with_the_wrong_contract_id_or_version() {
    let wrong_id = SNAPSHOT.replace("\"id\": \"call_center_interactions\"", "\"id\": \"other\"");
    let report = validate_source_file(
        &contract(),
        &SourceSnapshot::from_json(&wrong_id).expect("snapshot shape remains valid"),
        SOURCE,
    );

    assert!(report.is_err());
}
