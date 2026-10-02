use std::collections::BTreeMap;

use improvement_engine_core::enriched_history::{
    AvailabilityClockMode, AvailabilityProfile, EnrichedHistoryAdapter, EnrichedHistoryError,
    EnrichedHistoryManifest, PackageFile, ProvenanceDigests, QualityFinding, QualityFindingKind,
    ReplayRowAvailability, TableInput, replay_projection_digest,
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
        AvailabilityClockMode::observed_ingested_at(),
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

fn replay_availability(rows: &[serde_json::Value]) -> Vec<ReplayRowAvailability> {
    rows.iter()
        .map(|row| {
            let object = row.as_object().expect("replay fixture rows are objects");
            let event_time = object
                .get("event_time")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(CUTOFF)
                .to_owned();
            ReplayRowAvailability::new(
                object
                    .keys()
                    .map(|field| (field.clone(), event_time.clone()))
                    .collect(),
            )
        })
        .collect()
}

fn replay_snapshot() -> SourceSnapshot {
    replay_snapshot_with(&digest('b'), &digest('c'))
}

fn replay_snapshot_with(header_digest: &str, contract_digest: &str) -> SourceSnapshot {
    SourceSnapshot::from_json(
        &json!({
          "contract_version":{"major":1,"minor":0},
          "tenant_id":"tenant-a",
          "source_namespace":"platform_history",
          "world_ref":"e0-disputes-2025",
          "observed_cutoff":"2025-06-30T23:59:59Z",
          "sources":[{
            "table":"case","uri":"file://fixture.csv",
            "file_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "header_digest": header_digest,
            "row_count":1,
            "source_contract_ref":{"id":"case","version":"v1","digest": contract_digest}
          }]
        })
        .to_string(),
    )
    .unwrap()
}

fn replay_manifest(
    rows: &[serde_json::Value],
    snapshot: &SourceSnapshot,
) -> EnrichedHistoryManifest {
    let availability = replay_availability(rows);
    let fields = rows
        .iter()
        .flat_map(|row| {
            row.as_object()
                .expect("replay fixture rows are objects")
                .keys()
        })
        .map(|field| (field.clone(), CUTOFF.to_owned()))
        .collect();
    EnrichedHistoryManifest::new_replay(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        AvailabilityProfile::new(
            "e0_replay_clock",
            1,
            AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed"),
            snapshot.binding_digest(),
        ),
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(fields)
            .with_replay_projection_digest(replay_projection_digest(rows, &availability))
            .with_source_file_seal(snapshot.source_file_seal("case").unwrap()),
        ],
    )
}

fn replay_input(rows: Vec<serde_json::Value>) -> TableInput {
    let availability = replay_availability(&rows);
    input(rows).with_replay_row_availability(availability)
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
fn replay_at_event_time_admits_e0_rows_without_physical_ingested_at_and_labels_the_zero_lag_assumption()
 {
    let rows = vec![json!({
        "case_id": "case-1",
        "event_time": "2025-06-01T10:00:00Z",
        "topic": "disputar_cargo"
    })];
    let snapshot = replay_snapshot();
    let manifest = replay_manifest(&rows, &snapshot);

    let report = EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot)
        .unwrap()
        .discovery_table("case", replay_input(rows))
        .unwrap();

    assert_eq!(
        report.provenance.availability_clock,
        AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed")
    );
}

#[test]
fn replay_at_event_time_rejects_a_manifest_that_claims_a_physical_ingestion_clock() {
    let snapshot = replay_snapshot();
    let manifest = EnrichedHistoryManifest::new_replay(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        AvailabilityProfile::new(
            "e0_replay_clock",
            1,
            AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed"),
            snapshot.binding_digest(),
        ),
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(fields())
            .with_source_file_seal(snapshot.source_file_seal("case").unwrap()),
        ],
    );

    assert_eq!(
        EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot).unwrap_err(),
        EnrichedHistoryError::UnexpectedAvailabilityClock {
            table: "case".to_owned(),
            field: "ingested_at".to_owned(),
        }
    );
}

