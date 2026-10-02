//! Deterministic diagnostic rates over authenticated U08-E receipts.
//!
//! This is deliberately narrower than the older generic sensor: it accepts
//! only an opaque `VerifiedE0QueryResult`, carries U04's replay commitments
//! into its immutable output, and cannot infer labels/outcomes, access data,
//! call a model, or authorize a write/release.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::ArtifactReference;
use crate::e0_query_lab::VerifiedE0QueryResult;

/// A declared boolean diagnostic over one E0-projected field and its event
/// clock. It names neither a business outcome nor a causal explanation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0BooleanRateSpec {
    metric_id: String,
    field: String,
    event_time_field: String,
    positive_value: String,
}

impl E0BooleanRateSpec {
    pub fn new(
        metric_id: impl Into<String>,
        field: impl Into<String>,
        event_time_field: impl Into<String>,
        positive_value: impl Into<String>,
    ) -> Result<Self, E0DiagnosticSensorError> {
        let spec = Self {
            metric_id: metric_id.into(),
            field: field.into(),
            event_time_field: event_time_field.into(),
            positive_value: positive_value.into(),
        };
        if !valid_identifier(&spec.metric_id)
            || !valid_identifier(&spec.field)
            || !valid_identifier(&spec.event_time_field)
            || spec.field == spec.event_time_field
            || !matches!(spec.positive_value.as_str(), "true" | "false")
        {
            return Err(E0DiagnosticSensorError::InvalidSpec);
        }
        Ok(spec)
    }

    #[must_use]
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }
}

/// Inclusive virtual observation window. It must end at or before U04's
/// sealed replay cutoff; the sensor verifies every measured event timestamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct E0DiagnosticWindow {
    start_unix_seconds: u64,
    end_unix_seconds: u64,
}

impl E0DiagnosticWindow {
    pub fn new(
        start_unix_seconds: u64,
        end_unix_seconds: u64,
    ) -> Result<Self, E0DiagnosticSensorError> {
        if start_unix_seconds > end_unix_seconds {
            return Err(E0DiagnosticSensorError::InvalidWindow);
        }
        Ok(Self {
            start_unix_seconds,
            end_unix_seconds,
        })
    }

    #[must_use]
    pub fn start_unix_seconds(&self) -> u64 {
        self.start_unix_seconds
    }

    #[must_use]
    pub fn end_unix_seconds(&self) -> u64 {
        self.end_unix_seconds
    }
}

/// Immutable, descriptive signal. It reports what the supplied sealed E0
/// evidence contains; it is not a label, outcome, causal finding, or release
/// decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0DiagnosticSignal {
    metric_id: String,
    numerator: u64,
    denominator: u64,
    missing: u64,
    coverage_basis_points: u16,
    window: E0DiagnosticWindow,
    cutoff_unix_seconds: u64,
    tenant_id: String,
    source_snapshot_ref: ArtifactReference,
    source_snapshot_binding: String,
    availability_profile_digest: String,
    table: String,
    source_contract_digest: String,
    source_digest: String,
    transform_digest: String,
    field_commitment: String,
    replay_projection_digest: String,
    source_evidence_digest: String,
    query_receipt_digests: Vec<String>,
    digest: String,
}

