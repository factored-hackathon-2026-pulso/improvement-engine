use std::collections::BTreeMap;

use improvement_engine_core::enriched_history::{
    EnrichedHistoryAdapter, EnrichedHistoryError, EnrichedHistoryManifest, PackageFile,
    ProvenanceDigests, QualityFinding, QualityFindingKind, TableInput,
};
use improvement_engine_core::source_validation::SourceSnapshot;
use serde_json::json;

const CUTOFF: &str = "2025-06-30T23:59:59Z";

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn fields() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("case_id".to_owned(), CUTOFF.to_owned()),
        ("event_time".to_owned(), CUTOFF.to_owned()),
        ("ingested_at".to_owned(), CUTOFF.to_owned()),
        ("topic".to_owned(), CUTOFF.to_owned()),
        ("details".to_owned(), CUTOFF.to_owned()),
    ])
}

fn manifest() -> EnrichedHistoryManifest {
    EnrichedHistoryManifest::new(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                "2025-06-30T23:59:59Z",
            )
            .with_field_availability(fields()),
        ],
    )
}

fn input(rows: Vec<serde_json::Value>) -> TableInput {
    TableInput::new(
        ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
        rows,
    )
}

fn valid_row() -> serde_json::Value {
    json!({
        "case_id": "case-1",
        "event_time": "2025-06-01T10:00:00Z",
        "ingested_at": "2025-06-01T10:01:00Z",
        "topic": "disputar_cargo"
    })
}

#[test]
fn sealed_manifest_preserves_namespace_world_cutoff_and_every_provenance_digest() {
    let adapter = EnrichedHistoryAdapter::from_manifest(manifest()).unwrap();
    let report = adapter
        .discovery_table("case", input(vec![valid_row()]))
        .unwrap();

    assert_eq!(report.provenance.source_namespace, "platform_history");
    assert_eq!(report.provenance.world_ref, "e0-disputes-2025");
    assert_eq!(report.provenance.observed_cutoff, CUTOFF);
    assert_eq!(report.provenance.digests.file_digest, digest('a'));
    assert_eq!(report.provenance.digests.schema_digest, digest('b'));
    assert_eq!(report.provenance.digests.transform_digest, digest('c'));
    assert_eq!(report.provenance.digests.policy_digest, digest('d'));
    assert_eq!(report.rows, vec![valid_row()]);
    assert!(report.findings.is_empty());
}

#[test]
fn discovery_rejects_rows_or_files_that_were_not_available_by_the_sealed_cutoff() {
    let adapter = EnrichedHistoryAdapter::from_manifest(manifest()).unwrap();
    let future_row = json!({
        "case_id": "case-2",
        "event_time": "2025-07-01T00:00:00Z",
        "ingested_at": "2025-06-30T23:00:00Z"
    });
    assert_eq!(
        adapter
            .discovery_table("case", input(vec![future_row]))
            .unwrap_err(),
        EnrichedHistoryError::FutureData {
            table: "case".to_owned(),
            field: "event_time".to_owned(),
        }
    );

    let unavailable_manifest = EnrichedHistoryManifest::new(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        vec![PackageFile::new(
            "case",
            ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
            "2025-07-01T00:00:00Z",
        )],
    );
    assert_eq!(
        EnrichedHistoryAdapter::from_manifest(unavailable_manifest).unwrap_err(),
        EnrichedHistoryError::UnavailableFile {
            table: "case".to_owned()
        }
    );
}

#[test]
fn impossible_utc_clock_or_a_field_group_available_after_cutoff_is_rejected() {
    let adapter = EnrichedHistoryAdapter::from_manifest(manifest()).unwrap();
    assert_eq!(
        adapter
            .discovery_table(
                "case",
                input(vec![json!({
                    "event_time": "2025-99-99T99:99:99Z",
                    "ingested_at": "2025-06-01T10:01:00Z"
                })]),
            )
            .unwrap_err(),
        EnrichedHistoryError::InvalidAvailabilityClock {
            table: "case".to_owned(),
            field: "event_time".to_owned(),
        }
    );
    let mut delayed_fields = fields();
    delayed_fields.insert("topic".to_owned(), "2025-07-01T00:00:00Z".to_owned());
    let delayed = EnrichedHistoryManifest::new(
        "platform_history",
        "world",
        CUTOFF,
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(delayed_fields),
        ],
    );
    assert_eq!(
        EnrichedHistoryAdapter::from_manifest(delayed).unwrap_err(),
        EnrichedHistoryError::UnavailableField {
            table: "case".to_owned(),
            field: "topic".to_owned(),
        }
    );
}

#[test]
fn discovery_blocks_precomputed_signals_labels_final_answers_and_nested_leakage() {
    let adapter = EnrichedHistoryAdapter::from_manifest(manifest()).unwrap();
    assert_eq!(
        adapter
            .discovery_table("signal", input(vec![valid_row()]))
            .unwrap_err(),
        EnrichedHistoryError::ForbiddenDiscoveryTable {
            table: "signal".to_owned()
        }
    );
    assert_eq!(
        adapter
            .discovery_table(
                "case",
                input(vec![json!({
                    "event_time": "2025-06-01T10:00:00Z",
                    "ingested_at": "2025-06-01T10:01:00Z",
                    "details": {"final_resolution": "refund"}
                })]),
            )
            .unwrap_err(),
        EnrichedHistoryError::ForbiddenDiscoveryField {
            table: "case".to_owned(),
            field: "final_resolution".to_owned(),
        }
    );
}