#[test]
fn replay_at_event_time_preserves_cutoff_and_discovery_leakage_guards() {
    let future_rows = vec![json!({
        "case_id": "case-future",
        "event_time": "2025-07-01T00:00:00Z",
        "topic": "disputar_cargo"
    })];
    let future_snapshot = replay_snapshot();
    let future_adapter = EnrichedHistoryAdapter::from_snapshot(
        replay_manifest(&future_rows, &future_snapshot),
        &future_snapshot,
    )
    .unwrap();

    assert_eq!(
        future_adapter
            .discovery_table("case", replay_input(future_rows))
            .unwrap_err(),
        EnrichedHistoryError::FutureData {
            table: "case".to_owned(),
            field: "event_time".to_owned(),
        }
    );
    let leaked_rows = vec![json!({
        "case_id": "case-leak",
        "event_time": "2025-06-01T10:00:00Z",
        "topic": {"final_resolution": "refund"}
    })];
    let leaked_snapshot = replay_snapshot();
    let leaked_adapter = EnrichedHistoryAdapter::from_snapshot(
        replay_manifest(&leaked_rows, &leaked_snapshot),
        &leaked_snapshot,
    )
    .unwrap();
    assert_eq!(
        leaked_adapter
            .discovery_table("case", replay_input(leaked_rows))
            .unwrap_err(),
        EnrichedHistoryError::ForbiddenDiscoveryField {
            table: "case".to_owned(),
            field: "final_resolution".to_owned(),
        }
    );
}

#[test]
fn replay_at_event_time_rejects_rows_with_physical_ingested_at_to_avoid_mode_ambiguity() {
    let snapshot = replay_snapshot();
    let rows = vec![json!({
        "case_id": "case-1",
        "event_time": "2025-06-01T10:00:00Z",
        "ingested_at": "2025-06-01T10:01:00Z",
        "topic": "disputar_cargo"
    })];
    let availability = replay_availability(&rows);
    let manifest = EnrichedHistoryManifest::new_replay(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        AvailabilityProfile::new(
            "e0_replay_clock",
            1,
            AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed"),
            snapshot.binding_digest(),
        ),
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(BTreeMap::from([
                ("case_id".to_owned(), CUTOFF.to_owned()),
                ("event_time".to_owned(), CUTOFF.to_owned()),
                ("topic".to_owned(), CUTOFF.to_owned()),
            ]))
            .with_replay_projection_digest(replay_projection_digest(&rows, &availability))
            .with_source_file_seal(snapshot.source_file_seal("case").unwrap()),
        ],
    );
    let adapter = EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot).unwrap();

    assert_eq!(
        adapter
            .discovery_table(
                "case",
                input(rows).with_replay_row_availability(availability),
            )
            .unwrap_err(),
        EnrichedHistoryError::UnexpectedAvailabilityClock {
            table: "case".to_owned(),
            field: "ingested_at".to_owned(),
        }
    );
}

#[test]
fn availability_clock_is_explicit_and_replay_assumptions_are_machine_validated() {
    let snapshot = replay_snapshot();
    let manifest = EnrichedHistoryManifest::new_replay(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        AvailabilityProfile::new(
            "e0_replay_clock",
            1,
            AvailabilityClockMode::replay_at_event_time("human-readable but unversioned"),
            snapshot.binding_digest(),
        ),
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(BTreeMap::from([
                ("case_id".to_owned(), CUTOFF.to_owned()),
                ("event_time".to_owned(), CUTOFF.to_owned()),
            ]))
            .with_source_file_seal(snapshot.source_file_seal("case").unwrap()),
        ],
    );
    assert_eq!(
        EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot).unwrap_err(),
        EnrichedHistoryError::InvalidReplayAssumption
    );
}

#[test]
fn unversioned_n_minus_one_manifest_migrates_only_to_observed_ingestion() {
    let legacy = EnrichedHistoryManifest::from_json(
        r#"{
          "source_namespace":"platform_history",
          "world_ref":"e0-disputes-2025",
          "observed_cutoff":"2025-06-30T23:59:59Z",
          "files":[{
            "table":"case",
            "digests":{
              "file_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "schema_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "transform_digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
              "policy_digest":"sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            },
            "available_at":"2025-06-30T23:59:59Z",
            "field_availability":{
              "case_id":"2025-06-30T23:59:59Z",
              "event_time":"2025-06-30T23:59:59Z",
              "ingested_at":"2025-06-30T23:59:59Z"
            }
          }]
        }"#,
    )
    .unwrap();
    assert_eq!(legacy.manifest_version, 1);
    assert_eq!(
        legacy.availability_clock,
        AvailabilityClockMode::observed_ingested_at()
    );
    assert!(EnrichedHistoryAdapter::from_manifest(legacy).is_ok());
}

