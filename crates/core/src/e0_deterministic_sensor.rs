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
use crate::enriched_history::rfc3339_utc_to_unix_seconds;

/// The reviewed, versioned allowlist for descriptive E0 diagnostics. It is
/// deliberately an enum without caller-supplied field names: adding a metric
/// requires a source-reviewed policy revision rather than a prompt/config
/// asking the sensor to reinterpret an outcome as a diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum DiagnosticMetricPolicy {
    TechnicalErrorRateV1,
}

/// A sealed selection from the explicit diagnostic policy allowlist. The
/// commitment carries the policy identity, version, field and semantics into
/// the immutable output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticMetricSpec {
    metric_id: String,
    field: String,
    event_time_field: String,
    positive_value: String,
    semantics: String,
    policy_id: String,
    policy_version: u16,
    commitment: String,
}

impl DiagnosticMetricSpec {
    #[must_use]
    pub fn from_policy(policy: DiagnosticMetricPolicy) -> Self {
        let (
            metric_id,
            field,
            event_time_field,
            positive_value,
            semantics,
            policy_id,
            policy_version,
        ) = match policy {
            DiagnosticMetricPolicy::TechnicalErrorRateV1 => (
                "e0_technical_error_rate",
                "technical_error",
                "event_time",
                "true",
                "observed_technical_error_flag",
                "e0_diagnostic_allowlist",
                1,
            ),
        };
        let commitment = metric_spec_commitment(
            metric_id,
            field,
            event_time_field,
            positive_value,
            semantics,
            policy_id,
            policy_version,
        );
        Self {
            metric_id: metric_id.to_owned(),
            field: field.to_owned(),
            event_time_field: event_time_field.to_owned(),
            positive_value: positive_value.to_owned(),
            semantics: semantics.to_owned(),
            policy_id: policy_id.to_owned(),
            policy_version,
            commitment,
        }
    }

    #[must_use]
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }

    #[must_use]
    pub fn policy_id(&self) -> &str {
        &self.policy_id
    }

    #[must_use]
    pub fn policy_version(&self) -> u16 {
        self.policy_version
    }

    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
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
    metric_policy_id: String,
    metric_policy_version: u16,
    metric_semantics: String,
    metric_spec_commitment: String,
    numerator: u64,
    denominator: u64,
    missing: u64,
    coverage_basis_points: u16,
    window: E0DiagnosticWindow,
    cutoff_unix_seconds: u64,
    tenant_id: String,
    grant_id: String,
    authority_ref: String,
    run_id: String,
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

/// Crate-private projection for the U13-E trusted composition. It contains
/// only immutable commitments and descriptive aggregates, never query rows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct E0ScoutSignalBinding {
    pub(crate) signal_digest: String,
    pub(crate) metric_id: String,
    pub(crate) metric_spec_commitment: String,
    pub(crate) metric_policy_id: String,
    pub(crate) metric_policy_version: u16,
    pub(crate) metric_semantics: String,
    pub(crate) numerator: u64,
    pub(crate) denominator: u64,
    pub(crate) missing: u64,
    pub(crate) coverage_basis_points: u16,
    pub(crate) window: E0DiagnosticWindow,
    pub(crate) cutoff_unix_seconds: u64,
    pub(crate) tenant_id: String,
    pub(crate) grant_id: String,
    pub(crate) authority_ref: String,
    pub(crate) run_id: String,
    pub(crate) source_snapshot_ref: ArtifactReference,
    pub(crate) source_snapshot_binding: String,
    pub(crate) availability_profile_digest: String,
    pub(crate) table: String,
    pub(crate) source_contract_digest: String,
    pub(crate) source_digest: String,
    pub(crate) transform_digest: String,
    pub(crate) field_commitment: String,
    pub(crate) replay_projection_digest: String,
    pub(crate) source_evidence_digest: String,
    pub(crate) query_receipt_digests: Vec<String>,
}

