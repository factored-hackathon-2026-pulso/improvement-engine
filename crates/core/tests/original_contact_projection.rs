use improvement_engine_core::source_validation::SourceSnapshot;
use improvement_engine_core::source_validation::{SourceContract, validate_source_file};
use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactReference, ArtifactRepository, InMemoryArtifactRepository,
    original_contact_projection::*,
};
use serde_json::json;
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

const CONTACTS: &str = concat!(
    "interaction_id,customer_id,agent_id,interaction_date,reason_category,contact_reason,channel,was_resolved,requires_followup,was_escalated,duration_seconds,wait_time_seconds,description\n",
    "id-1,c-1,a-1,2026-04-03T08:00:00Z,Queja,private free text,Phone,false,true,true,120,30,\"quoted, \"\"private\"\"\"\n",
    "id-2,c-2,a-2,2026-04-12T08:00:00Z,Queja,another private text,Phone,true,false,false,60,10,\"line one\nline two\"\n",
    "id-3,c-3,a-3,2026-09-02T08:00:00Z,Queja,private,Phone,false,true,false,90,20,ignored\n",
);

#[test]
fn projection_binds_manifest_filters_after_cutoff_and_marks_sample_partial() {
    let bytes = CONTACTS.as_bytes();
    let plan = plan(
        ProjectionTable::Contacts,
        &[('p', bytes)],
        ProjectionCoverage::Partial,
        1,
    );
    let projection = project_contacts(&plan, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap();

    assert_eq!(projection.status, SupportStatus::Supported);
    assert_eq!(
        projection.source_snapshot_ref.id,
        "018f0f4e-7bbd-7000-8000-000000000501"
    );
    assert_eq!(projection.observed_cutoff, "2026-09-01T00:00:00Z");
    assert_eq!(projection.coverage, ProjectionCoverage::Partial);
    assert_eq!(projection.policy_version, 1);
    assert_eq!(
        projection
            .aggregates
            .iter()
            .map(|cell| cell.contact_count)
            .sum::<u64>(),
        2
    );
    assert_eq!(projection.suppressed_count, 0);
    assert!(!format!("{projection:?}").contains("private"));
    assert!(!format!("{projection:?}").contains("id-1"));
}

#[test]
fn projection_rejects_duplicate_headers_and_truncated_rows() {
    for bytes in [
        b"interaction_date,reason_category,channel,channel\n2026-01-01T00:00:00Z,Queja,Phone,Web\n"
            .as_slice(),
        b"interaction_date,reason_category,channel\n2026-01-01T00:00:00Z,Queja\n".as_slice(),
    ] {
        let manifest = plan(
            ProjectionTable::Contacts,
            &[('p', bytes)],
            ProjectionCoverage::Partial,
            1,
        );
        assert!(matches!(
            project_contacts(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]),
            Err(ProjectionError::DuplicateHeader | ProjectionError::MalformedCsv)
        ));
    }
}