#[test]
fn replay_at_event_time_rejects_a_field_that_only_became_available_after_its_event() {
    let snapshot = replay_snapshot();
    let rows = vec![json!({
        "case_id": "case-1",
        "event_time": "2025-06-01T10:00:00Z",
        "topic": "disputar_cargo"
    })];
    let availability = vec![ReplayRowAvailability::new(BTreeMap::from([
        ("case_id".to_owned(), "2025-06-01T10:00:00Z".to_owned()),
        ("event_time".to_owned(), "2025-06-01T10:00:00Z".to_owned()),
        ("topic".to_owned(), "2025-06-01T10:01:00Z".to_owned()),
    ]))];
    let manifest = EnrichedHistoryManifest::new_replay(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        AvailabilityProfile::new(
            "e0_replay_clock",
            1,
            AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed"),
            snapshot.binding_digest(),
        ),
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(BTreeMap::from([
                ("case_id".to_owned(), CUTOFF.to_owned()),
                ("event_time".to_owned(), CUTOFF.to_owned()),
                ("topic".to_owned(), CUTOFF.to_owned()),
            ]))
            .with_replay_projection_digest(replay_projection_digest(&rows, &availability))
            .with_source_file_seal(snapshot.source_file_seal("case").unwrap()),
        ],
    );
    let adapter = EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot).unwrap();

    assert_eq!(
        adapter
            .discovery_table(
                "case",
                input(rows).with_replay_row_availability(availability),
            )
            .unwrap_err(),
        EnrichedHistoryError::FutureFieldAtEvent {
            table: "case".to_owned(),
            field: "topic".to_owned(),
        }
    );
}

#[test]
fn replay_profile_commits_the_clock_and_exact_source_snapshot_bytes() {
    let snapshot_raw = r#"{
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
    }"#;
    let snapshot = SourceSnapshot::from_json(snapshot_raw).unwrap();
    let rows = vec![json!({
        "case_id": "case-1",
        "event_time": "2025-06-01T10:00:00Z",
        "topic": "disputar_cargo"
    })];
    let availability = replay_availability(&rows);
    let profile = AvailabilityProfile::new(
        "e0_replay_clock",
        1,
        AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed"),
        snapshot.binding_digest(),
    );
    let manifest = EnrichedHistoryManifest::new_replay(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        profile,
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(BTreeMap::from([
                ("case_id".to_owned(), CUTOFF.to_owned()),
                ("event_time".to_owned(), CUTOFF.to_owned()),
                ("topic".to_owned(), CUTOFF.to_owned()),
            ]))
            .with_replay_projection_digest(replay_projection_digest(&rows, &availability))
            .with_source_file_seal(snapshot.source_file_seal("case").unwrap()),
        ],
    );

    assert_eq!(
        EnrichedHistoryAdapter::from_manifest(manifest.clone()).unwrap_err(),
        EnrichedHistoryError::ReplaySnapshotBindingRequired
    );
    assert!(EnrichedHistoryAdapter::from_snapshot(manifest.clone(), &snapshot).is_ok());

    let mut mutated = manifest;
    mutated.availability_clock = AvailabilityClockMode::observed_ingested_at();
    assert_eq!(
        EnrichedHistoryAdapter::from_manifest(mutated).unwrap_err(),
        EnrichedHistoryError::InvalidAvailabilityProfile
    );

    let same_provenance_different_bytes =
        SourceSnapshot::from_json(&format!("\n{snapshot_raw}")).unwrap();
    let mismatched_profile = AvailabilityProfile::new(
        "e0_replay_clock",
        1,
        AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed"),
        same_provenance_different_bytes.binding_digest(),
    );
    let mismatched_manifest = EnrichedHistoryManifest::new_replay(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        mismatched_profile,
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(BTreeMap::from([
                ("case_id".to_owned(), CUTOFF.to_owned()),
                ("event_time".to_owned(), CUTOFF.to_owned()),
                ("topic".to_owned(), CUTOFF.to_owned()),
            ]))
            .with_replay_projection_digest(replay_projection_digest(&rows, &availability))
            .with_source_file_seal(snapshot.source_file_seal("case").unwrap()),
        ],
    );
    assert_eq!(
        EnrichedHistoryAdapter::from_snapshot(mismatched_manifest, &snapshot).unwrap_err(),
        EnrichedHistoryError::SnapshotAvailabilityProfileMismatch
    );
}