impl E0DiagnosticSignal {
    #[allow(dead_code)] // Consumed by U13-E trusted composition.
    pub(crate) fn scout_binding(&self) -> E0ScoutSignalBinding {
        E0ScoutSignalBinding {
            signal_digest: self.digest.clone(),
            metric_id: self.metric_id.clone(),
            metric_spec_commitment: self.metric_spec_commitment.clone(),
            metric_policy_id: self.metric_policy_id.clone(),
            metric_policy_version: self.metric_policy_version,
            metric_semantics: self.metric_semantics.clone(),
            numerator: self.numerator,
            denominator: self.denominator,
            missing: self.missing,
            coverage_basis_points: self.coverage_basis_points,
            window: self.window,
            cutoff_unix_seconds: self.cutoff_unix_seconds,
            tenant_id: self.tenant_id.clone(),
            grant_id: self.grant_id.clone(),
            authority_ref: self.authority_ref.clone(),
            run_id: self.run_id.clone(),
            source_snapshot_ref: self.source_snapshot_ref.clone(),
            source_snapshot_binding: self.source_snapshot_binding.clone(),
            availability_profile_digest: self.availability_profile_digest.clone(),
            table: self.table.clone(),
            source_contract_digest: self.source_contract_digest.clone(),
            source_digest: self.source_digest.clone(),
            transform_digest: self.transform_digest.clone(),
            field_commitment: self.field_commitment.clone(),
            replay_projection_digest: self.replay_projection_digest.clone(),
            source_evidence_digest: self.source_evidence_digest.clone(),
            query_receipt_digests: self.query_receipt_digests.clone(),
        }
    }
    #[must_use]
    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }

    #[must_use]
    pub fn metric_policy_id(&self) -> &str {
        &self.metric_policy_id
    }

    #[must_use]
    pub fn metric_policy_version(&self) -> u16 {
        self.metric_policy_version
    }

    #[must_use]
    pub fn metric_spec_commitment(&self) -> &str {
        &self.metric_spec_commitment
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
        spec: &DiagnosticMetricSpec,
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
            metric_policy_id: spec.policy_id.clone(),
            metric_policy_version: spec.policy_version,
            metric_semantics: spec.semantics.clone(),
            metric_spec_commitment: spec.commitment.clone(),
            numerator,
            denominator,
            missing,
            coverage_basis_points,
            window,
            cutoff_unix_seconds: baseline.cutoff_unix_seconds(),
            tenant_id: baseline.tenant_id().to_owned(),
            grant_id: first_receipt.grant_id.clone(),
            authority_ref: first_receipt.authority_ref.clone(),
            run_id: first_receipt.run_id.clone(),
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

fn digest_of<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("sensor evidence is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Test-only U02 → U04-B V2 → U08 ledger → U12-E path. U13-E regression
/// tests use this instead of minting an `E0DiagnosticSignal` fixture.
#[cfg(all(test, feature = "test-support"))]
pub(crate) fn real_signal_for_scout_test() -> E0DiagnosticSignal {
    let evidence = tests::authenticated_result('a', vec!["event_time", "technical_error"]);
    E0DiagnosticSensor::measure(
        &DiagnosticMetricSpec::from_policy(DiagnosticMetricPolicy::TechnicalErrorRateV1),
        E0DiagnosticWindow::new(100, 100).expect("fixed test window"),
        &[evidence],
    )
    .expect("real E0 chain is valid")
}

/// Test-only faithful U02→U04-B replay companion for the real E0 signal.
/// It is deliberately emitted by the same helper that creates the U08 result,
/// rather than being a nominal `VerifiedReplayAvailability` fixture.
#[cfg(all(test, feature = "test-support"))]
pub(crate) fn real_signal_and_replay_for_frozen_summary_test() -> (
    E0DiagnosticSignal,
    crate::enriched_history::VerifiedReplayAvailability,
) {
    let (evidence, replay) =
        tests::authenticated_result_with_replay('a', vec!["event_time", "technical_error"]);
    (
        E0DiagnosticSensor::measure(
            &DiagnosticMetricSpec::from_policy(DiagnosticMetricPolicy::TechnicalErrorRateV1),
            E0DiagnosticWindow::new(100, 100).expect("fixed test window"),
            &[evidence],
        )
        .expect("real E0 chain is valid"),
        replay,
    )
}

#[allow(clippy::too_many_arguments)]
fn metric_spec_commitment(
    metric_id: &str,
    field: &str,
    event_time_field: &str,
    positive_value: &str,
    semantics: &str,
    policy_id: &str,
    policy_version: u16,
) -> String {
    #[derive(Serialize)]
    struct MetricPolicyCommitment<'a> {
        metric_id: &'a str,
        field: &'a str,
        event_time_field: &'a str,
        positive_value: &'a str,
        semantics: &'a str,
        policy_id: &'a str,
        policy_version: u16,
    }
    digest_of(&MetricPolicyCommitment {
        metric_id,
        field,
        event_time_field,
        positive_value,
        semantics,
        policy_id,
        policy_version,
    })
}

/// ```compile_fail
/// use improvement_engine_core::deterministic_sensor::DeterministicSensor;
/// use improvement_engine_core::e0_deterministic_sensor::{
///     DiagnosticMetricPolicy, DiagnosticMetricSpec, E0DiagnosticSensor, E0DiagnosticWindow,
/// };
/// use improvement_engine_core::local_lab::QueryResult;
///
/// let spec = DiagnosticMetricSpec::from_policy(DiagnosticMetricPolicy::TechnicalErrorRateV1);
/// let window = E0DiagnosticWindow::new(0, 1).unwrap();
/// let public_result: Vec<QueryResult> = Vec::new();
/// let _ = E0DiagnosticSensor::measure(&spec, window, &public_result);
/// let _ = DeterministicSensor; // Generic U12 is not authenticated E0 evidence.
/// ```
const _E0_SENSOR_REJECTS_PUBLIC_QUERY_RESULTS: () = ();

/// ```compile_fail
/// use improvement_engine_core::e0_query_lab::{E0QueryCommitments, VerifiedE0QueryResult};
///
/// // The commitment is crate-private and neither it nor the opaque result can
/// // be minted from caller-controlled rows, receipts or digest strings.
/// let _ = E0QueryCommitments {};
/// let _ = VerifiedE0QueryResult {};
/// ```
const _E0_EVIDENCE_CANNOT_BE_CONSTRUCTED_EXTERNALLY: () = ();

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::e0_query_lab::E0QueryLab;
    use crate::enriched_history::{
        AvailabilityClockMode, AvailabilityProfile, EnrichedHistoryAdapter,
        EnrichedHistoryManifest, PackageFile, ProvenanceDigests, ReplayRowAvailability, TableInput,
        replay_projection_digest,
    };
    use crate::local_lab::{
        InMemoryLabGrantAuthority, InMemoryLabSourceAuthority, LabAccess, LabDataClassification,
        LabGrant, LabQuery, LabSource, LabSourceApprovalPort, LabSourceManifest, LabTable,
        LocalInvestigationLab,
    };
    use crate::source_validation::{SourceSnapshot, resolve_source_snapshot_artifact};
    use crate::{ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository};
    use serde_json::json;

    fn digest(seed: char) -> String {
        format!("sha256:{}", seed.to_string().repeat(64))
    }

    fn row(event_time: &str, technical_error: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("event_time".to_owned(), event_time.to_owned()),
            ("technical_error".to_owned(), technical_error.to_owned()),
        ])
    }

    pub(super) fn authenticated_result(
        projection_seed: char,
        selected_columns: Vec<&str>,
    ) -> VerifiedE0QueryResult {
        authenticated_result_with_replay(projection_seed, selected_columns).0
    }

    pub(super) fn authenticated_result_with_replay(
        projection_seed: char,
        selected_columns: Vec<&str>,
    ) -> (
        VerifiedE0QueryResult,
        crate::enriched_history::VerifiedReplayAvailability,
    ) {
        // Deliberately non-canonical: outer whitespace and nested/root key
        // order are part of the U04 source-byte binding, not the U02 artifact
        // content digest.
        let raw_snapshot = format!(
            r#"
 {{ "world_ref":"world_a", "sources":[{{"row_count":3,"header_digest":"{}","table":"contacts","source_contract_ref":{{"version":"v1","digest":"{}","id":"contacts"}},"uri":"file://contacts.csv","file_digest":"{}"}}], "tenant_id":"tenant_a", "observed_cutoff":"1970-01-01T00:01:40Z", "contract_version":{{"minor":0,"major":1}}, "source_namespace":"platform_history" }}
"#,
            digest('b'),
            digest('c'),
            digest(projection_seed),
        );
        let snapshot = SourceSnapshot::from_json(&raw_snapshot).unwrap();
        let rows = vec![
            json!({"event_time":"1970-01-01T00:01:40Z", "technical_error":"true"}),
            json!({"event_time":"1970-01-01T00:01:40Z", "technical_error":"false"}),
            json!({"event_time":"1970-01-01T00:01:40Z", "technical_error":""}),
        ];
        let availability = (0..rows.len())
            .map(|_| {
                ReplayRowAvailability::new(BTreeMap::from([
                    ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                    (
                        "technical_error".to_owned(),
                        "1970-01-01T00:01:40Z".to_owned(),
                    ),
                ]))
            })
            .collect::<Vec<_>>();
        let manifest = EnrichedHistoryManifest::new_replay(
            "platform_history",
            "world_a",
            "1970-01-01T00:01:40Z",
            AvailabilityProfile::new(
                "e0_replay",
                1,
                AvailabilityClockMode::replay_at_event_time("e0_zero_lag"),
                "tenant_a",
                snapshot.binding_digest(),
            ),
            vec![
                PackageFile::new(
                    "contacts",
                    ProvenanceDigests::new(
                        digest(projection_seed),
                        digest('b'),
                        digest('c'),
                        digest('d'),
                    ),
                    "1970-01-01T00:01:40Z",
                )
                .with_field_availability(BTreeMap::from([
                    ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                    (
                        "technical_error".to_owned(),
                        "1970-01-01T00:01:40Z".to_owned(),
                    ),
                ]))
                .with_replay_projection_digest(replay_projection_digest(&rows, &availability))
                .with_source_file_seal(snapshot.source_file_seal("contacts").unwrap()),
            ],
        );
        let adapter = EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot).unwrap();
        let replay = adapter.verified_replay_availability(&snapshot).unwrap();
        let projection = adapter
            .verified_e0_query_projection(
                &snapshot,
                &replay,
                "contacts",
                TableInput::new(
                    ProvenanceDigests::new(
                        digest(projection_seed),
                        digest('b'),
                        digest('c'),
                        digest('d'),
                    ),
                    rows,
                )
                .with_replay_row_availability(availability),
            )
            .unwrap();

        let mut source_repository = InMemoryArtifactRepository::default();
        let snapshot_artifact = source_repository
            .append(
                None,
                ArtifactDraft::new(
                    "tenant_a",
                    format!("018f50a1-7f00-7000-8000-00000000000{projection_seed}"),
                    1,
                    ArtifactKind::SourceSnapshot,
                    json!({"source_snapshot_json": raw_snapshot}),
                    None,
                ),
            )
            .unwrap()
            .reference();
        assert_ne!(snapshot_artifact.digest, snapshot.binding_digest());
        let source_binding =
            resolve_source_snapshot_artifact(&mut source_repository, &snapshot_artifact).unwrap();
        assert_eq!(
            source_binding.snapshot_binding_digest(),
            snapshot.binding_digest()
        );

        let source = LabSource::new(
            LabSourceManifest {
                tenant_id: "tenant_a".to_owned(),
                snapshot_ref: snapshot_artifact.clone(),
                source_contract_digest: digest('b'),
                source_digest: digest(projection_seed),
                transform_digest: digest('c'),
                cutoff_unix_seconds: 100,
                classification: LabDataClassification::Treated,
                safe_for_discovery: true,
            },
            vec![LabTable::new(
                "contacts",
                vec!["event_time", "technical_error"],
                vec![
                    row("1970-01-01T00:01:40Z", ""),
                    row("1970-01-01T00:01:40Z", "false"),
                    row("1970-01-01T00:01:40Z", "true"),
                ],
            )],
        )
        .unwrap();
        let approved = InMemoryLabSourceAuthority.approve(source).unwrap();
        let approved = projection
            .bind_approved_lab_source(approved, source_binding)
            .unwrap();
        let access = LabAccess::new(
            "run_e0",
            "tenant_a",
            "investigation",
            "grant_e0",
            "authority_e0",
            snapshot_artifact,
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
        (E0QueryLab::admit(&projection, candidate).unwrap(), replay)
    }

    fn spec() -> DiagnosticMetricSpec {
        DiagnosticMetricSpec::from_policy(DiagnosticMetricPolicy::TechnicalErrorRateV1)
    }

    #[test]
    fn authenticated_e0_evidence_emits_reproducible_diagnostic_with_full_commitments() {
        let evidence = authenticated_result('a', vec!["event_time", "technical_error"]);
        let window = E0DiagnosticWindow::new(100, 100).unwrap();
        let source_artifact_digest = evidence
            .result()
            .receipt()
            .source_snapshot_ref
            .digest
            .clone();

        let first =
            E0DiagnosticSensor::measure(&spec(), window, std::slice::from_ref(&evidence)).unwrap();
        let second = E0DiagnosticSensor::measure(&spec(), window, &[evidence]).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.numerator(), 1);
        assert_eq!(first.denominator(), 2);
        assert_eq!(first.missing(), 1);
        assert_eq!(first.metric_policy_id(), "e0_diagnostic_allowlist");
        assert_eq!(first.metric_policy_version(), 1);
        assert_eq!(first.metric_spec_commitment(), spec().commitment());
        assert_eq!(first.window(), window);
        assert_eq!(first.cutoff_unix_seconds(), 100);
        assert_ne!(first.source_snapshot_binding(), source_artifact_digest);
        assert!(first.source_snapshot_binding().starts_with("sha256:"));
        assert!(first.availability_profile_digest().starts_with("sha256:"));
        assert!(first.replay_projection_digest().starts_with("sha256:"));
        assert_eq!(first.query_receipt_digests().len(), 1);
        assert!(first.has_valid_digest());
        assert!(!first.authorizes_execution_or_release());
    }

    #[test]
    fn window_cannot_cross_cutoff_and_every_event_must_be_within_it() {
        let evidence = authenticated_result('a', vec!["event_time", "technical_error"]);
        assert_eq!(
            E0DiagnosticSensor::measure(
                &spec(),
                E0DiagnosticWindow::new(0, 101).unwrap(),
                &[evidence]
            ),
            Err(E0DiagnosticSensorError::WindowBeyondCutoff)
        );

        let evidence = authenticated_result('a', vec!["event_time", "technical_error"]);
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
        let first = authenticated_result('a', vec!["event_time", "technical_error"]);
        let changed_projection = authenticated_result('b', vec!["event_time", "technical_error"]);
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
    fn only_the_reviewed_descriptive_metric_policy_can_measure_e0_evidence() {
        let evidence = authenticated_result('a', vec!["event_time", "technical_error"]);
        let signal = E0DiagnosticSensor::measure(
            &spec(),
            E0DiagnosticWindow::new(100, 100).unwrap(),
            &[evidence],
        )
        .unwrap();
        assert_eq!(signal.metric_id(), "e0_technical_error_rate");
        assert_ne!(signal.metric_id(), "resolved");
    }
}
