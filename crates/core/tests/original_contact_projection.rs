use improvement_engine_core::source_validation::SourceSnapshot;
use improvement_engine_core::{ArtifactReference, original_contact_projection::*};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

const CONTACTS: &str = concat!(
    "interaction_id,customer_id,agent_id,interaction_date,reason_category,contact_reason,channel,was_resolved,requires_followup,was_escalated,duration_seconds,wait_time_seconds,description\n",
    "id-1,c-1,a-1,2026-04-03,Queja,private free text,Phone,false,true,true,120,30,\"quoted, \"\"private\"\"\"\n",
    "id-2,c-2,a-2,2026-04-12,Queja,another private text,Phone,true,false,false,60,10,\"line one\nline two\"\n",
    "id-3,c-3,a-3,2026-09-02,Queja,private,Phone,false,true,false,90,20,ignored\n",
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
        b"interaction_date,reason_category,channel,channel\n2026-01-01,Queja,Phone,Web\n"
            .as_slice(),
        b"interaction_date,reason_category,channel\n2026-01-01,Queja\n".as_slice(),
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
    let bytes = b"interaction_date,reason_category,channel\n2026-01-01,Queja,\n";
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
    let bytes = b"interaction_date,reason_category,channel\n2026-01-01,person@example.test,Phone\n";
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
    let bytes = b"interaction_date,reason_category,channel,was_resolved,requires_followup,was_escalated,duration_seconds,wait_time_seconds\n2026-01-01,Queja,Phone,true,false,false,20,5\n2026-01-02,Queja,Phone,false,true,false,30,\n2026-01-03,Queja,Phone,true,false,false,40,7\n2026-01-04,Queja,Phone,false,false,true,,9\n2026-01-05,Queja,Phone,true,false,false,10,10\n2026-01-06,Producto,Email,true,false,false,50,10\n";
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
    let bytes = b"creation_date,category,reception_channel,sla_breached,first_response_date,resolution_days,resolution_satisfaction\n2026-04-01,Queja,Phone,true,2026-04-03,2,5\n2026-04-02,Queja,Phone,false,2026-04-02,0,4\n2026-04-03,Queja,Phone,,2026-04-05,,\n2026-04-04,Queja,Phone,true,2026-04-04,1,3\n2026-04-05,Queja,Phone,false,2026-04-06,1,5\n";
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
            cell.sla_breached.valid_count,
            cell.sla_breached.missing_count,
            cell.sla_breached.positive_count
        ),
        (4, 1, 2)
    );
    assert_eq!(
        (
            cell.first_response_calendar_days.valid_count,
            cell.first_response_calendar_days.missing_count
        ),
        (5, 0)
    );
    assert_eq!(cell.first_response_calendar_days.mean, Some(1.0));
    assert_eq!(
        (
            cell.resolution_days.valid_count,
            cell.resolution_days.missing_count
        ),
        (4, 1)
    );
    assert_eq!(
        (
            cell.resolution_satisfaction.valid_count,
            cell.resolution_satisfaction.missing_count
        ),
        (4, 1)
    );
}

#[test]
fn partition_inventory_is_exact_and_content_is_hashed() {
    let bytes = b"interaction_date,reason_category,channel\n2026-04-03,Queja,Phone\n";
    let manifest = plan(
        ProjectionTable::Contacts,
        &[('p', bytes)],
        ProjectionCoverage::Complete,
        1,
    );
    let wrong = b"interaction_date,reason_category,channel\n2026-04-03,Queja,Web\n";
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
            .collect();
        let manifest = smoke_plan(table, inventory);
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
            SupportStatus::Supported,
            "unsupported schema in {subdir}"
        );
        assert!(cells > 0, "empty projection for {subdir}");
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

fn smoke_plan(table: ProjectionTable, partitions: Vec<ManifestPartition>) -> ProjectionManifest {
    let table_name = table.as_str();
    let source = SourceSnapshot::from_json(&format!(
        r#"{{"contract_version":{{"major":1,"minor":0}},"tenant_id":"tenant-a","source_namespace":"bank_history","world_ref":"original-v1","observed_cutoff":"2026-09-01T00:00:00Z","sources":[{{"table":"{table_name}","uri":"file:///synthetic/{table_name}.csv","file_digest":"sha256:{}","header_digest":"sha256:{}","row_count":0,"source_contract_ref":{{"id":"{table_name}","version":"v1","digest":"sha256:{}"}}}}]}}"#,
        "0".repeat(64), "1".repeat(64), "2".repeat(64),
    )).unwrap();
    ProjectionManifest::new(
        reference(),
        &source,
        table,
        partitions,
        ProjectionCoverage::Partial,
        ProjectionPolicy {
            version: 1,
            minimum_cell_count: 5,
        },
    )
    .unwrap()
}

fn reference() -> ArtifactReference {
    ArtifactReference {
        tenant_id: "tenant-a".into(),
        id: "018f0f4e-7bbd-7000-8000-000000000501".into(),
        revision: 1,
        digest: format!("sha256:{}", "a".repeat(64)),
    }
}

fn plan(
    table: ProjectionTable,
    partitions: &[(char, &[u8])],
    coverage: ProjectionCoverage,
    minimum_cell_count: u64,
) -> ProjectionManifest {
    let table_name = table.as_str();
    let source = SourceSnapshot::from_json(&format!(
        r#"{{"contract_version":{{"major":1,"minor":0}},"tenant_id":"tenant-a","source_namespace":"bank_history","world_ref":"original-v1","observed_cutoff":"2026-09-01T00:00:00Z","sources":[{{"table":"{table_name}","uri":"file:///synthetic/{table_name}.csv","file_digest":"sha256:{}","header_digest":"sha256:{}","row_count":0,"source_contract_ref":{{"id":"{table_name}","version":"v1","digest":"sha256:{}"}}}}]}}"#,
        "0".repeat(64), "1".repeat(64), "2".repeat(64),
    )).unwrap();
    let partitions = partitions
        .iter()
        .map(|(id, bytes)| ManifestPartition::new(id.to_string(), digest(bytes)))
        .collect();
    ProjectionManifest::new(
        ArtifactReference {
            tenant_id: "tenant-a".into(),
            id: "018f0f4e-7bbd-7000-8000-000000000501".into(),
            revision: 1,
            digest: format!("sha256:{}", "a".repeat(64)),
        },
        &source,
        table,
        partitions,
        coverage,
        ProjectionPolicy {
            version: 1,
            minimum_cell_count,
        },
    )
    .unwrap()
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(bytes))
}
