use crate::{
    ArtifactKind, ArtifactReference, ArtifactRepository, source_validation::SourceSnapshot,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionTable {
    Contacts,
    Complaints,
}
impl ProjectionTable {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contacts => "call_center_interactions",
            Self::Complaints => "complaints",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionCoverage {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionPolicy {
    pub version: u32,
    pub minimum_cell_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestPartition {
    id: String,
    sha256: String,
}
impl ManifestPartition {
    pub fn new(id: String, sha256: String) -> Self {
        Self { id, sha256 }
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    InvalidManifest,
    PartitionSetMismatch,
    DigestMismatch,
    DuplicateHeader,
    HeaderDigestMismatch,
    MalformedCsv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportStatus {
    Supported,
    Unsupported { missing_fields: Vec<&'static str> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionTemporalSemantics {
    /// Rows are selected by source event timestamp; no as-of claim is implied
    /// for status/outcome values carried by those rows.
    EventDateCohort,
    /// Rows are selected by creation timestamp at or before cutoff, but their
    /// outcome fields are final values from the extract and may be later.
    CreationCohortWithFinalOutcomes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContactCategory {
    Complaint,
    Transactional,
    Technical,
    GeneralInquiry,
    Product,
    Account,
    Card,
    Loan,
    Other,
    Unclassified,
}
impl ContactCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complaint => "complaint",
            Self::Transactional => "transactional",
            Self::Technical => "technical",
            Self::GeneralInquiry => "general_inquiry",
            Self::Product => "product",
            Self::Account => "account",
            Self::Card => "card",
            Self::Loan => "loan",
            Self::Other => "other",
            Self::Unclassified => "unclassified",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    Phone,
    Web,
    Chat,
    Email,
    Branch,
    MobileApp,
    Other,
}
impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Web => "web",
            Self::Chat => "chat",
            Self::Email => "email",
            Self::Branch => "branch",
            Self::MobileApp => "mobile_app",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricSummary {
    pub valid_count: u64,
    pub missing_count: u64,
    pub mean: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContactAggregate {
    pub period: String,
    pub category: ContactCategory,
    pub channel: Channel,
    pub contact_count: u64,
    pub resolved: BooleanSummary,
    pub followup: BooleanSummary,
    pub escalated: BooleanSummary,
    pub duration_seconds: MetricSummary,
    pub wait_time_seconds: MetricSummary,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComplaintAggregate {
    pub period: String,
    pub category: ContactCategory,
    pub channel: Channel,
    pub complaint_count: u64,
    /// Final extract value for the creation-date cohort; not an as-of-cutoff metric.
    pub final_sla_breached: BooleanSummary,
    /// Elapsed days to the final recorded first response; may be after cutoff.
    pub final_first_response_elapsed_days: MetricSummary,
    /// Final extract's resolution duration; may be after cutoff.
    pub final_resolution_days: MetricSummary,
    /// Final extract's satisfaction score; may be after cutoff.
    pub final_resolution_satisfaction: MetricSummary,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BooleanSummary {
    pub valid_count: u64,
    pub missing_count: u64,
    pub positive_count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Projection<T> {
    pub status: SupportStatus,
    /// Explicitly distinguishes row-date cohorting from retrospective outcomes.
    pub temporal_semantics: ProjectionTemporalSemantics,
    pub source_snapshot_ref: ArtifactReference,
    pub source_snapshot_binding_digest: String,
    pub manifest_digest: String,
    /// Snapshot observation cutoff. Its business-time interpretation is bounded
    /// by `temporal_semantics`; it does not imply outcome censoring.
    pub observed_cutoff: String,
    pub coverage: ProjectionCoverage,
    pub policy_version: u32,
    pub suppressed_count: u64,
    pub rejected_rows: u64,
    pub aggregates: Vec<T>,
}

pub struct CsvPartition<R> {
    id: String,
    reader: R,
}
impl<R> CsvPartition<R> {
    pub fn new(id: impl Into<String>, reader: R) -> Self {
        Self {
            id: id.into(),
            reader,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionManifest {
    source_snapshot_ref: ArtifactReference,
    snapshot_binding_digest: String,
    table: ProjectionTable,
    source_header_digest: String,
    cutoff_timestamp: String,
    observed_cutoff: String,
    partitions: BTreeMap<String, String>,
    coverage: ProjectionCoverage,
    policy: ProjectionPolicy,
    digest: String,
}

impl ProjectionManifest {
    pub fn new(
        repository: &mut impl ArtifactRepository,
        source_snapshot_ref: ArtifactReference,
        table: ProjectionTable,
        partitions: Vec<ManifestPartition>,
        coverage: ProjectionCoverage,
        policy: ProjectionPolicy,
    ) -> Result<Self, ProjectionError> {
        let draft = repository
            .get(
                &source_snapshot_ref.tenant_id,
                &source_snapshot_ref.id,
                source_snapshot_ref.revision,
            )
            .map_err(|_| ProjectionError::InvalidManifest)?
            .ok_or(ProjectionError::InvalidManifest)?;
        if draft.kind != ArtifactKind::SourceSnapshot || draft.reference() != source_snapshot_ref {
            return Err(ProjectionError::InvalidManifest);
        }
        let raw_snapshot = draft
            .payload
            .get("source_snapshot_json")
            .and_then(serde_json::Value::as_str)
            .ok_or(ProjectionError::InvalidManifest)?;
        let snapshot = SourceSnapshot::from_json(raw_snapshot)
            .map_err(|_| ProjectionError::InvalidManifest)?;
        if snapshot.tenant_id() != source_snapshot_ref.tenant_id {
            return Err(ProjectionError::InvalidManifest);
        }
        Self::from_verified_snapshot(
            source_snapshot_ref,
            &snapshot,
            table,
            partitions,
            coverage,
            policy,
        )
    }

    fn from_verified_snapshot(
        source_snapshot_ref: ArtifactReference,
        snapshot: &SourceSnapshot,
        table: ProjectionTable,
        partitions: Vec<ManifestPartition>,
        coverage: ProjectionCoverage,
        policy: ProjectionPolicy,
    ) -> Result<Self, ProjectionError> {
        if !snapshot.has_canonical_binding()
            || snapshot.source_file_seal(table.as_str()).is_none()
            || source_snapshot_ref.tenant_id != snapshot.tenant_id()
            || source_snapshot_ref.revision == 0
            || !valid_uuid_v7(&source_snapshot_ref.id)
            || !valid_digest(&source_snapshot_ref.digest)
            || policy.version == 0
            || policy.minimum_cell_count == 0
            || partitions.is_empty()
        {
            return Err(ProjectionError::InvalidManifest);
        }
        let mut expected = BTreeMap::new();
        for partition in partitions {
            if !valid_partition_id(&partition.id)
                || !valid_digest(&partition.sha256)
                || expected.insert(partition.id, partition.sha256).is_some()
            {
                return Err(ProjectionError::InvalidManifest);
            }
        }
        let cutoff = snapshot.observed_cutoff().to_owned();
        if parse_utc_timestamp(&cutoff).is_none() {
            return Err(ProjectionError::InvalidManifest);
        }
        let seal = snapshot
            .source_file_seal(table.as_str())
            .ok_or(ProjectionError::InvalidManifest)?;
        let inventory_digest = partition_inventory_digest(&expected);
        if seal.partition_inventory_digest() != Some(inventory_digest.as_str()) {
            return Err(ProjectionError::InvalidManifest);
        }
        let source_header_digest = seal.header_digest().to_owned();
        let snapshot_binding_digest = snapshot.binding_digest();
        let digest = manifest_digest(ManifestDigestInput {
            reference: &source_snapshot_ref,
            snapshot_digest: &snapshot_binding_digest,
            table,
            source_header_digest: &source_header_digest,
            cutoff: &cutoff,
            partitions: &expected,
            coverage,
            policy,
        });
        Ok(Self {
            source_snapshot_ref,
            snapshot_binding_digest,
            table,
            source_header_digest,
            cutoff_timestamp: cutoff.clone(),
            observed_cutoff: cutoff,
            partitions: expected,
            coverage,
            policy,
            digest,
        })
    }
}

pub fn project_contacts<R: Read>(
    manifest: &ProjectionManifest,
    partitions: impl IntoIterator<Item = CsvPartition<R>>,
) -> Result<Projection<ContactAggregate>, ProjectionError> {
    if manifest.table != ProjectionTable::Contacts {
        return Err(ProjectionError::InvalidManifest);
    }
    let mut seen = BTreeSet::new();
    let mut missing_fields = BTreeSet::new();
    let mut grouped: BTreeMap<(String, ContactCategory, Channel), ContactAccumulator> =
        BTreeMap::new();
    let mut rejected_rows = 0_u64;
    let mut valid_timestamps = 0_u64;
    for partition in partitions {
        if !manifest.partitions.contains_key(&partition.id) || !seen.insert(partition.id.clone()) {
            return Err(ProjectionError::PartitionSetMismatch);
        }
        let hashed = HashingReader::new(partition.reader);
        let mut csv = csv::ReaderBuilder::new()
            .flexible(false)
            .from_reader(hashed);
        let headers = csv
            .headers()
            .map_err(|_| ProjectionError::MalformedCsv)?
            .clone();
        let mut positions = BTreeMap::new();
        for (index, header) in headers.iter().enumerate() {
            let name = header.trim().to_ascii_lowercase();
            if positions.insert(name, index).is_some() {
                return Err(ProjectionError::DuplicateHeader);
            }
        }
        let date_idx = field(&positions, &["interaction_date"]);
        let category_idx = field(&positions, &["reason_category", "contact_reason"]);
        let channel_idx = field(&positions, &["channel"]);
        for (name, found) in [
            ("interaction_date", date_idx.is_some()),
            ("reason_category|contact_reason", category_idx.is_some()),
            ("channel", channel_idx.is_some()),
        ] {
            if !found {
                missing_fields.insert(name);
            }
        }
        if let (Some(date_idx), Some(category_idx), Some(channel_idx)) =
            (date_idx, category_idx, channel_idx)
        {
            for record in csv.records() {
                let record = record.map_err(|_| ProjectionError::MalformedCsv)?;
                if record.len() != headers.len() {
                    return Err(ProjectionError::MalformedCsv);
                }
                let date = csv_value(&record, date_idx);
                let Some((timestamp, period)) = date_and_period(date) else {
                    rejected_rows += 1;
                    continue;
                };
                valid_timestamps += 1;
                if timestamp.as_str() > manifest.cutoff_timestamp.as_str() {
                    continue;
                }
                let Some(channel) = normalize_channel(csv_value(&record, channel_idx)) else {
                    rejected_rows += 1;
                    continue;
                };
                let category = normalize_category(csv_value(&record, category_idx));
                let aggregate = grouped.entry((period, category, channel)).or_default();
                aggregate.contact_count += 1;
                aggregate.resolved.push(parse_bool(csv_field(
                    &record,
                    field(&positions, &["was_resolved"]),
                )));
                aggregate.followup.push(parse_bool(csv_field(
                    &record,
                    field(&positions, &["requires_followup"]),
                )));
                aggregate.escalated.push(parse_bool(csv_field(
                    &record,
                    field(&positions, &["was_escalated"]),
                )));
                aggregate.duration_seconds.push(parse_nonnegative(csv_field(
                    &record,
                    field(&positions, &["duration_seconds"]),
                )));
                aggregate
                    .wait_time_seconds
                    .push(parse_nonnegative(csv_field(
                        &record,
                        field(&positions, &["wait_time_seconds"]),
                    )));
            }
        } else {
            // Still consume and hash the file so a wrong/mutated partition is not accepted.
            for record in csv.records() {
                record.map_err(|_| ProjectionError::MalformedCsv)?;
            }
        }
        let hasher = csv.into_inner();
        let header_digest = hasher.header_digest();
        let actual_digest = format!("sha256:{:x}", hasher.hash.finalize());
        if manifest.partitions.get(&partition.id) != Some(&actual_digest) {
            return Err(ProjectionError::DigestMismatch);
        }
        if header_digest != manifest.source_header_digest {
            return Err(ProjectionError::HeaderDigestMismatch);
        }
    }
    if seen.len() != manifest.partitions.len() {
        return Err(ProjectionError::PartitionSetMismatch);
    }
    let status = if missing_fields.is_empty() && (valid_timestamps > 0 || rejected_rows == 0) {
        SupportStatus::Supported
    } else {
        if valid_timestamps == 0 && rejected_rows > 0 {
            missing_fields.insert("timezone-qualified timestamp");
        }
        SupportStatus::Unsupported {
            missing_fields: missing_fields.into_iter().collect(),
        }
    };
    let mut suppressed_count = 0;
    let aggregates = if status == SupportStatus::Supported {
        grouped
            .into_iter()
            .filter_map(|((period, category, channel), values)| {
                if values.contact_count < manifest.policy.minimum_cell_count {
                    suppressed_count += 1;
                    None
                } else {
                    Some(ContactAggregate {
                        period,
                        category,
                        channel,
                        contact_count: values.contact_count,
                        resolved: values.resolved.finish(values.contact_count),
                        followup: values.followup.finish(values.contact_count),
                        escalated: values.escalated.finish(values.contact_count),
                        duration_seconds: values.duration_seconds.finish(values.contact_count),
                        wait_time_seconds: values.wait_time_seconds.finish(values.contact_count),
                    })
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Projection {
        status,
        temporal_semantics: ProjectionTemporalSemantics::EventDateCohort,
        source_snapshot_ref: manifest.source_snapshot_ref.clone(),
        source_snapshot_binding_digest: manifest.snapshot_binding_digest.clone(),
        manifest_digest: manifest.digest.clone(),
        observed_cutoff: manifest.observed_cutoff.clone(),
        coverage: manifest.coverage,
        policy_version: manifest.policy.version,
        suppressed_count,
        rejected_rows,
        aggregates,
    })
}

pub fn project_complaints<R: Read>(
    manifest: &ProjectionManifest,
    partitions: impl IntoIterator<Item = CsvPartition<R>>,
) -> Result<Projection<ComplaintAggregate>, ProjectionError> {
    if manifest.table != ProjectionTable::Complaints {
        return Err(ProjectionError::InvalidManifest);
    }
    let mut seen = BTreeSet::new();
    let mut missing_fields = BTreeSet::new();
    let mut grouped: BTreeMap<(String, ContactCategory, Channel), ComplaintAccumulator> =
        BTreeMap::new();
    let mut rejected_rows = 0_u64;
    let mut valid_timestamps = 0_u64;
    for partition in partitions {
        if !manifest.partitions.contains_key(&partition.id) || !seen.insert(partition.id.clone()) {
            return Err(ProjectionError::PartitionSetMismatch);
        }
        let hashed = HashingReader::new(partition.reader);
        let mut csv = csv::ReaderBuilder::new()
            .flexible(false)
            .from_reader(hashed);
        let headers = csv
            .headers()
            .map_err(|_| ProjectionError::MalformedCsv)?
            .clone();
        let mut positions = BTreeMap::new();
        for (index, header) in headers.iter().enumerate() {
            if positions
                .insert(header.trim().to_ascii_lowercase(), index)
                .is_some()
            {
                return Err(ProjectionError::DuplicateHeader);
            }
        }
        let date_idx = field(&positions, &["creation_date"]);
        let category_idx = field(&positions, &["category"]);
        let channel_idx = field(&positions, &["reception_channel"]);
        for (name, found) in [
            ("creation_date", date_idx.is_some()),
            ("category", category_idx.is_some()),
            ("reception_channel", channel_idx.is_some()),
        ] {
            if !found {
                missing_fields.insert(name);
            }
        }
        if let (Some(date_idx), Some(category_idx), Some(channel_idx)) =
            (date_idx, category_idx, channel_idx)
        {
            for result in csv.records() {
                let record = result.map_err(|_| ProjectionError::MalformedCsv)?;
                if record.len() != headers.len() {
                    return Err(ProjectionError::MalformedCsv);
                }
                let Some((creation_timestamp, period)) =
                    date_and_period(csv_value(&record, date_idx))
                else {
                    rejected_rows += 1;
                    continue;
                };
                valid_timestamps += 1;
                if creation_timestamp > manifest.cutoff_timestamp {
                    continue;
                }
                let Some(channel) = normalize_channel(csv_value(&record, channel_idx)) else {
                    rejected_rows += 1;
                    continue;
                };
                let category = normalize_category(csv_value(&record, category_idx));
                let aggregate = grouped.entry((period, category, channel)).or_default();
                aggregate.complaint_count += 1;
                aggregate.sla_breached.push(parse_bool(csv_field(
                    &record,
                    field(&positions, &["sla_breached"]),
                )));
                let response_days = field(&positions, &["first_response_date"])
                    .and_then(|idx| csv_value(&record, idx))
                    .and_then(|end| elapsed_timestamp_days(csv_value(&record, date_idx)?, end));
                aggregate.first_response_calendar_days.push(response_days);
                aggregate.resolution_days.push(parse_nonnegative(csv_field(
                    &record,
                    field(&positions, &["resolution_days"]),
                )));
                aggregate
                    .resolution_satisfaction
                    .push(parse_satisfaction(csv_field(
                        &record,
                        field(&positions, &["resolution_satisfaction"]),
                    )));
            }
        } else {
            for record in csv.records() {
                record.map_err(|_| ProjectionError::MalformedCsv)?;
            }
        }
        let hasher = csv.into_inner();
        let header_digest = hasher.header_digest();
        let actual_digest = format!("sha256:{:x}", hasher.hash.finalize());
        if manifest.partitions.get(&partition.id) != Some(&actual_digest) {
            return Err(ProjectionError::DigestMismatch);
        }
        if header_digest != manifest.source_header_digest {
            return Err(ProjectionError::HeaderDigestMismatch);
        }
    }
    if seen.len() != manifest.partitions.len() {
        return Err(ProjectionError::PartitionSetMismatch);
    }
    let status = if missing_fields.is_empty() && (valid_timestamps > 0 || rejected_rows == 0) {
        SupportStatus::Supported
    } else {
        if valid_timestamps == 0 && rejected_rows > 0 {
            missing_fields.insert("timezone-qualified timestamp");
        }
        SupportStatus::Unsupported {
            missing_fields: missing_fields.into_iter().collect(),
        }
    };
    let mut suppressed_count = 0;
    let aggregates = if status == SupportStatus::Supported {
        grouped
            .into_iter()
            .filter_map(|((period, category, channel), values)| {
                if values.complaint_count < manifest.policy.minimum_cell_count {
                    suppressed_count += 1;
                    None
                } else {
                    Some(ComplaintAggregate {
                        period,
                        category,
                        channel,
                        complaint_count: values.complaint_count,
                        final_sla_breached: values.sla_breached.finish(values.complaint_count),
                        final_first_response_elapsed_days: values
                            .first_response_calendar_days
                            .finish(values.complaint_count),
                        final_resolution_days: values
                            .resolution_days
                            .finish(values.complaint_count),
                        final_resolution_satisfaction: values
                            .resolution_satisfaction
                            .finish(values.complaint_count),
                    })
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Projection {
        status,
        temporal_semantics: ProjectionTemporalSemantics::CreationCohortWithFinalOutcomes,
        source_snapshot_ref: manifest.source_snapshot_ref.clone(),
        source_snapshot_binding_digest: manifest.snapshot_binding_digest.clone(),
        manifest_digest: manifest.digest.clone(),
        observed_cutoff: manifest.observed_cutoff.clone(),
        coverage: manifest.coverage,
        policy_version: manifest.policy.version,
        suppressed_count,
        rejected_rows,
        aggregates,
    })
}

/// Canonical `SourceSnapshot.partition_inventory_digest` for a partitioned
/// source table. Partition IDs are opaque and sorted before hashing. Each ID
/// and digest is length-prefixed to make the encoding unambiguous.
#[must_use]
pub fn canonical_partition_inventory_digest(partitions: &[ManifestPartition]) -> Option<String> {
    let mut inventory = BTreeMap::new();
    for partition in partitions {
        if !valid_partition_id(&partition.id)
            || !valid_digest(&partition.sha256)
            || inventory
                .insert(partition.id.clone(), partition.sha256.clone())
                .is_some()
        {
            return None;
        }
    }
    if inventory.is_empty() {
        return None;
    }
    Some(partition_inventory_digest(&inventory))
}

fn partition_inventory_digest(partitions: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pulso-source-partition-inventory-v1\0");
    for (id, digest) in partitions {
        hasher.update((id.len() as u64).to_be_bytes());
        hasher.update(id.as_bytes());
        hasher.update((digest.len() as u64).to_be_bytes());
        hasher.update(digest.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[derive(Default)]
struct ComplaintAccumulator {
    complaint_count: u64,
    sla_breached: BoolAccumulator,
    first_response_calendar_days: NumericAccumulator,
    resolution_days: NumericAccumulator,
    resolution_satisfaction: NumericAccumulator,
}

fn parse_satisfaction(value: Option<&str>) -> Option<f64> {
    parse_nonnegative(value).filter(|score| (1.0..=5.0).contains(score))
}

fn date_ordinal(date: &str) -> Option<i64> {
    let (year, month, day) = (
        date[..4].parse::<i64>().ok()?,
        date[5..7].parse::<i64>().ok()?,
        date[8..10].parse::<i64>().ok()?,
    );
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146097 + day_of_era)
}

fn elapsed_timestamp_days(start: &str, end: &str) -> Option<f64> {
    let (start, _) = date_and_period(Some(start))?;
    let (end, _) = date_and_period(Some(end))?;
    let elapsed_seconds = parse_utc_timestamp(&end)? - parse_utc_timestamp(&start)?;
    if elapsed_seconds < 0 {
        return None;
    }
    Some(elapsed_seconds as f64 / 86_400.0)
}

fn valid_partition_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

struct HashingReader<R> {
    inner: R,
    hash: Sha256,
    header: Vec<u8>,
    header_complete: bool,
}
impl<R> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
            header: Vec::new(),
            header_complete: false,
        }
    }

    fn header_digest(&self) -> String {
        let bytes = self.header.strip_suffix(b"\r").unwrap_or(&self.header);
        format!("sha256:{:x}", Sha256::digest(bytes))
    }
}
impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.hash.update(&buffer[..count]);
        if !self.header_complete {
            for byte in &buffer[..count] {
                if *byte == b'\n' {
                    self.header_complete = true;
                    break;
                }
                self.header.push(*byte);
            }
        }
        Ok(count)
    }
}
fn field(positions: &BTreeMap<String, usize>, aliases: &[&str]) -> Option<usize> {
    aliases
        .iter()
        .find_map(|name| positions.get(*name).copied())
}
fn csv_value(record: &csv::StringRecord, index: usize) -> Option<&str> {
    record
        .get(index)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}
fn csv_field(record: &csv::StringRecord, index: Option<usize>) -> Option<&str> {
    index.and_then(|index| csv_value(record, index))
}
fn normalize_channel(value: Option<&str>) -> Option<Channel> {
    let normalized = value?.trim().to_ascii_lowercase();
    Some(match normalized.as_str() {
        "phone" | "telefono" | "teléfono" | "call" => Channel::Phone,
        "web" | "website" => Channel::Web,
        "chat" | "live chat" => Channel::Chat,
        "email" | "correo" => Channel::Email,
        "branch" | "sucursal" | "in-person" => Channel::Branch,
        "mobile app" | "mobile_app" | "app" => Channel::MobileApp,
        _ => Channel::Other,
    })
}

#[derive(Default)]
struct ContactAccumulator {
    contact_count: u64,
    resolved: BoolAccumulator,
    followup: BoolAccumulator,
    escalated: BoolAccumulator,
    duration_seconds: NumericAccumulator,
    wait_time_seconds: NumericAccumulator,
}
#[derive(Default)]
struct BoolAccumulator {
    valid_count: u64,
    positive_count: u64,
}
impl BoolAccumulator {
    fn push(&mut self, value: Option<bool>) {
        if let Some(value) = value {
            self.valid_count += 1;
            self.positive_count += u64::from(value);
        }
    }
    fn finish(&self, denominator: u64) -> BooleanSummary {
        BooleanSummary {
            valid_count: self.valid_count,
            missing_count: denominator - self.valid_count,
            positive_count: self.positive_count,
        }
    }
}
#[derive(Default)]
struct NumericAccumulator {
    valid_count: u64,
    sum: f64,
}
impl NumericAccumulator {
    fn push(&mut self, value: Option<f64>) {
        if let Some(value) = value.filter(|v| v.is_finite() && *v >= 0.0) {
            self.valid_count += 1;
            self.sum += value;
        }
    }
    fn finish(&self, denominator: u64) -> MetricSummary {
        MetricSummary {
            valid_count: self.valid_count,
            missing_count: denominator - self.valid_count,
            mean: (self.valid_count > 0).then(|| self.sum / self.valid_count as f64),
        }
    }
}
fn parse_nonnegative(value: Option<&str>) -> Option<f64> {
    value?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}
fn parse_bool(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "si" | "sí" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}
fn normalize_category(value: Option<&str>) -> ContactCategory {
    match value.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "queja" | "reclamo" | "complaint" => ContactCategory::Complaint,
        "transaccional" | "transactional" | "transaction" => ContactCategory::Transactional,
        "tecnico" | "técnico" | "technical" => ContactCategory::Technical,
        "consulta general" | "consulta_general" | "general inquiry" | "general_inquiry" => {
            ContactCategory::GeneralInquiry
        }
        "producto" | "product" => ContactCategory::Product,
        "cuenta" | "account" => ContactCategory::Account,
        "tarjeta" | "card" => ContactCategory::Card,
        "prestamo" | "préstamo" | "loan" | "credit" => ContactCategory::Loan,
        "otro" | "other" => ContactCategory::Other,
        _ => ContactCategory::Unclassified,
    }
}
fn date_and_period(value: Option<&str>) -> Option<(String, String)> {
    let value = value?;
    parse_utc_timestamp(value)?;
    let date = value.get(..10)?;
    Some((value.to_owned(), date[..7].to_owned()))
}

fn parse_utc_timestamp(value: &str) -> Option<i64> {
    if value.len() != 20 || value.as_bytes().get(10) != Some(&b'T') || !value.ends_with('Z') {
        return None;
    }
    let date = value.get(..10)?;
    let b = date.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    let year = date[..4].parse::<u16>().ok()?;
    let month = date[5..7].parse::<u8>().ok()?;
    let day = date[8..10].parse::<u8>().ok()?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if month == 0 || month > 12 || day == 0 || day > days[month as usize - 1] {
        return None;
    }
    let time = value.get(11..19)?.as_bytes();
    if time[2] != b':' || time[5] != b':' {
        return None;
    }
    let digits = [time[0], time[1], time[3], time[4], time[6], time[7]];
    if !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let hour = value[11..13].parse::<u8>().ok()?;
    let minute = value[14..16].parse::<u8>().ok()?;
    let second = value[17..19].parse::<u8>().ok()?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(
        date_ordinal(date)? * 86_400
            + i64::from(hour) * 3_600
            + i64::from(minute) * 60
            + i64::from(second),
    )
}
fn valid_uuid_v7(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 36
        && [8, 13, 18, 23].iter().all(|i| b[*i] == b'-')
        && b[14] == b'7'
        && matches!(b[19], b'8' | b'9' | b'a' | b'b')
        && b.iter()
            .enumerate()
            .all(|(i, c)| [8, 13, 18, 23].contains(&i) || c.is_ascii_hexdigit())
}
fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}
struct ManifestDigestInput<'a> {
    reference: &'a ArtifactReference,
    snapshot_digest: &'a str,
    table: ProjectionTable,
    source_header_digest: &'a str,
    cutoff: &'a str,
    partitions: &'a BTreeMap<String, String>,
    coverage: ProjectionCoverage,
    policy: ProjectionPolicy,
}

fn manifest_digest(input: ManifestDigestInput<'_>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for field in [
        input.reference.tenant_id.as_str(),
        input.reference.id.as_str(),
        &input.reference.revision.to_string(),
        input.reference.digest.as_str(),
        input.snapshot_digest,
        input.table.as_str(),
        input.source_header_digest,
        input.cutoff,
        if input.coverage == ProjectionCoverage::Complete {
            "complete"
        } else {
            "partial"
        },
        &input.policy.version.to_string(),
        &input.policy.minimum_cell_count.to_string(),
    ] {
        h.update((field.len() as u64).to_be_bytes());
        h.update(field.as_bytes());
    }
    for (id, digest) in input.partitions {
        h.update((id.len() as u64).to_be_bytes());
        h.update(id.as_bytes());
        h.update(digest.as_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}
