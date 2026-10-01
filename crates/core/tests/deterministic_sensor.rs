use std::collections::BTreeMap;

use improvement_engine_core::ArtifactReference;
use improvement_engine_core::deterministic_sensor::{
    BooleanRateSpec, DeterministicSensor, SensorError,
};
use improvement_engine_core::local_lab::{
    InMemoryLabGrantAuthority, InMemoryLabSourceAuthority, LabAccess, LabDataClassification,
    LabGrant, LabQuery, LabSource, LabSourceApprovalPort, LabSourceManifest, LabTable,
    LocalInvestigationLab,
};

fn digest(seed: char) -> String {
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn reference() -> ArtifactReference {
    ArtifactReference {
        tenant_id: "bank_demo".to_owned(),
        id: "018f3a54-7eaf-7c83-8a04-5bf4ec1a9d26".to_owned(),
        revision: 1,
        digest: digest('a'),
    }
}

fn access() -> LabAccess {
    LabAccess::new(
        "run-1",
        "bank_demo",
        "investigation",
        "grant-1",
        reference(),
        200,
    )
}

fn row(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn result() -> improvement_engine_core::local_lab::QueryResult {
    let access = access();
    let mut authority = InMemoryLabGrantAuthority::default();
    authority.issue(LabGrant::from_access(&access));
    let mut lab = LocalInvestigationLab::new(authority);
    let source = LabSource::new(
        LabSourceManifest {
            tenant_id: "bank_demo".to_owned(),
            snapshot_ref: reference(),
            source_contract_digest: digest('b'),
            source_digest: digest('c'),
            transform_digest: digest('d'),
            cutoff_unix_seconds: 100,
            classification: LabDataClassification::Treated,
            safe_for_discovery: true,
        },
        vec![LabTable::new(
            "contacts",
            vec!["resolved"],
            vec![
                row(&[("resolved", "true")]),
                row(&[("resolved", "false")]),
                row(&[("resolved", "")]),
            ],
        )],
    )
    .unwrap();
    let mut source_authority = InMemoryLabSourceAuthority;
    let approved = source_authority.approve(source).unwrap();
    let session = lab.open(access.clone(), approved, 100).unwrap();
    lab.query(
        session.session_id(),
        &access,
        LabQuery::select("contacts", vec!["resolved"], None),
        101,
    )
    .unwrap()
}

#[test]
fn produces_reproducible_rate_with_numerator_denominator_missingness_and_sealed_receipt() {
    let result = result();
    let spec = BooleanRateSpec::new("contact_resolution", "resolved", "true").unwrap();
    let first = DeterministicSensor::measure(&spec, std::slice::from_ref(&result)).unwrap();
    let second = DeterministicSensor::measure(&spec, std::slice::from_ref(&result)).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.numerator, 1);
    assert_eq!(first.denominator, 2);
    assert_eq!(first.missing, 1);
    assert_eq!(first.coverage_basis_points, 6_666);
    assert_eq!(first.query_receipts.len(), 1);
    assert!(!first.digest.is_empty());
}

#[test]
fn missing_or_unknown_values_are_not_coerced_into_a_denominator_or_numerator() {
    let result = result();
    let spec = BooleanRateSpec::new("contact_resolution", "resolved", "true").unwrap();
    let signal = DeterministicSensor::measure(&spec, &[result]).unwrap();
    assert_eq!(signal.numerator + 1, signal.denominator);
    assert_eq!(signal.missing, 1);
}

#[test]
fn rejects_tampered_or_source_mismatched_receipts_before_calculating_a_signal() {
    let result = result();
    let spec = BooleanRateSpec::new("contact_resolution", "resolved", "true").unwrap();
    let (rows, mut receipt) = result.clone().into_parts();
    receipt.source_digest = digest('f');
    let tampered = improvement_engine_core::local_lab::QueryResult::untrusted(rows, receipt);
    assert_eq!(
        DeterministicSensor::measure(&spec, &[tampered]),
        Err(SensorError::ReceiptIntegrityDenied)
    );

    let (rows, mut receipt) = result.clone().into_parts();
    receipt.source_snapshot_ref.digest = digest('e');
    let mismatched = improvement_engine_core::local_lab::QueryResult::untrusted(rows, receipt);
    assert_eq!(
        DeterministicSensor::measure(&spec, &[result, mismatched]),
        Err(SensorError::ReceiptIntegrityDenied)
    );
}

#[test]
fn receipt_binds_canonical_rows_and_a_sensor_rejects_duplicate_or_cross_session_evidence() {
    let result = result();
    let spec = BooleanRateSpec::new("contact_resolution", "resolved", "true").unwrap();
    let (mut rows, receipt) = result.clone().into_parts();
    rows[0].insert("resolved".to_owned(), "false".to_owned());
    assert_eq!(
        DeterministicSensor::measure(
            &spec,
            &[improvement_engine_core::local_lab::QueryResult::untrusted(
                rows, receipt
            )]
        ),
        Err(SensorError::ReceiptIntegrityDenied)
    );
    assert_eq!(
        DeterministicSensor::measure(&spec, &[result.clone(), result]),
        Err(SensorError::ReceiptIntegrityDenied)
    );
}

#[test]
fn metric_field_must_be_present_in_authorized_query_rows() {
    let result = result();
    let spec = BooleanRateSpec::new("contact_resolution", "not_present", "true").unwrap();
    assert_eq!(
        DeterministicSensor::measure(&spec, &[result]),
        Err(SensorError::MetricFieldMissing)
    );
}