#[test]
fn discovery_is_safe_by_default_for_field_availability_and_nested_array_leakage() {
    let adapter = EnrichedHistoryAdapter::from_manifest(manifest()).unwrap();
    assert_eq!(
        adapter
            .discovery_table(
                "case",
                input(vec![json!({
                    "event_time": "2025-06-01T10:00:00Z",
                    "ingested_at": "2025-06-01T10:01:00Z",
                    "resolved_at": "2025-07-02T00:00:00Z"
                })]),
            )
            .unwrap_err(),
        EnrichedHistoryError::UnavailableField {
            table: "case".to_owned(),
            field: "resolved_at".to_owned(),
        }
    );
    assert_eq!(
        adapter
            .discovery_table(
                "case",
                input(vec![json!({
                    "event_time": "2025-06-01T10:00:00Z",
                    "ingested_at": "2025-06-01T10:01:00Z",
                    "details": [[{"Precomputed_Signal": "answer"}]]
                })]),
            )
            .unwrap_err(),
        EnrichedHistoryError::ForbiddenDiscoveryField {
            table: "case".to_owned(),
            field: "Precomputed_Signal".to_owned(),
        }
    );
}

#[test]
fn sealed_provenance_drift_is_a_deterministic_quality_finding_not_a_silent_acceptance() {
    let adapter = EnrichedHistoryAdapter::from_manifest(manifest()).unwrap();
    assert_eq!(
        adapter
            .discovery_table(
                "case",
                TableInput::new(
                    ProvenanceDigests::new(digest('e'), digest('b'), digest('c'), digest('d')),
                    vec![valid_row()],
                ),
            )
            .unwrap_err(),
        EnrichedHistoryError::QualityBlocked {
            table: "case".to_owned(),
            findings: vec![QualityFinding {
                kind: QualityFindingKind::FileDigestMismatch,
                table: "case".to_owned(),
            }],
        }
    );
}

#[test]
fn malformed_or_unsealed_inputs_fail_deterministically_before_rows_are_exposed() {
    let duplicate = EnrichedHistoryManifest::new(
        "platform_history",
        "world",
        CUTOFF,
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(fields()),
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(fields()),
        ],
    );
    assert_eq!(
        EnrichedHistoryAdapter::from_manifest(duplicate).unwrap_err(),
        EnrichedHistoryError::DuplicateTable {
            table: "case".to_owned()
        }
    );

    let adapter = EnrichedHistoryAdapter::from_manifest(manifest()).unwrap();
    assert_eq!(
        adapter
            .discovery_table(
                "case",
                input(vec![json!({"event_time": "2025-06-01T00:00:00Z"})]),
            )
            .unwrap_err(),
        EnrichedHistoryError::MissingAvailabilityClock {
            table: "case".to_owned(),
            field: "ingested_at".to_owned(),
        }
    );
}

#[test]
fn report_order_is_stable_across_multiple_tables() {
    let manifest = EnrichedHistoryManifest::new(
        "platform_history",
        "world",
        CUTOFF,
        vec![
            PackageFile::new(
                "turn",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(fields()),
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(fields()),
        ],
    );
    let adapter = EnrichedHistoryAdapter::from_manifest(manifest).unwrap();
    let mut inputs = BTreeMap::new();
    inputs.insert("turn".to_owned(), input(vec![valid_row()]));
    inputs.insert("case".to_owned(), input(vec![valid_row()]));
    let reports = adapter.discovery_tables(inputs).unwrap();
    assert_eq!(
        reports
            .iter()
            .map(|report| report.table.as_str())
            .collect::<Vec<_>>(),
        vec!["case", "turn"]
    );
}

#[test]
fn enriched_manifest_cannot_substitute_world_or_cutoff_of_the_existing_source_snapshot() {
    let snapshot = SourceSnapshot::from_json(
        r#"{
          "contract_version":{"major":1,"minor":0},
          "tenant_id":"tenant-a",
          "source_namespace":"platform_history",
          "world_ref":"e0-disputes-2025",
          "observed_cutoff":"2025-06-30T23:59:59Z",
          "sources":[{
            "table":"case","uri":"file://fixture.csv",
            "file_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "header_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "row_count":1,
            "source_contract_ref":{"id":"case","version":"v1","digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}
          }]
        }"#,
    )
    .unwrap();
    assert!(EnrichedHistoryAdapter::from_snapshot(manifest(), &snapshot).is_ok());
    assert_eq!(
        EnrichedHistoryAdapter::from_snapshot(
            EnrichedHistoryManifest::new(
                "platform_history",
                "different-world",
                CUTOFF,
                manifest().files,
            ),
            &snapshot,
        )
        .unwrap_err(),
        EnrichedHistoryError::SnapshotProvenanceMismatch
    );
    assert_eq!(
        EnrichedHistoryAdapter::from_snapshot(
            EnrichedHistoryManifest::new(
                "another_namespace",
                "e0-disputes-2025",
                CUTOFF,
                manifest().files,
            ),
            &snapshot,
        )
        .unwrap_err(),
        EnrichedHistoryError::SnapshotProvenanceMismatch
    );
}