#[test]
fn snapshot_binding_rejects_unlisted_tables_and_divergent_source_file_digests() {
    let snapshot = replay_snapshot();
    let rows = vec![json!({
        "case_id": "case-1",
        "event_time": "2025-06-01T10:00:00Z",
        "topic": "disputar_cargo"
    })];

    let mut unlisted = replay_manifest(&rows, &snapshot);
    unlisted.files[0].table = "unlisted".to_owned();
    assert_eq!(
        EnrichedHistoryAdapter::from_snapshot(unlisted, &snapshot).unwrap_err(),
        EnrichedHistoryError::SnapshotSourceNotListed {
            table: "unlisted".to_owned(),
        }
    );

    let mut divergent_file = replay_manifest(&rows, &snapshot);
    divergent_file.files[0].digests.file_digest = digest('f');
    assert_eq!(
        EnrichedHistoryAdapter::from_snapshot(divergent_file, &snapshot).unwrap_err(),
        EnrichedHistoryError::SourceFileSealMismatch {
            table: "case".to_owned(),
        }
    );
}

#[test]
fn snapshot_binding_compares_header_and_source_contract_seals_not_just_file_digest() {
    let canonical_snapshot = replay_snapshot();
    let rows = vec![json!({
        "case_id": "case-1",
        "event_time": "2025-06-01T10:00:00Z",
        "topic": "disputar_cargo"
    })];

    for candidate_snapshot in [
        replay_snapshot_with(&digest('f'), &digest('c')),
        replay_snapshot_with(&digest('b'), &digest('e')),
    ] {
        let mut manifest = replay_manifest(&rows, &candidate_snapshot);
        manifest.files[0] = manifest.files[0]
            .clone()
            .with_source_file_seal(canonical_snapshot.source_file_seal("case").unwrap());
        assert_eq!(
            EnrichedHistoryAdapter::from_snapshot(manifest, &candidate_snapshot).unwrap_err(),
            EnrichedHistoryError::SourceFileSealMismatch {
                table: "case".to_owned(),
            }
        );
    }
}

#[test]
fn v1_manifest_remains_observed_only_while_replay_requires_v2_profile() {
    assert!(EnrichedHistoryAdapter::from_manifest(manifest()).is_ok());
    let replay_v1 = EnrichedHistoryManifest::new(
        "platform_history",
        "e0-disputes-2025",
        CUTOFF,
        AvailabilityClockMode::replay_at_event_time("e0_ingestion_lag_zero_assumed"),
        vec![
            PackageFile::new(
                "case",
                ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                CUTOFF,
            )
            .with_field_availability(BTreeMap::from([
                ("case_id".to_owned(), CUTOFF.to_owned()),
                ("event_time".to_owned(), CUTOFF.to_owned()),
                ("topic".to_owned(), CUTOFF.to_owned()),
            ])),
        ],
    );
    assert_eq!(
        EnrichedHistoryAdapter::from_manifest(replay_v1).unwrap_err(),
        EnrichedHistoryError::UnsupportedManifestVersion {
            manifest_version: 1,
        }
    );
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
        AvailabilityClockMode::observed_ingested_at(),
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
        AvailabilityClockMode::observed_ingested_at(),
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
        AvailabilityClockMode::observed_ingested_at(),
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
        AvailabilityClockMode::observed_ingested_at(),
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
    let mut bound_manifest = manifest();
    bound_manifest.files[0] = bound_manifest.files[0]
        .clone()
        .with_source_file_seal(snapshot.source_file_seal("case").unwrap());
    assert!(EnrichedHistoryAdapter::from_snapshot(bound_manifest, &snapshot).is_ok());
    assert_eq!(
        EnrichedHistoryAdapter::from_snapshot(
            EnrichedHistoryManifest::new(
                "platform_history",
                "different-world",
                CUTOFF,
                AvailabilityClockMode::observed_ingested_at(),
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
                AvailabilityClockMode::observed_ingested_at(),
                manifest().files,
            ),
            &snapshot,
        )
        .unwrap_err(),
        EnrichedHistoryError::SnapshotProvenanceMismatch
    );
}
