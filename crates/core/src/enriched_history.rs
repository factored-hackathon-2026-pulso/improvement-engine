//! Governed, read-only projection of an enriched E0 history package.
//!
//! The adapter deliberately receives parsed metadata and rows from an approved
//! caller. It does not open parquet files, discover a local directory, send
//! records outside the process, or synthesize signals from precomputed labels.
//! That separation makes the access boundary explicit while still making the
//! temporal and provenance controls executable in local tests.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::source_validation::SourceSnapshot;

const DISCOVERY_FORBIDDEN_TABLES: &[&str] = &["labels", "signal"];
const REQUIRED_AVAILABILITY_CLOCKS: &[&str] = &["event_time", "ingested_at"];

/// Four independent seals carried by every enriched package file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceDigests {
    pub file_digest: String,
    pub schema_digest: String,
    pub transform_digest: String,
    pub policy_digest: String,
}

impl ProvenanceDigests {
    #[must_use]
    pub fn new(
        file_digest: impl Into<String>,
        schema_digest: impl Into<String>,
        transform_digest: impl Into<String>,
        policy_digest: impl Into<String>,
    ) -> Self {
        Self {
            file_digest: file_digest.into(),
            schema_digest: schema_digest.into(),
            transform_digest: transform_digest.into(),
            policy_digest: policy_digest.into(),
        }
    }

    fn validate(&self) -> bool {
        [
            &self.file_digest,
            &self.schema_digest,
            &self.transform_digest,
            &self.policy_digest,
        ]
        .into_iter()
        .all(|digest| is_sha256_digest(digest))
    }
}

/// One sealed file in an enriched history package.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub table: String,
    pub digests: ProvenanceDigests,
    /// Latest instant at which this file was available to a discovery run.
    pub available_at: String,
    /// Availability is explicit per top-level field/group. A group can contain
    /// nested JSON, but it receives the same immutable availability decision.
    #[serde(default)]
    pub field_availability: BTreeMap<String, String>,
}

impl PackageFile {
    #[must_use]
    pub fn new(
        table: impl Into<String>,
        digests: ProvenanceDigests,
        available_at: impl Into<String>,
    ) -> Self {
        Self {
            table: table.into(),
            digests,
            available_at: available_at.into(),
            field_availability: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_field_availability(mut self, field_availability: BTreeMap<String, String>) -> Self {
        self.field_availability = field_availability;
        self
    }
}

/// Read-only package manifest, independently sealed before the adapter runs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrichedHistoryManifest {
    pub source_namespace: String,
    pub world_ref: String,
    pub observed_cutoff: String,
    pub files: Vec<PackageFile>,
}

impl EnrichedHistoryManifest {
    #[must_use]
    pub fn new(
        source_namespace: impl Into<String>,
        world_ref: impl Into<String>,
        observed_cutoff: impl Into<String>,
        files: Vec<PackageFile>,
    ) -> Self {
        Self {
            source_namespace: source_namespace.into(),
            world_ref: world_ref.into(),
            observed_cutoff: observed_cutoff.into(),
            files,
        }
    }