#[test]
fn missing_channel_is_not_collapsed_into_other() {
    let bytes = b"interaction_date,reason_category,channel\n2026-01-01T00:00:00Z,Queja,\n";
    let manifest = plan(
        ProjectionTable::Contacts,
        &[('p', bytes)],
        ProjectionCoverage::Partial,
        1,
    );
    let projection =
        project_contacts(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap();
    assert!(projection.aggregates.is_empty());
}

#[test]
fn pii_like_category_is_projected_only_as_unclassified_enum() {
    let bytes = b"interaction_date,reason_category,channel\n2026-01-01T00:00:00Z,person@example.test,Phone\n";
    let manifest = plan(
        ProjectionTable::Contacts,
        &[('p', bytes)],
        ProjectionCoverage::Partial,
        1,
    );
    let projection =
        project_contacts(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap();
    assert_eq!(
        projection.aggregates[0].category,
        ContactCategory::Unclassified
    );
    assert!(!format!("{projection:?}").contains("person@example.test"));
}

#[test]
fn per_metric_denominators_retain_missing_values_and_k_policy_suppresses_small_cells() {
    let bytes = b"interaction_date,reason_category,channel,was_resolved,requires_followup,was_escalated,duration_seconds,wait_time_seconds\n2026-01-01T00:00:00Z,Queja,Phone,true,false,false,20,5\n2026-01-02T00:00:00Z,Queja,Phone,false,true,false,30,\n2026-01-03T00:00:00Z,Queja,Phone,true,false,false,40,7\n2026-01-04T00:00:00Z,Queja,Phone,false,false,true,,9\n2026-01-05T00:00:00Z,Queja,Phone,true,false,false,10,10\n2026-01-06T00:00:00Z,Producto,Email,true,false,false,50,10\n";
    let manifest = plan(
        ProjectionTable::Contacts,
        &[('p', bytes)],
        ProjectionCoverage::Partial,
        5,
    );
    let projection =
        project_contacts(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap();
    assert_eq!(projection.suppressed_count, 1);
    assert_eq!(projection.aggregates.len(), 1);
    let cell = &projection.aggregates[0];
    assert_eq!(cell.contact_count, 5);
    assert_eq!(
        (
            cell.duration_seconds.valid_count,
            cell.duration_seconds.missing_count
        ),
        (4, 1)
    );
    assert_eq!(
        (
            cell.wait_time_seconds.valid_count,
            cell.wait_time_seconds.missing_count
        ),
        (4, 1)
    );
    assert_eq!(
        (
            cell.resolved.valid_count,
            cell.resolved.missing_count,
            cell.resolved.positive_count
        ),
        (5, 0, 3)
    );
    assert_eq!(
        (cell.followup.valid_count, cell.followup.missing_count),
        (5, 0)
    );
}

#[test]
fn complaint_metrics_keep_known_denominators_and_elapsed_calendar_time() {
    let bytes = b"creation_date,category,reception_channel,sla_breached,first_response_date,resolution_days,resolution_satisfaction\n2026-04-01T00:00:00Z,Queja,Phone,true,2026-04-03T00:00:00Z,2,5\n2026-04-02T00:00:00Z,Queja,Phone,false,2026-04-02T00:00:00Z,0,4\n2026-04-03T00:00:00Z,Queja,Phone,,2026-04-05T00:00:00Z,,\n2026-04-04T00:00:00Z,Queja,Phone,true,2026-04-04T00:00:00Z,1,3\n2026-04-05T00:00:00Z,Queja,Phone,false,2026-04-06T00:00:00Z,1,5\n";
    let manifest = plan(
        ProjectionTable::Complaints,
        &[('p', bytes)],
        ProjectionCoverage::Partial,
        5,
    );
    let projection =
        project_complaints(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap();
    assert_eq!(projection.aggregates.len(), 1);
    let cell = &projection.aggregates[0];
    assert_eq!(cell.complaint_count, 5);
    assert_eq!(
        (
            cell.final_sla_breached.valid_count,
            cell.final_sla_breached.missing_count,
            cell.final_sla_breached.positive_count
        ),
        (4, 1, 2)
    );
    assert_eq!(
        (
            cell.final_first_response_elapsed_days.valid_count,
            cell.final_first_response_elapsed_days.missing_count
        ),
        (5, 0)
    );
    assert_eq!(cell.final_first_response_elapsed_days.mean, Some(1.0));
    assert_eq!(
        (
            cell.final_resolution_days.valid_count,
            cell.final_resolution_days.missing_count
        ),
        (4, 1)
    );
    assert_eq!(
        (
            cell.final_resolution_satisfaction.valid_count,
            cell.final_resolution_satisfaction.missing_count
        ),
        (4, 1)
    );
}

#[test]
fn complaint_outcomes_after_cutoff_are_marked_as_final_creation_cohort_metrics() {
    let bytes = b"creation_date,category,reception_channel,sla_breached,first_response_date,resolution_date,closing_date,resolution_days,resolution_satisfaction\n2026-08-31T09:00:00Z,Queja,Phone,true,2026-09-05T09:00:00Z,2026-09-10T09:00:00Z,2026-09-12T09:00:00Z,10,2\n";
    let manifest = plan(
        ProjectionTable::Complaints,
        &[('p', bytes)],
        ProjectionCoverage::Partial,
        1,
    );
    let projection =
        project_complaints(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap();

    assert_eq!(
        projection.temporal_semantics,
        ProjectionTemporalSemantics::CreationCohortWithFinalOutcomes
    );
    assert_eq!(projection.aggregates[0].final_sla_breached.valid_count, 1);
    assert_eq!(
        projection.aggregates[0]
            .final_first_response_elapsed_days
            .mean,
        Some(5.0)
    );
    assert_eq!(
        projection.aggregates[0].final_resolution_days.mean,
        Some(10.0)
    );
    assert_eq!(
        projection.aggregates[0].final_resolution_satisfaction.mean,
        Some(2.0)
    );
}

#[test]
fn partition_inventory_is_exact_and_content_is_hashed() {
    let bytes = b"interaction_date,reason_category,channel\n2026-04-03T00:00:00Z,Queja,Phone\n";
    let manifest = plan(
        ProjectionTable::Contacts,
        &[('p', bytes)],
        ProjectionCoverage::Complete,
        1,
    );
    let wrong = b"interaction_date,reason_category,channel\n2026-04-03T00:00:00Z,Queja,Web\n";
    assert_eq!(
        project_contacts(&manifest, [CsvPartition::new("p", Cursor::new(wrong))]).unwrap_err(),
        ProjectionError::DigestMismatch
    );
    assert_eq!(
        project_contacts(&manifest, std::iter::empty::<CsvPartition<Cursor<&[u8]>>>()).unwrap_err(),
        ProjectionError::PartitionSetMismatch
    );
    assert_eq!(
        project_contacts(
            &manifest,
            [
                CsvPartition::new("p", Cursor::new(bytes)),
                CsvPartition::new("unexpected", Cursor::new(bytes)),
            ]
        )
        .unwrap_err(),
        ProjectionError::PartitionSetMismatch
    );
}

#[test]
fn projection_manifest_rejects_an_unrelated_source_reference() {
    let bytes = b"interaction_date,reason_category,channel\n2026-04-03T00:00:00Z,Queja,Phone\n";
    let inventory = vec![ManifestPartition::new("p".into(), digest(bytes))];
    let (mut repository, source_ref) = stored_snapshot(
        ProjectionTable::Contacts,
        "2026-09-01T00:00:00Z",
        &inventory,
        bytes,
    );
    let unrelated = ArtifactReference {
        id: "018f0f4e-7bbd-7000-8000-000000000777".into(),
        ..source_ref
    };
    assert!(
        ProjectionManifest::new(
            &mut repository,
            unrelated,
            ProjectionTable::Contacts,
            inventory,
            ProjectionCoverage::Partial,
            ProjectionPolicy {
                version: 1,
                minimum_cell_count: 1
            },
        )
        .is_err()
    );
}

#[test]
fn projection_manifest_rejects_partitions_not_bound_by_source_table_seal() {
    let bytes = b"interaction_date,reason_category,channel\n2026-04-03T00:00:00Z,Queja,Phone\n";
    let sealed_inventory = vec![ManifestPartition::new("p".into(), digest(bytes))];
    let (mut repository, source_ref) = stored_snapshot(
        ProjectionTable::Contacts,
        "2026-09-01T00:00:00Z",
        &sealed_inventory,
        bytes,
    );
    let different_partition =
        b"interaction_date,reason_category,channel\n2026-04-03T00:00:00Z,Queja,Web\n";
    assert!(
        ProjectionManifest::new(
            &mut repository,
            source_ref,
            ProjectionTable::Contacts,
            vec![ManifestPartition::new(
                "p".into(),
                digest(different_partition)
            )],
            ProjectionCoverage::Partial,
            ProjectionPolicy {
                version: 1,
                minimum_cell_count: 1
            },
        )
        .is_err()
    );
}

#[test]
fn projection_manifest_rejects_partitioned_source_without_explicit_inventory_seal() {
    let bytes = b"interaction_date,reason_category,channel\n2026-04-03T00:00:00Z,Queja,Phone\n";
    let inventory = vec![ManifestPartition::new("p".into(), digest(bytes))];
    let (mut repository, source_ref) = stored_snapshot_with_inventory(
        ProjectionTable::Contacts,
        "2026-09-01T00:00:00Z",
        &inventory,
        bytes,
        false,
    );
    assert!(
        ProjectionManifest::new(
            &mut repository,
            source_ref,
            ProjectionTable::Contacts,
            inventory,
            ProjectionCoverage::Partial,
            ProjectionPolicy {
                version: 1,
                minimum_cell_count: 1
            },
        )
        .is_err()
    );
}

#[test]
fn source_file_digest_and_partition_inventory_are_verified_as_distinct_seals() {
    let bytes = include_bytes!(
        "../../../contracts/fixtures/sources-v1/validation/call_center_interactions.csv"
    );
    let mut snapshot_json = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../../contracts/fixtures/sources-v1/validation/source-snapshot.json"
    ))
    .unwrap();
    let inventory = vec![ManifestPartition::new(
        "validation-partition".into(),
        digest(bytes),
    )];
    snapshot_json["sources"][0]["partition_inventory_digest"] =
        json!(canonical_partition_inventory_digest(&inventory).unwrap());
    let raw_snapshot = serde_json::to_string(&snapshot_json).unwrap();
    let snapshot = SourceSnapshot::from_json(&raw_snapshot).unwrap();
    let contract = SourceContract::from_json(include_str!(
        "../../../contracts/sources/call_center_interactions.v1.json"
    ))
    .unwrap();
    assert!(
        validate_source_file(&contract, &snapshot, bytes)
            .unwrap()
            .findings
            .is_empty()
    );

    let mut repository = InMemoryArtifactRepository::default();
    let source = ArtifactDraft::new(
        "demo",
        "018f0f4e-7bbd-7000-8000-000000000601",
        1,
        ArtifactKind::SourceSnapshot,
        json!({"source_snapshot_json": raw_snapshot}),
        None,
    );
    let stored = repository.append(None, source).unwrap();
    let manifest = ProjectionManifest::new(
        &mut repository,
        stored.reference(),
        ProjectionTable::Contacts,
        inventory,
        ProjectionCoverage::Partial,
        ProjectionPolicy {
            version: 1,
            minimum_cell_count: 1,
        },
    )
    .unwrap();
    let projection = project_contacts(
        &manifest,
        [CsvPartition::new(
            "validation-partition",
            Cursor::new(bytes),
        )],
    )
    .unwrap();
    assert_eq!(projection.status, SupportStatus::Supported);
    assert_eq!(
        projection
            .aggregates
            .iter()
            .map(|cell| cell.contact_count)
            .sum::<u64>(),
        1
    );
}

#[test]
fn projection_rejects_csv_header_not_committed_by_source_table_seal() {
    let sealed_header = b"interaction_date,reason_category,channel\n";
    let bytes =
        b"interaction_date,reason_category,channel,extra\n2026-04-03T00:00:00Z,Queja,Phone,x\n";
    let inventory = vec![ManifestPartition::new("p".into(), digest(bytes))];
    let (mut repository, source_ref) = stored_snapshot(
        ProjectionTable::Contacts,
        "2026-09-01T00:00:00Z",
        &inventory,
        sealed_header,
    );
    let manifest = ProjectionManifest::new(
        &mut repository,
        source_ref,
        ProjectionTable::Contacts,
        inventory,
        ProjectionCoverage::Partial,
        ProjectionPolicy {
            version: 1,
            minimum_cell_count: 1,
        },
    )
    .unwrap();
    assert_eq!(
        project_contacts(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap_err(),
        ProjectionError::HeaderDigestMismatch,
    );
}

#[test]
fn projection_preserves_second_precision_at_cutoff() {
    let bytes = b"interaction_date,reason_category,channel\n2026-09-01T11:59:59Z,Queja,Phone\n2026-09-01T12:00:00Z,Queja,Phone\n2026-09-01T12:00:01Z,Queja,Phone\n";
    let inventory = vec![ManifestPartition::new("p".into(), digest(bytes))];
    let (mut repository, source_ref) = stored_snapshot(
        ProjectionTable::Contacts,
        "2026-09-01T12:00:00Z",
        &inventory,
        bytes,
    );
    let manifest = ProjectionManifest::new(
        &mut repository,
        source_ref,
        ProjectionTable::Contacts,
        inventory,
        ProjectionCoverage::Partial,
        ProjectionPolicy {
            version: 1,
            minimum_cell_count: 1,
        },
    )
    .unwrap();
    let projection =
        project_contacts(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap();
    assert_eq!(
        projection
            .aggregates
            .iter()
            .map(|cell| cell.contact_count)
            .sum::<u64>(),
        2
    );
}

#[test]
fn projection_manifest_rejects_cutoffs_without_supported_utc_second_semantics() {
    let bytes = b"interaction_date,reason_category,channel\n2026-09-01T11:59:59Z,Queja,Phone\n";
    let inventory = vec![ManifestPartition::new("p".into(), digest(bytes))];
    for cutoff in ["2026-09-01T12:00:00.500Z"] {
        let (mut repository, source_ref) =
            stored_snapshot(ProjectionTable::Contacts, cutoff, &inventory, bytes);
        assert!(
            ProjectionManifest::new(
                &mut repository,
                source_ref,
                ProjectionTable::Contacts,
                inventory.clone(),
                ProjectionCoverage::Partial,
                ProjectionPolicy {
                    version: 1,
                    minimum_cell_count: 1
                },
            )
            .is_err()
        );
    }
}

#[test]
fn contact_projection_counts_invalid_or_missing_dates_as_rejected_rows() {
    let bytes = b"interaction_date,reason_category,channel\nnot-a-date,Queja,Phone\n,Queja,Phone\n2026-04-03T00:00:00Zgarbage,Queja,Phone\n2026-04-03T00:00:00Z,Queja,Phone\n";
    let manifest = plan(
        ProjectionTable::Contacts,
        &[('p', bytes)],
        ProjectionCoverage::Partial,
        1,
    );
    let projection =
        project_contacts(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap();
    assert_eq!(projection.rejected_rows, 3);
    assert_eq!(
        projection
            .aggregates
            .iter()
            .map(|cell| cell.contact_count)
            .sum::<u64>(),
        1
    );
}

#[test]
fn naive_timestamp_is_unsupported_and_never_coerced_to_utc() {
    let bytes = b"interaction_date,reason_category,channel\n2026-04-03 12:00:00,Queja,Phone\n";
    let manifest = plan(
        ProjectionTable::Contacts,
        &[('p', bytes)],
        ProjectionCoverage::Partial,
        1,
    );
    let projection =
        project_contacts(&manifest, [CsvPartition::new("p", Cursor::new(bytes))]).unwrap();
    assert_eq!(projection.rejected_rows, 1);
    assert_eq!(
        projection.status,
        SupportStatus::Unsupported {
            missing_fields: vec!["timezone-qualified timestamp"]
        }
    );
    assert!(projection.aggregates.is_empty());
}

#[test]
#[ignore = "requires local private source files; emits aggregate counts only"]
fn local_original_contacts_and_complaints_smoke_aggregates_only() {
    let root = PathBuf::from(
        std::env::var_os("PULSO_ORIGINAL_DATA_ROOT").expect("set only for local smoke"),
    );
    for (subdir, table, projector) in [
        ("call_center_interactions", ProjectionTable::Contacts, true),
        ("complaints", ProjectionTable::Complaints, false),
    ] {
        let paths = files_below(&root.join(subdir))
            .into_iter()
            .take(25)
            .collect::<Vec<_>>();
        assert!(!paths.is_empty(), "expected sample partitions for {subdir}");
        let partition_count = paths.len();
        let inputs = paths
            .into_iter()
            .enumerate()
            .map(|(ordinal, path)| {
                let bytes = fs::read(&path).unwrap();
                (format!("partition-{ordinal:05}"), bytes)
            })
            .collect::<Vec<_>>();
        let inventory = inputs
            .iter()
            .map(|(id, bytes)| ManifestPartition::new(id.clone(), digest(bytes)))
            .collect::<Vec<_>>();
        let manifest = smoke_plan(table, &inventory, &inputs[0].1);
        let rows = inputs
            .into_iter()
            .map(|(id, bytes)| CsvPartition::new(id, Cursor::new(bytes)));
        let (status, rejected, cells) = if projector {
            let projection = project_contacts(&manifest, rows).unwrap();
            (
                projection.status,
                projection.rejected_rows,
                projection.aggregates.len(),
            )
        } else {
            let projection = project_complaints(&manifest, rows).unwrap();
            (
                projection.status,
                projection.rejected_rows,
                projection.aggregates.len(),
            )
        };
        assert_eq!(
            status,
            SupportStatus::Unsupported {
                missing_fields: vec!["timezone-qualified timestamp"]
            },
            "source timestamp semantics unexpectedly became supported in {subdir}"
        );
        assert_eq!(cells, 0, "unsupported source emitted aggregate cells");
        assert!(rejected > 0, "unsupported timestamp rows were not counted");
        println!(
            "{subdir}: partitions={partition_count}, rejected_rows={rejected}, aggregate_cells={cells}, coverage=partial"
        );
    }
}

fn files_below(directory: &Path) -> Vec<PathBuf> {
    let mut pending = vec![directory.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "csv") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn smoke_plan(
    table: ProjectionTable,
    partitions: &[ManifestPartition],
    header_source: &[u8],
) -> ProjectionManifest {
    let (mut repository, source_ref) =
        stored_snapshot(table, "2026-09-01T00:00:00Z", partitions, header_source);
    ProjectionManifest::new(
        &mut repository,
        source_ref,
        table,
        partitions.to_vec(),
        ProjectionCoverage::Partial,
        ProjectionPolicy {
            version: 1,
            minimum_cell_count: 5,
        },
    )
    .unwrap()
}

fn plan(
    table: ProjectionTable,
    partitions: &[(char, &[u8])],
    coverage: ProjectionCoverage,
    minimum_cell_count: u64,
) -> ProjectionManifest {
    let header_source = partitions
        .first()
        .map(|(_, bytes)| *bytes)
        .expect("fixture has a partition");
    let inventory = partitions
        .iter()
        .map(|(id, bytes)| ManifestPartition::new(id.to_string(), digest(bytes)))
        .collect::<Vec<_>>();
    let (mut repository, source_ref) =
        stored_snapshot(table, "2026-09-01T00:00:00Z", &inventory, header_source);
    ProjectionManifest::new(
        &mut repository,
        source_ref,
        table,
        inventory,
        coverage,
        ProjectionPolicy {
            version: 1,
            minimum_cell_count,
        },
    )
    .unwrap()
}

fn stored_snapshot(
    table: ProjectionTable,
    cutoff: &str,
    partitions: &[ManifestPartition],
    header_source: &[u8],
) -> (InMemoryArtifactRepository, ArtifactReference) {
    stored_snapshot_with_inventory(table, cutoff, partitions, header_source, true)
}

fn stored_snapshot_with_inventory(
    table: ProjectionTable,
    cutoff: &str,
    partitions: &[ManifestPartition],
    header_source: &[u8],
    include_inventory_seal: bool,
) -> (InMemoryArtifactRepository, ArtifactReference) {
    let table_name = table.as_str();
    let inventory_digest = canonical_partition_inventory_digest(partitions).unwrap();
    let header_digest = header_digest(header_source);
    let partition_inventory_field = if include_inventory_seal {
        format!(r#","partition_inventory_digest":"{}""#, inventory_digest)
    } else {
        String::new()
    };
    let raw_snapshot = format!(
        r#"{{"contract_version":{{"major":1,"minor":0}},"tenant_id":"tenant-a","source_namespace":"bank_history","world_ref":"original-v1","observed_cutoff":"{cutoff}","sources":[{{"table":"{table_name}","uri":"file:///synthetic/{table_name}/first-partition.csv","file_digest":"{}","header_digest":"sha256:{}"{partition_inventory_field},"row_count":0,"source_contract_ref":{{"id":"{table_name}","version":"v1","digest":"sha256:{}"}}}}]}}"#,
        partitions.first().unwrap().sha256(),
        header_digest.strip_prefix("sha256:").unwrap(),
        "2".repeat(64),
    );
    SourceSnapshot::from_json(&raw_snapshot).unwrap();
    let mut repository = InMemoryArtifactRepository::default();
    let source = ArtifactDraft::new(
        "tenant-a",
        "018f0f4e-7bbd-7000-8000-000000000501",
        1,
        ArtifactKind::SourceSnapshot,
        json!({"source_snapshot_json": raw_snapshot}),
        None,
    );
    let stored = repository.append(None, source).unwrap();
    (repository, stored.reference())
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn header_digest(bytes: &[u8]) -> String {
    let header = bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let header = header.strip_suffix(b"\r").unwrap_or(header);
    digest(header)
}
