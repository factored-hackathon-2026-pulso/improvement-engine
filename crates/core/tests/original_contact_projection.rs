use improvement_engine_core::original_contact_projection::{
    ContactCategory, SupportStatus, project_complaint_csvs, project_contact_csvs,
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

#[test]
fn contacts_project_only_allowlisted_dimensions_and_aggregate_metrics() {
    let csv = concat!(
        "interaction_id,customer_id,agent_id,interaction_date,reason_category,contact_reason,channel,was_resolved,requires_followup,was_escalated,duration_seconds,wait_time_seconds,description\n",
        "id-1,c-1,a-1,2026-04-03,Queja,private free text,Phone,false,true,true,120,30,do not read\n",
        "id-2,c-2,a-2,2026-04-12,Queja,another private text,Phone,true,false,false,60,10,never project\n",
        "id-3,c-3,a-3,2026-05-01,personal@example.com,leaked string,Phone,false,true,false,90,20,ignored\n",
    );

    let projection = project_contact_csvs([Cursor::new(csv.as_bytes())]);

    assert_eq!(projection.status, SupportStatus::Supported);
    assert_eq!(projection.rows_seen, 3);
    assert_eq!(projection.aggregates.len(), 2);
    let april = projection
        .aggregates
        .iter()
        .find(|row| row.period == "2026-04")
        .unwrap();
    assert_eq!(april.category, ContactCategory::Complaint);
    assert_eq!(april.channel.as_str(), "phone");
    assert_eq!(april.contact_count, 2);
    assert_eq!(april.resolved_yes, 1);
    assert_eq!(april.followup_yes, 1);
    assert_eq!(april.escalated_yes, 1);
    assert_eq!(april.duration_mean_seconds, Some(90.0));
    assert_eq!(april.wait_mean_seconds, Some(20.0));
    let may = projection
        .aggregates
        .iter()
        .find(|row| row.period == "2026-05")
        .unwrap();
    assert_eq!(may.category.as_str(), "unclassified");
    assert!(!format!("{projection:?}").contains("private free text"));
    assert!(!format!("{projection:?}").contains("personal@example.com"));
    assert!(!format!("{projection:?}").contains("id-1"));
}

#[test]
fn contacts_report_unsupported_when_required_partition_dimensions_are_absent() {
    let csv = "interaction_id,was_resolved\nid-1,true\n";
    let projection = project_contact_csvs([Cursor::new(csv.as_bytes())]);

    assert!(matches!(
        projection.status,
        SupportStatus::Unsupported { .. }
    ));
    assert!(projection.aggregates.is_empty());
}

#[test]
fn complaints_aggregate_sla_resolution_and_satisfaction_without_identifiers_or_text() {
    let csv = concat!(
        "complaint_id,customer_id,creation_date,first_response_date,category,subcategory,reception_channel,sla_breached,resolution_days,resolution_satisfaction,description\n",
        "pqr-1,c-1,2026-06-02,2026-06-04,Queja,private subreason,Web,true,4,2,private details\n",
        "pqr-2,c-2,2026-06-17,2026-06-17,Queja,other private detail,Web,false,2,4,never project\n",
    );

    let projection = project_complaint_csvs([Cursor::new(csv.as_bytes())]);

    assert_eq!(projection.status, SupportStatus::Supported);
    assert_eq!(projection.rows_seen, 2);
    assert_eq!(projection.aggregates.len(), 1);
    let row = &projection.aggregates[0];
    assert_eq!(row.period, "2026-06");
    assert_eq!(row.channel.as_str(), "web");
    assert_eq!(row.complaint_count, 2);
    assert_eq!(row.sla_breached_yes, 1);
    assert_eq!(row.first_response_mean_days, Some(1.0));
    assert_eq!(row.resolution_days_mean, Some(3.0));
    assert_eq!(row.satisfaction_mean, Some(3.0));
    let rendered = format!("{projection:?}");
    for forbidden in ["pqr-1", "c-1", "private subreason", "private details"] {
        assert!(!rendered.contains(forbidden));
    }
}

#[test]
fn complaints_with_missing_sla_field_are_unsupported_not_zero_sla() {
    let csv = "complaint_id,creation_date,category,reception_channel\npqr-1,2026-06-02,Queja,Web\n";
    let projection = project_complaint_csvs([Cursor::new(csv.as_bytes())]);

    assert!(matches!(
        projection.status,
        SupportStatus::Unsupported { .. }
    ));
    assert!(projection.aggregates.is_empty());
}

#[test]
fn metrics_are_available_only_when_every_partition_has_the_field() {
    let first = "interaction_date,reason_category,channel,was_resolved,duration_seconds\n2026-01-01,Queja,Phone,true,10\n";
    let second =
        "interaction_date,reason_category,channel,was_resolved\n2026-01-02,Queja,Phone,false\n";
    let projection = project_contact_csvs([
        Cursor::new(first.as_bytes()),
        Cursor::new(second.as_bytes()),
    ]);

    assert_eq!(projection.status, SupportStatus::Supported);
    assert_eq!(projection.available_metrics, vec!["was_resolved"]);
    assert!(projection.missing_metrics.contains(&"duration_seconds"));
    assert_eq!(projection.aggregates.len(), 1);
    assert_eq!(projection.aggregates[0].contact_count, 2);
    assert_eq!(projection.aggregates[0].duration_mean_seconds, Some(10.0));
}

#[test]
#[ignore = "requires local private source files; emits no row-level output"]
fn local_original_contacts_and_complaints_smoke_aggregates_only() {
    let root = PathBuf::from(
        std::env::var_os("PULSO_ORIGINAL_DATA_ROOT").expect("set only for local smoke"),
    );
    let contacts = files_below(&root.join("call_center_interactions"))
        .into_iter()
        .take(25)
        .collect::<Vec<_>>();
    let complaints = files_below(&root.join("complaints"))
        .into_iter()
        .take(25)
        .collect::<Vec<_>>();
    assert!(!contacts.is_empty());
    assert!(!complaints.is_empty());
    let contact_projection = project_contact_csvs(
        contacts
            .into_iter()
            .map(|path| fs::File::open(path).unwrap()),
    );
    let complaint_projection = project_complaint_csvs(
        complaints
            .into_iter()
            .map(|path| fs::File::open(path).unwrap()),
    );
    assert_eq!(contact_projection.status, SupportStatus::Supported);
    assert_eq!(complaint_projection.status, SupportStatus::Supported);
    assert!(contact_projection.rows_seen > 0 && !contact_projection.aggregates.is_empty());
    assert!(complaint_projection.rows_seen > 0 && !complaint_projection.aggregates.is_empty());
    println!(
        "contacts: input_records={}, rejected_records={}, aggregate_cells={}; complaints: input_records={}, rejected_records={}, aggregate_cells={}",
        contact_projection.rows_seen,
        contact_projection.rows_rejected,
        contact_projection.aggregates.len(),
        complaint_projection.rows_seen,
        complaint_projection.rows_rejected,
        complaint_projection.aggregates.len(),
    );
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