impl E0DiagnosticSignal {
    #[must_use]
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }

    #[must_use]
    pub fn numerator(&self) -> u64 {
        self.numerator
    }

    #[must_use]
    pub fn denominator(&self) -> u64 {
        self.denominator
    }

    #[must_use]
    pub fn missing(&self) -> u64 {
        self.missing
    }

    #[must_use]
    pub fn window(&self) -> E0DiagnosticWindow {
        self.window
    }

    #[must_use]
    pub fn cutoff_unix_seconds(&self) -> u64 {
        self.cutoff_unix_seconds
    }

    #[must_use]
    pub fn source_snapshot_binding(&self) -> &str {
        &self.source_snapshot_binding
    }

    #[must_use]
    pub fn availability_profile_digest(&self) -> &str {
        &self.availability_profile_digest
    }

    #[must_use]
    pub fn replay_projection_digest(&self) -> &str {
        &self.replay_projection_digest
    }

    #[must_use]
    pub fn query_receipt_digests(&self) -> &[String] {
        &self.query_receipt_digests
    }

    #[must_use]
    pub fn has_valid_digest(&self) -> bool {
        let mut unsigned = self.clone();
        let expected = std::mem::take(&mut unsigned.digest);
        expected == digest_of(&unsigned)
    }

    #[must_use]
    pub fn authorizes_execution_or_release(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum E0DiagnosticSensorError {
    InvalidSpec,
    InvalidWindow,
    NoAuthenticatedEvidence,
    WindowBeyondCutoff,
    EvidenceCommitmentMismatch,
    ReceiptMismatch,
    UnauthorizedField,
    MetricFieldMissing,
    EventTimeMissing,
    InvalidEventTime,
    EventOutsideWindow,
}

/// Deterministic calculation over only authenticated E0 evidence.
pub struct E0DiagnosticSensor;

impl E0DiagnosticSensor {
    #[must_use]
    pub fn authorizes_execution_or_release() -> bool {
        false
    }

    pub fn measure(
        spec: &E0BooleanRateSpec,
        window: E0DiagnosticWindow,
        evidence: &[VerifiedE0QueryResult],
    ) -> Result<E0DiagnosticSignal, E0DiagnosticSensorError> {
        let first = evidence
            .first()
            .ok_or(E0DiagnosticSensorError::NoAuthenticatedEvidence)?;
        let baseline = first.commitments();
        if window.end_unix_seconds > baseline.cutoff_unix_seconds() {
            return Err(E0DiagnosticSensorError::WindowBeyondCutoff);
        }

        let mut numerator = 0_u64;
        let mut denominator = 0_u64;
        let mut missing = 0_u64;
        let mut receipt_digests = BTreeSet::new();
        let mut query_receipt_digests = Vec::with_capacity(evidence.len());
        let first_receipt = first.result().receipt();

        for result in evidence {
            let commitments = result.commitments();
            let receipt = result.result().receipt();
            if !same_commitments(baseline, commitments) {
                return Err(E0DiagnosticSensorError::EvidenceCommitmentMismatch);
            }
            if receipt.tenant_id != baseline.tenant_id()
                || receipt.cutoff_unix_seconds != baseline.cutoff_unix_seconds()
                || receipt.queried_table != baseline.table()
                || receipt.source_contract_digest != baseline.source_contract_digest()
                || receipt.source_digest != baseline.source_digest()
                || receipt.transform_digest != baseline.transform_digest()
                || receipt.session_id != first_receipt.session_id
                || receipt.run_id != first_receipt.run_id
                || receipt.grant_id != first_receipt.grant_id
                || receipt.authority_ref != first_receipt.authority_ref
                || receipt.source_approval_id != first_receipt.source_approval_id
                || receipt.source_snapshot_ref != first_receipt.source_snapshot_ref
                || !receipt.has_valid_digest()
                || !receipt.binds_rows(result.result().rows())
                || !receipt_digests.insert(receipt.digest.clone())
            {
                return Err(E0DiagnosticSensorError::ReceiptMismatch);
            }
            if !receipt.accessed_columns.contains(&spec.field)
                || !receipt.accessed_columns.contains(&spec.event_time_field)
            {
                return Err(E0DiagnosticSensorError::UnauthorizedField);
            }
            for row in result.result().rows() {
                let event_time = row
                    .get(&spec.event_time_field)
                    .ok_or(E0DiagnosticSensorError::EventTimeMissing)?;
                let event_seconds = rfc3339_utc_to_unix_seconds(event_time)
                    .ok_or(E0DiagnosticSensorError::InvalidEventTime)?;
                if event_seconds < window.start_unix_seconds
                    || event_seconds > window.end_unix_seconds
                {
                    return Err(E0DiagnosticSensorError::EventOutsideWindow);
                }
                let value = row
                    .get(&spec.field)
                    .ok_or(E0DiagnosticSensorError::MetricFieldMissing)?;
                match value.as_str() {
                    "true" | "false" => {
                        denominator = denominator
                            .checked_add(1)
                            .ok_or(E0DiagnosticSensorError::ReceiptMismatch)?;
                        if value == &spec.positive_value {
                            numerator = numerator
                                .checked_add(1)
                                .ok_or(E0DiagnosticSensorError::ReceiptMismatch)?;
                        }
                    }
                    _ => {
                        missing = missing
                            .checked_add(1)
                            .ok_or(E0DiagnosticSensorError::ReceiptMismatch)?;
                    }
                }
            }
            query_receipt_digests.push(receipt.digest.clone());
        }

        let total = denominator
            .checked_add(missing)
            .ok_or(E0DiagnosticSensorError::ReceiptMismatch)?;
        let coverage_basis_points = denominator
            .checked_mul(10_000)
            .and_then(|scaled| scaled.checked_div(total))
            .unwrap_or(0) as u16;
        let mut signal = E0DiagnosticSignal {
            metric_id: spec.metric_id.clone(),
            numerator,
            denominator,
            missing,
            coverage_basis_points,
            window,
            cutoff_unix_seconds: baseline.cutoff_unix_seconds(),
            tenant_id: baseline.tenant_id().to_owned(),
            source_snapshot_ref: first_receipt.source_snapshot_ref.clone(),
            source_snapshot_binding: baseline.source_snapshot_binding().to_owned(),
            availability_profile_digest: baseline.availability_profile_digest().to_owned(),
            table: baseline.table().to_owned(),
            source_contract_digest: baseline.source_contract_digest().to_owned(),
            source_digest: baseline.source_digest().to_owned(),
            transform_digest: baseline.transform_digest().to_owned(),
            field_commitment: baseline.field_commitment().to_owned(),
            replay_projection_digest: baseline.replay_projection_digest().to_owned(),
            source_evidence_digest: baseline.source_evidence_digest().to_owned(),
            query_receipt_digests,
            digest: String::new(),
        };
        signal.digest = digest_of(&signal);
        Ok(signal)
    }
}

fn same_commitments(
    left: &crate::e0_query_lab::E0QueryCommitments,
    right: &crate::e0_query_lab::E0QueryCommitments,
) -> bool {
    left.tenant_id() == right.tenant_id()
        && left.cutoff_unix_seconds() == right.cutoff_unix_seconds()
        && left.source_snapshot_binding() == right.source_snapshot_binding()
        && left.availability_profile_digest() == right.availability_profile_digest()
        && left.table() == right.table()
        && left.source_contract_digest() == right.source_contract_digest()
        && left.source_digest() == right.source_digest()
        && left.transform_digest() == right.transform_digest()
        && left.field_commitment() == right.field_commitment()
        && left.replay_projection_digest() == right.replay_projection_digest()
        && left.source_evidence_digest() == right.source_evidence_digest()
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

fn digest_of<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("sensor evidence is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn rfc3339_utc_to_unix_seconds(value: &str) -> Option<u64> {
    if !(value.len() == 20
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
        && value.as_bytes().get(16) == Some(&b':')
        && value.ends_with('Z')
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| [4, 7, 10, 13, 16, 19].contains(&index) || byte.is_ascii_digit()))
    {
        return None;
    }
    let number = |start: usize, end: usize| value[start..end].parse::<i64>().ok();
    let (year, month, day, hour, minute, second) = (
        number(0, 4)?,
        number(5, 7)?,
        number(8, 10)?,
        number(11, 13)?,
        number(14, 16)?,
        number(17, 19)?,
    );
    if year == 0
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => return None,
    };
    if day > days_in_month {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let month_from_march = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days_since_epoch = era * 146_097 + day_of_era - 719_468;
    let seconds = days_since_epoch
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?;
    u64::try_from(seconds).ok()
}

/// ```compile_fail
/// use improvement_engine_core::deterministic_sensor::DeterministicSensor;
/// use improvement_engine_core::e0_deterministic_sensor::{
///     E0BooleanRateSpec, E0DiagnosticSensor, E0DiagnosticWindow,
/// };
/// use improvement_engine_core::local_lab::QueryResult;
///
/// let spec = E0BooleanRateSpec::new("metric", "flag", "event_time", "true").unwrap();
/// let window = E0DiagnosticWindow::new(0, 1).unwrap();
/// let public_result: Vec<QueryResult> = Vec::new();
/// let _ = E0DiagnosticSensor::measure(&spec, window, &public_result);
/// let _ = DeterministicSensor; // Generic U12 is not authenticated E0 evidence.
/// ```
const _E0_SENSOR_REJECTS_PUBLIC_QUERY_RESULTS: () = ();

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::e0_query_lab::E0QueryLab;
    use crate::enriched_history::VerifiedE0QueryProjection;
    use crate::local_lab::{
        InMemoryLabGrantAuthority, InMemoryLabSourceAuthority, LabAccess, LabDataClassification,
        LabGrant, LabQuery, LabSource, LabSourceApprovalPort, LabSourceManifest, LabTable,
        LocalInvestigationLab,
    };
    use crate::source_validation::VerifiedSourceArtifactBinding;

    fn digest(seed: char) -> String {
        format!("sha256:{}", seed.to_string().repeat(64))
    }

    fn source_ref(seed: char) -> ArtifactReference {
        ArtifactReference {
            tenant_id: "tenant_a".to_owned(),
            id: format!("018f50a1-7f00-7000-8000-00000000000{seed}"),
            revision: 1,
            digest: digest(seed),
        }
    }

    fn row(event_time: &str, resolved: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("event_time".to_owned(), event_time.to_owned()),
            ("resolved".to_owned(), resolved.to_owned()),
        ])
    }

    fn authenticated_result(
        projection_seed: char,
        selected_columns: Vec<&str>,
    ) -> VerifiedE0QueryResult {
        let snapshot = source_ref(projection_seed);
        let projection = VerifiedE0QueryProjection::deterministic_for_e0_query_test(
            "tenant_a",
            100,
            digest(projection_seed),
            digest('b'),
            "contacts",
            digest('c'),
            digest(projection_seed),
            digest('d'),
            digest('e'),
            digest('f'),
            BTreeMap::from([
                ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                ("resolved".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
            ]),
            vec![
                row("1970-01-01T00:01:40Z", ""),
                row("1970-01-01T00:01:40Z", "false"),
                row("1970-01-01T00:01:40Z", "true"),
            ],
        );
        let source = LabSource::new(
            LabSourceManifest {
                tenant_id: "tenant_a".to_owned(),
                snapshot_ref: snapshot.clone(),
                source_contract_digest: digest('c'),
                source_digest: digest(projection_seed),
                transform_digest: digest('d'),
                cutoff_unix_seconds: 100,
                classification: LabDataClassification::Treated,
                safe_for_discovery: true,
            },
            vec![LabTable::new(
                "contacts",
                vec!["event_time", "resolved"],
                vec![
                    row("1970-01-01T00:01:40Z", "true"),
                    row("1970-01-01T00:01:40Z", "false"),
                    row("1970-01-01T00:01:40Z", ""),
                ],
            )],
        )
        .unwrap();
        let approved = InMemoryLabSourceAuthority.approve(source).unwrap();
        let approved = projection
            .bind_approved_lab_source(
                approved,
                VerifiedSourceArtifactBinding::deterministic_for_test(
                    snapshot.clone(),
                    digest(projection_seed),
                ),
            )
            .unwrap();
        let access = LabAccess::new(
            "run_e0",
            "tenant_a",
            "investigation",
            "grant_e0",
            "authority_e0",
            snapshot,
            1_000,
        );
        let mut grants = InMemoryLabGrantAuthority::default();
        grants.issue(LabGrant::from_access(&access));
        let mut lab = LocalInvestigationLab::new(grants);
        let session = lab.open(access.clone(), approved, 100).unwrap();
        let result = lab
            .query(
                session.session_id(),
                &access,
                LabQuery::select("contacts", selected_columns, None),
                100,
            )
            .unwrap();
        let candidate = lab
            .governed_e0_candidate(session.session_id(), &access, &result.receipt().digest, 100)
            .unwrap();
        E0QueryLab::admit(&projection, candidate).unwrap()
    }

    fn spec() -> E0BooleanRateSpec {
        E0BooleanRateSpec::new("contact_resolution", "resolved", "event_time", "true").unwrap()
    }

    #[test]
    fn authenticated_e0_evidence_emits_reproducible_diagnostic_with_full_commitments() {
        let evidence = authenticated_result('a', vec!["event_time", "resolved"]);
        let window = E0DiagnosticWindow::new(100, 100).unwrap();

        let first =
            E0DiagnosticSensor::measure(&spec(), window, std::slice::from_ref(&evidence)).unwrap();
        let second = E0DiagnosticSensor::measure(&spec(), window, &[evidence]).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.numerator(), 1);
        assert_eq!(first.denominator(), 2);
        assert_eq!(first.missing(), 1);
        assert_eq!(first.window(), window);
        assert_eq!(first.cutoff_unix_seconds(), 100);
        assert_eq!(first.source_snapshot_binding(), digest('a'));
        assert_eq!(first.availability_profile_digest(), digest('b'));
        assert_eq!(first.replay_projection_digest(), digest('f'));
        assert_eq!(first.query_receipt_digests().len(), 1);
        assert!(first.has_valid_digest());
        assert!(!first.authorizes_execution_or_release());
    }

    #[test]
    fn window_cannot_cross_cutoff_and_every_event_must_be_within_it() {
        let evidence = authenticated_result('a', vec!["event_time", "resolved"]);
        assert_eq!(
            E0DiagnosticSensor::measure(
                &spec(),
                E0DiagnosticWindow::new(0, 101).unwrap(),
                &[evidence]
            ),
            Err(E0DiagnosticSensorError::WindowBeyondCutoff)
        );

        let evidence = authenticated_result('a', vec!["event_time", "resolved"]);
        assert_eq!(
            E0DiagnosticSensor::measure(
                &spec(),
                E0DiagnosticWindow::new(0, 99).unwrap(),
                &[evidence]
            ),
            Err(E0DiagnosticSensorError::EventOutsideWindow)
        );
    }

    #[test]
    fn authenticated_e0_results_with_drift_or_unread_metric_field_fail_closed() {
        let first = authenticated_result('a', vec!["event_time", "resolved"]);
        let changed_projection = authenticated_result('b', vec!["event_time", "resolved"]);
        assert_eq!(
            E0DiagnosticSensor::measure(
                &spec(),
                E0DiagnosticWindow::new(100, 100).unwrap(),
                &[first, changed_projection]
            ),
            Err(E0DiagnosticSensorError::EvidenceCommitmentMismatch)
        );

        let event_time_only = authenticated_result('a', vec!["event_time"]);
        assert_eq!(
            E0DiagnosticSensor::measure(
                &spec(),
                E0DiagnosticWindow::new(100, 100).unwrap(),
                &[event_time_only]
            ),
            Err(E0DiagnosticSensorError::UnauthorizedField)
        );
    }

    #[test]
    fn labels_are_not_inferred_from_e0_evidence() {
        let evidence = authenticated_result('a', vec!["event_time", "resolved"]);
        let label_spec =
            E0BooleanRateSpec::new("label_rate", "label", "event_time", "true").unwrap();
        assert_eq!(
            E0DiagnosticSensor::measure(
                &label_spec,
                E0DiagnosticWindow::new(100, 100).unwrap(),
                &[evidence]
            ),
            Err(E0DiagnosticSensorError::UnauthorizedField)
        );
    }
}