    pub fn from_json(raw: &str) -> Result<Self, EnrichedHistoryError> {
        serde_json::from_str(raw).map_err(|_| EnrichedHistoryError::MalformedManifest)
    }
}

/// Adapter-provided rows and their observed file metadata. The adapter owns no
/// filesystem access and cannot mutate source data through this type.
#[derive(Clone, Debug, PartialEq)]
pub struct TableInput {
    pub digests: ProvenanceDigests,
    pub rows: Vec<Value>,
}

impl TableInput {
    #[must_use]
    pub fn new(digests: ProvenanceDigests, rows: Vec<Value>) -> Self {
        Self { digests, rows }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnrichedHistoryProvenance {
    pub source_namespace: String,
    pub world_ref: String,
    pub observed_cutoff: String,
    pub digests: ProvenanceDigests,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QualityFindingKind {
    FileDigestMismatch,
    SchemaDigestMismatch,
    TransformDigestMismatch,
    PolicyDigestMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QualityFinding {
    pub kind: QualityFindingKind,
    pub table: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryTable {
    pub table: String,
    pub provenance: EnrichedHistoryProvenance,
    pub rows: Vec<Value>,
    pub findings: Vec<QualityFinding>,
    /// A provenance mismatch is visible for data quality work but never a
    /// trustworthy input for downstream discovery.
    pub eligible_for_discovery: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnrichedHistoryError {
    MalformedManifest,
    DuplicateTable {
        table: String,
    },
    InvalidManifestField {
        field: String,
    },
    UnavailableFile {
        table: String,
    },
    SnapshotProvenanceMismatch,
    UnknownTable {
        table: String,
    },
    ForbiddenDiscoveryTable {
        table: String,
    },
    ForbiddenDiscoveryField {
        table: String,
        field: String,
    },
    RowIsNotObject {
        table: String,
    },
    MissingAvailabilityClock {
        table: String,
        field: String,
    },
    InvalidAvailabilityClock {
        table: String,
        field: String,
    },
    FutureData {
        table: String,
        field: String,
    },
    UnavailableField {
        table: String,
        field: String,
    },
    QualityBlocked {
        table: String,
        findings: Vec<QualityFinding>,
    },
}

impl fmt::Display for EnrichedHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for EnrichedHistoryError {}

/// The retained, sealed package view. It only exposes discovery-safe rows.
#[derive(Debug)]
pub struct EnrichedHistoryAdapter {
    source_namespace: String,
    world_ref: String,
    observed_cutoff: String,
    files: BTreeMap<String, PackageFile>,
}

impl EnrichedHistoryAdapter {
    /// Validates a manifest before any rows are considered.
    pub fn from_manifest(manifest: EnrichedHistoryManifest) -> Result<Self, EnrichedHistoryError> {
        validate_required_string(&manifest.source_namespace, "source_namespace")?;
        validate_required_string(&manifest.world_ref, "world_ref")?;
        validate_timestamp(&manifest.observed_cutoff, "observed_cutoff")?;
        if manifest.files.is_empty() {
            return Err(EnrichedHistoryError::InvalidManifestField {
                field: "files".to_owned(),
            });
        }

        let mut files = BTreeMap::new();
        for file in manifest.files {
            validate_required_string(&file.table, "files.table")?;
            if !is_identifier(&file.table) || !file.digests.validate() {
                return Err(EnrichedHistoryError::InvalidManifestField {
                    field: format!("files.{}", file.table),
                });
            }
            validate_timestamp(&file.available_at, "files.available_at")?;
            if file.available_at > manifest.observed_cutoff {
                return Err(EnrichedHistoryError::UnavailableFile { table: file.table });
            }
            if file.field_availability.is_empty() {
                return Err(EnrichedHistoryError::InvalidManifestField {
                    field: format!("files.{}.field_availability", file.table),
                });
            }
            for (field, available_at) in &file.field_availability {
                if !is_identifier(field) {
                    return Err(EnrichedHistoryError::InvalidManifestField {
                        field: format!("files.{}.field_availability", file.table),
                    });
                }
                validate_timestamp(available_at, "files.field_availability")?;
                if available_at > &manifest.observed_cutoff {
                    return Err(EnrichedHistoryError::UnavailableField {
                        table: file.table.clone(),
                        field: field.clone(),
                    });
                }
            }
            let table = file.table.clone();
            if files.insert(table.clone(), file).is_some() {
                return Err(EnrichedHistoryError::DuplicateTable { table });
            }
        }

        Ok(Self {
            source_namespace: manifest.source_namespace,
            world_ref: manifest.world_ref,
            observed_cutoff: manifest.observed_cutoff,
            files,
        })
    }

    /// Binds enriched history metadata to an existing immutable source snapshot.
    /// It prevents a package from silently substituting a world or a cutoff.
    pub fn from_snapshot(
        manifest: EnrichedHistoryManifest,
        snapshot: &SourceSnapshot,
    ) -> Result<Self, EnrichedHistoryError> {
        let source = snapshot.provenance();
        if manifest.source_namespace != source.source_namespace
            || manifest.world_ref != source.world_ref
            || manifest.observed_cutoff != source.observed_cutoff
        {
            return Err(EnrichedHistoryError::SnapshotProvenanceMismatch);
        }
        Self::from_manifest(manifest)
    }

    /// Returns a deterministic, discovery-safe projection for exactly one table.
    pub fn discovery_table(
        &self,
        table: &str,
        input: TableInput,
    ) -> Result<DiscoveryTable, EnrichedHistoryError> {
        if DISCOVERY_FORBIDDEN_TABLES.contains(&table) {
            return Err(EnrichedHistoryError::ForbiddenDiscoveryTable {
                table: table.to_owned(),
            });
        }
        let sealed = self
            .files
            .get(table)
            .ok_or_else(|| EnrichedHistoryError::UnknownTable {
                table: table.to_owned(),
            })?;
        let findings = provenance_findings(table, &sealed.digests, &input.digests);
        if !findings.is_empty() {
            return Err(EnrichedHistoryError::QualityBlocked {
                table: table.to_owned(),
                findings,
            });
        }
        for row in &input.rows {
            validate_discovery_row(
                table,
                row,
                &self.observed_cutoff,
                &sealed.field_availability,
            )?;
        }

        Ok(DiscoveryTable {
            table: table.to_owned(),
            provenance: EnrichedHistoryProvenance {
                source_namespace: self.source_namespace.clone(),
                world_ref: self.world_ref.clone(),
                observed_cutoff: self.observed_cutoff.clone(),
                digests: sealed.digests.clone(),
            },
            rows: input.rows,
            eligible_for_discovery: true,
            findings,
        })
    }

    /// Processes tables in `BTreeMap` order, independent of caller insertion order.
    pub fn discovery_tables(
        &self,
        inputs: BTreeMap<String, TableInput>,
    ) -> Result<Vec<DiscoveryTable>, EnrichedHistoryError> {
        inputs
            .into_iter()
            .map(|(table, input)| self.discovery_table(&table, input))
            .collect()
    }
}

fn provenance_findings(
    table: &str,
    sealed: &ProvenanceDigests,
    observed: &ProvenanceDigests,
) -> Vec<QualityFinding> {
    [
        (
            QualityFindingKind::FileDigestMismatch,
            sealed.file_digest != observed.file_digest,
        ),
        (
            QualityFindingKind::SchemaDigestMismatch,
            sealed.schema_digest != observed.schema_digest,
        ),
        (
            QualityFindingKind::TransformDigestMismatch,
            sealed.transform_digest != observed.transform_digest,
        ),
        (
            QualityFindingKind::PolicyDigestMismatch,
            sealed.policy_digest != observed.policy_digest,
        ),
    ]
    .into_iter()
    .filter_map(|(kind, mismatch)| {
        mismatch.then(|| QualityFinding {
            kind,
            table: table.to_owned(),
        })
    })
    .collect()
}

fn validate_discovery_row(
    table: &str,
    row: &Value,
    cutoff: &str,
    field_availability: &BTreeMap<String, String>,
) -> Result<(), EnrichedHistoryError> {
    let object = row
        .as_object()
        .ok_or_else(|| EnrichedHistoryError::RowIsNotObject {
            table: table.to_owned(),
        })?;
    let forbidden = find_forbidden_field(object);
    if let Some(field) = forbidden {
        return Err(EnrichedHistoryError::ForbiddenDiscoveryField {
            table: table.to_owned(),
            field,
        });
    }
    for field in object.keys() {
        if !field_availability.contains_key(field) {
            return Err(EnrichedHistoryError::UnavailableField {
                table: table.to_owned(),
                field: field.clone(),
            });
        }
    }
    for field in REQUIRED_AVAILABILITY_CLOCKS {
        let value = object.get(*field).and_then(Value::as_str).ok_or_else(|| {
            EnrichedHistoryError::MissingAvailabilityClock {
                table: table.to_owned(),
                field: (*field).to_owned(),
            }
        })?;
        if !is_rfc3339_utc(value) {
            return Err(EnrichedHistoryError::InvalidAvailabilityClock {
                table: table.to_owned(),
                field: (*field).to_owned(),
            });
        }
        if value > cutoff {
            return Err(EnrichedHistoryError::FutureData {
                table: table.to_owned(),
                field: (*field).to_owned(),
            });
        }
    }
    Ok(())
}

fn find_forbidden_field(object: &serde_json::Map<String, Value>) -> Option<String> {
    object.iter().find_map(|(key, value)| {
        if is_forbidden_field(key) {
            return Some(key.clone());
        }
        match value {
            Value::Object(child) => find_forbidden_field(child),
            Value::Array(children) => children.iter().find_map(find_forbidden_value),
            _ => None,
        }
    })
}

fn find_forbidden_value(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => find_forbidden_field(object),
        Value::Array(values) => values.iter().find_map(find_forbidden_value),
        _ => None,
    }
}

fn is_forbidden_field(field: &str) -> bool {
    let normalized = field
        .chars()
        .filter_map(|character| {
            character
                .is_ascii_alphanumeric()
                .then_some(character.to_ascii_lowercase())
        })
        .collect::<String>();
    normalized.contains("label")
        || normalized.contains("signal")
        || normalized.starts_with("final")
        || normalized.starts_with("expected")
        || matches!(normalized.as_str(), "humanerrors" | "stresstest")
}

fn validate_required_string(value: &str, field: &str) -> Result<(), EnrichedHistoryError> {
    if value.is_empty() {
        Err(EnrichedHistoryError::InvalidManifestField {
            field: field.to_owned(),
        })
    } else {
        Ok(())
    }
}

fn validate_timestamp(value: &str, field: &str) -> Result<(), EnrichedHistoryError> {
    if is_rfc3339_utc(value) {
        Ok(())
    } else {
        Err(EnrichedHistoryError::InvalidManifestField {
            field: field.to_owned(),
        })
    }
}

// Fixed-width UTC timestamps are intentionally required. Their lexical order is
// chronological, so cutoff comparison remains dependency-free and deterministic.
fn is_rfc3339_utc(value: &str) -> bool {
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
        return false;
    }
    let number = |start: usize, end: usize| value[start..end].parse::<u32>().ok();
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0, 4),
        number(5, 7),
        number(8, 10),
        number(11, 13),
        number(14, 16),
        number(17, 19),
    ) else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days_in_month).contains(&day)
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(character) if character.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}
