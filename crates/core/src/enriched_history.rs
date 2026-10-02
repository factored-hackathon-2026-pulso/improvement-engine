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
use sha2::{Digest, Sha256};

use crate::source_validation::{SourceFileSeal, SourceSnapshot};

const DISCOVERY_FORBIDDEN_TABLES: &[&str] = &["labels", "signal"];
const EVENT_TIME_CLOCK: &str = "event_time";
const INGESTED_AT_CLOCK: &str = "ingested_at";
const LEGACY_MANIFEST_VERSION: u16 = 1;
const CURRENT_MANIFEST_VERSION: u16 = 2;

/// The clock that proves when a row may participate in discovery.
///
/// `ReplayAtEventTime` is deliberately a separate, sealed mode for E0. It
/// means the package has no observed physical ingestion timestamp and replay
/// therefore assumes availability at `event_time` with a zero ingestion lag.
/// It is not evidence about production ingestion latency.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum AvailabilityClockMode {
    #[default]
    ObservedIngestedAt,
    ReplayAtEventTime {
        assumption_label: String,
    },
}

impl AvailabilityClockMode {
    #[must_use]
    pub fn observed_ingested_at() -> Self {
        Self::ObservedIngestedAt
    }

    #[must_use]
    pub fn replay_at_event_time(assumption_label: impl Into<String>) -> Self {
        Self::ReplayAtEventTime {
            assumption_label: assumption_label.into(),
        }
    }

    fn validate(&self) -> Result<(), EnrichedHistoryError> {
        match self {
            Self::ObservedIngestedAt => Ok(()),
            Self::ReplayAtEventTime { assumption_label }
                if is_assumption_label(assumption_label) =>
            {
                Ok(())
            }
            Self::ReplayAtEventTime { .. } => Err(EnrichedHistoryError::InvalidReplayAssumption),
        }
    }

    fn requires_physical_ingested_at(&self) -> bool {
        matches!(self, Self::ObservedIngestedAt)
    }
}

/// Versioned commitment for an availability-clock interpretation. The digest
/// commits to the mode, its assumption label and one immutable source snapshot
/// byte representation; a caller cannot flip the clock after the profile was
/// sealed without invalidating it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilityProfile {
    pub profile_id: String,
    pub profile_version: u16,
    #[serde(default)]
    pub availability_clock: AvailabilityClockMode,
    /// Tenant committed by the source snapshot this profile is permitted to
    /// bind. It is explicit in addition to the snapshot-byte digest so a
    /// cross-tenant substitution is rejected without relying on inference.
    pub source_tenant_id: String,
    pub source_snapshot_digest: String,
    pub profile_digest: String,
}

impl AvailabilityProfile {
    #[must_use]
    pub fn new(
        profile_id: impl Into<String>,
        profile_version: u16,
        availability_clock: AvailabilityClockMode,
        source_tenant_id: impl Into<String>,
        source_snapshot_digest: impl Into<String>,
    ) -> Self {
        let profile_id = profile_id.into();
        let source_tenant_id = source_tenant_id.into();
        let source_snapshot_digest = source_snapshot_digest.into();
        let profile_digest = availability_profile_digest(
            &profile_id,
            profile_version,
            &availability_clock,
            &source_tenant_id,
            &source_snapshot_digest,
        );
        Self {
            profile_id,
            profile_version,
            availability_clock,
            source_tenant_id,
            source_snapshot_digest,
            profile_digest,
        }
    }

    fn validate(&self) -> Result<(), EnrichedHistoryError> {
        if !is_identifier(&self.profile_id)
            || self.profile_version == 0
            || self.source_tenant_id.is_empty()
            || self.source_tenant_id.len() > 128
            || !is_sha256_digest(&self.source_snapshot_digest)
            || !is_sha256_digest(&self.profile_digest)
        {
            return Err(EnrichedHistoryError::InvalidAvailabilityProfile);
        }
        self.availability_clock.validate()?;
        if self.profile_digest
            != availability_profile_digest(
                &self.profile_id,
                self.profile_version,
                &self.availability_clock,
                &self.source_tenant_id,
                &self.source_snapshot_digest,
            )
        {
            return Err(EnrichedHistoryError::InvalidAvailabilityProfile);
        }
        Ok(())
    }
}

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
    /// Digest of the sealed E0 row/field availability projection. It is
    /// required only for `replay_at_event_time`; it prevents a caller from
    /// swapping temporal annotations independently from the data rows.
    #[serde(default)]
    pub replay_projection_digest: Option<String>,
    /// Exact source-file commitment supplied by `SourceSnapshot`. It is
    /// required when the package is opened through `from_snapshot`.
    #[serde(skip)]
    source_file_seal: Option<SourceFileSeal>,
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
            replay_projection_digest: None,
            source_file_seal: None,
        }
    }

    #[must_use]
    pub fn with_field_availability(mut self, field_availability: BTreeMap<String, String>) -> Self {
        self.field_availability = field_availability;
        self
    }

    #[must_use]
    pub fn with_replay_projection_digest(mut self, replay_projection_digest: String) -> Self {
        self.replay_projection_digest = Some(replay_projection_digest);
        self
    }

    #[must_use]
    pub fn with_source_file_seal(mut self, source_file_seal: SourceFileSeal) -> Self {
        self.source_file_seal = Some(source_file_seal);
        self
    }
}

/// Read-only package manifest, independently sealed before the adapter runs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrichedHistoryManifest {
    #[serde(default = "legacy_manifest_version")]
    pub manifest_version: u16,
    pub source_namespace: String,
    pub world_ref: String,
    pub observed_cutoff: String,
    #[serde(default)]
    pub availability_clock: AvailabilityClockMode,
    #[serde(default)]
    pub availability_profile: Option<AvailabilityProfile>,
    pub files: Vec<PackageFile>,
}

impl EnrichedHistoryManifest {
    #[must_use]
    pub fn new(
        source_namespace: impl Into<String>,
        world_ref: impl Into<String>,
        observed_cutoff: impl Into<String>,
        availability_clock: AvailabilityClockMode,
        files: Vec<PackageFile>,
    ) -> Self {
        Self {
            manifest_version: LEGACY_MANIFEST_VERSION,
            source_namespace: source_namespace.into(),
            world_ref: world_ref.into(),
            observed_cutoff: observed_cutoff.into(),
            availability_clock,
            availability_profile: None,
            files,
        }
    }

    #[must_use]
    pub fn new_replay(
        source_namespace: impl Into<String>,
        world_ref: impl Into<String>,
        observed_cutoff: impl Into<String>,
        availability_profile: AvailabilityProfile,
        files: Vec<PackageFile>,
    ) -> Self {
        Self {
            manifest_version: CURRENT_MANIFEST_VERSION,
            source_namespace: source_namespace.into(),
            world_ref: world_ref.into(),
            observed_cutoff: observed_cutoff.into(),
            availability_clock: availability_profile.availability_clock.clone(),
            availability_profile: Some(availability_profile),
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
    replay_row_availability: Option<Vec<ReplayRowAvailability>>,
}

impl TableInput {
    #[must_use]
    pub fn new(digests: ProvenanceDigests, rows: Vec<Value>) -> Self {
        Self {
            digests,
            rows,
            replay_row_availability: None,
        }
    }

    #[must_use]
    pub fn with_replay_row_availability(
        mut self,
        replay_row_availability: Vec<ReplayRowAvailability>,
    ) -> Self {
        self.replay_row_availability = Some(replay_row_availability);
        self
    }
}

/// Sealed availability annotations for one E0 replay row. They stay outside
/// the discovery projection and are never exposed as customer attributes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayRowAvailability {
    pub fields: BTreeMap<String, String>,
}

impl ReplayRowAvailability {
    #[must_use]
    pub fn new(fields: BTreeMap<String, String>) -> Self {
        Self { fields }
    }
}

/// Canonical digest of a replay projection and its per-row availability seals.
#[must_use]
pub fn replay_projection_digest(rows: &[Value], availability: &[ReplayRowAvailability]) -> String {
    let mut canonical = String::new();
    canonical.push('[');
    for (index, (row, row_availability)) in rows.iter().zip(availability).enumerate() {
        if index > 0 {
            canonical.push(',');
        }
        canonical_json(row, &mut canonical);
        canonical.push('|');
        canonical.push('{');
        for (field_index, (field, available_at)) in row_availability.fields.iter().enumerate() {
            if field_index > 0 {
                canonical.push(',');
            }
            canonical_json(&Value::String(field.clone()), &mut canonical);
            canonical.push(':');
            canonical_json(&Value::String(available_at.clone()), &mut canonical);
        }
        canonical.push('}');
    }
    canonical.push(']');
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnrichedHistoryProvenance {
    pub source_namespace: String,
    pub world_ref: String,
    pub observed_cutoff: String,
    pub availability_clock: AvailabilityClockMode,
    pub availability_profile: Option<AvailabilityProfile>,
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
    InvalidReplayAssumption,
    UnsupportedManifestVersion {
        manifest_version: u16,
    },
    MissingAvailabilityProfile,
    InvalidAvailabilityProfile,
    SnapshotAvailabilityProfileMismatch,
    /// Legacy manifests have no immutable snapshot digest, so binding one to a
    /// snapshot would leave its tenant unsealed. They remain readable through
    /// `from_manifest`, but must not enter a snapshot-bound adapter.
    SnapshotBindingUnavailable {
        manifest_version: u16,
    },
    ReplaySnapshotBindingRequired,
    SnapshotSourceNotListed {
        table: String,
    },
    MissingSourceFileSeal {
        table: String,
    },
    SourceFileSealMismatch {
        table: String,
    },
    MissingReplayProjectionDigest {
        table: String,
    },
    UnexpectedReplayProjectionDigest {
        table: String,
    },
    ReplayProjectionDigestMismatch {
        table: String,
    },
    MissingReplayRowAvailability {
        table: String,
    },
    ReplayAvailabilityRowCountMismatch {
        table: String,
    },
    ReplayAvailabilityFieldMismatch {
        table: String,
        field: String,
    },
    InvalidReplayFieldAvailability {
        table: String,
        field: String,
    },
    FutureFieldAtEvent {
        table: String,
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
    UnexpectedAvailabilityClock {
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
    availability_clock: AvailabilityClockMode,
    availability_profile: Option<AvailabilityProfile>,
    files: BTreeMap<String, PackageFile>,
}

/// Opaque, crate-private proof that a replay package remains bound to the
/// exact U04-B source snapshot from which it was opened.  It exposes only the
/// temporal/scope commitments needed by a later trusted composition; it never
/// exposes source rows or permits a caller-provided clock.
#[allow(dead_code)] // Consumed by the future trusted U04-B/U23 composition root.
pub(crate) struct VerifiedReplayAvailability {
    tenant_id: String,
    world_ref: String,
    cutoff_at_unix_seconds: u64,
    source_snapshot_digest: String,
    availability_profile_digest: String,
}

#[allow(dead_code)] // Consumed by the future trusted U04-B/U23 composition root.
impl VerifiedReplayAvailability {
    #[must_use]
    pub(crate) fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    #[must_use]
    pub(crate) fn world_ref(&self) -> &str {
        &self.world_ref
    }

    #[must_use]
    pub(crate) fn cutoff_at_unix_seconds(&self) -> u64 {
        self.cutoff_at_unix_seconds
    }

    #[must_use]
    pub(crate) fn source_snapshot_digest(&self) -> &str {
        &self.source_snapshot_digest
    }

    #[must_use]
    pub(crate) fn availability_profile_digest(&self) -> &str {
        &self.availability_profile_digest
    }
}

impl EnrichedHistoryAdapter {
    /// Validates a manifest before any rows are considered.
    pub fn from_manifest(manifest: EnrichedHistoryManifest) -> Result<Self, EnrichedHistoryError> {
        Self::from_manifest_bound(manifest, None)
    }

    fn from_manifest_bound(
        manifest: EnrichedHistoryManifest,
        snapshot_binding_digest: Option<&str>,
    ) -> Result<Self, EnrichedHistoryError> {
        validate_required_string(&manifest.source_namespace, "source_namespace")?;
        validate_required_string(&manifest.world_ref, "world_ref")?;
        validate_timestamp(&manifest.observed_cutoff, "observed_cutoff")?;
        manifest.availability_clock.validate()?;
        match manifest.manifest_version {
            LEGACY_MANIFEST_VERSION => {
                if !manifest.availability_clock.requires_physical_ingested_at()
                    || manifest.availability_profile.is_some()
                {
                    return Err(EnrichedHistoryError::UnsupportedManifestVersion {
                        manifest_version: manifest.manifest_version,
                    });
                }
            }
            CURRENT_MANIFEST_VERSION => {
                let profile = manifest
                    .availability_profile
                    .as_ref()
                    .ok_or(EnrichedHistoryError::MissingAvailabilityProfile)?;
                profile.validate()?;
                if profile.availability_clock != manifest.availability_clock {
                    return Err(EnrichedHistoryError::InvalidAvailabilityProfile);
                }
                if snapshot_binding_digest != Some(profile.source_snapshot_digest.as_str()) {
                    return Err(EnrichedHistoryError::ReplaySnapshotBindingRequired);
                }
            }
            _ => {
                return Err(EnrichedHistoryError::UnsupportedManifestVersion {
                    manifest_version: manifest.manifest_version,
                });
            }
        }
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
            if !manifest.availability_clock.requires_physical_ingested_at()
                && file.field_availability.contains_key(INGESTED_AT_CLOCK)
            {
                return Err(EnrichedHistoryError::UnexpectedAvailabilityClock {
                    table: file.table,
                    field: INGESTED_AT_CLOCK.to_owned(),
                });
            }
            match (
                manifest.availability_clock.requires_physical_ingested_at(),
                &file.replay_projection_digest,
            ) {
                (false, None) => {
                    return Err(EnrichedHistoryError::MissingReplayProjectionDigest {
                        table: file.table,
                    });
                }
                (false, Some(digest)) if !is_sha256_digest(digest) => {
                    return Err(EnrichedHistoryError::InvalidManifestField {
                        field: format!("files.{}.replay_projection_digest", file.table),
                    });
                }
                (true, Some(_)) => {
                    return Err(EnrichedHistoryError::UnexpectedReplayProjectionDigest {
                        table: file.table,
                    });
                }
                _ => {}
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
            availability_clock: manifest.availability_clock,
            availability_profile: manifest.availability_profile,
            files,
        })
    }

    /// Binds enriched history metadata to an existing immutable source snapshot.
    /// It prevents a package from silently substituting a world or a cutoff.
    pub fn from_snapshot(
        manifest: EnrichedHistoryManifest,
        snapshot: &SourceSnapshot,
    ) -> Result<Self, EnrichedHistoryError> {
        if !snapshot.has_canonical_binding() {
            return Err(EnrichedHistoryError::SnapshotAvailabilityProfileMismatch);
        }
        if manifest.manifest_version == LEGACY_MANIFEST_VERSION {
            return Err(EnrichedHistoryError::SnapshotBindingUnavailable {
                manifest_version: manifest.manifest_version,
            });
        }
        let source = snapshot.provenance();
        if manifest.source_namespace != source.source_namespace
            || manifest.world_ref != source.world_ref
            || manifest.observed_cutoff != source.observed_cutoff
        {
            return Err(EnrichedHistoryError::SnapshotProvenanceMismatch);
        }
        if manifest.manifest_version == CURRENT_MANIFEST_VERSION {
            let Some(profile) = manifest.availability_profile.as_ref() else {
                return Err(EnrichedHistoryError::SnapshotAvailabilityProfileMismatch);
            };
            if profile.source_tenant_id != snapshot.tenant_id()
                || profile.source_snapshot_digest != snapshot.binding_digest()
            {
                return Err(EnrichedHistoryError::SnapshotAvailabilityProfileMismatch);
            }
        }
        for file in &manifest.files {
            let snapshot_seal = snapshot.source_file_seal(&file.table).ok_or_else(|| {
                EnrichedHistoryError::SnapshotSourceNotListed {
                    table: file.table.clone(),
                }
            })?;
            let package_seal = file.source_file_seal.as_ref().ok_or_else(|| {
                EnrichedHistoryError::MissingSourceFileSeal {
                    table: file.table.clone(),
                }
            })?;
            if package_seal != &snapshot_seal
                || file.digests.file_digest != snapshot_seal.file_digest()
            {
                return Err(EnrichedHistoryError::SourceFileSealMismatch {
                    table: file.table.clone(),
                });
            }
        }
        Self::from_manifest_bound(manifest, Some(&snapshot.binding_digest()))
    }

    /// Emits the only non-test temporal input accepted by U23-P.  The caller
    /// cannot pick its tenant, world, cutoff, source digest or clock: all five
    /// values are revalidated against the V2 U04-B replay profile and exact
    /// parsed `SourceSnapshot` before this opaque projection is returned.
    #[allow(dead_code)] // Consumed by the future trusted U04-B/U23 composition root.
    pub(crate) fn verified_replay_availability(
        &self,
        snapshot: &SourceSnapshot,
    ) -> Result<VerifiedReplayAvailability, EnrichedHistoryError> {
        if !snapshot.has_canonical_binding() {
            return Err(EnrichedHistoryError::SnapshotAvailabilityProfileMismatch);
        }
        let Some(profile) = self.availability_profile.as_ref() else {
            return Err(EnrichedHistoryError::MissingAvailabilityProfile);
        };
        if !matches!(
            self.availability_clock,
            AvailabilityClockMode::ReplayAtEventTime { .. }
        ) || profile.availability_clock != self.availability_clock
            || profile.source_tenant_id != snapshot.tenant_id()
            || profile.source_snapshot_digest != snapshot.binding_digest()
            || self.source_namespace != snapshot.source_namespace()
            || self.world_ref != snapshot.world_ref()
            || self.observed_cutoff != snapshot.observed_cutoff()
        {
            return Err(EnrichedHistoryError::SnapshotAvailabilityProfileMismatch);
        }
        let cutoff_at_unix_seconds = rfc3339_utc_to_unix_seconds(&self.observed_cutoff)
            .ok_or_else(|| EnrichedHistoryError::InvalidManifestField {
                field: "observed_cutoff".to_owned(),
            })?;
        Ok(VerifiedReplayAvailability {
            tenant_id: snapshot.tenant_id().to_owned(),
            world_ref: snapshot.world_ref().to_owned(),
            cutoff_at_unix_seconds,
            source_snapshot_digest: snapshot.binding_digest(),
            availability_profile_digest: profile.profile_digest.clone(),
        })
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
        let replay_availability =
            validate_replay_projection(table, &input, sealed, &self.availability_clock)?;
        for (index, row) in input.rows.iter().enumerate() {
            validate_discovery_row(
                table,
                row,
                &self.observed_cutoff,
                &self.availability_clock,
                &sealed.field_availability,
                replay_availability.map(|values| &values[index]),
            )?;
        }

        Ok(DiscoveryTable {
            table: table.to_owned(),
            provenance: EnrichedHistoryProvenance {
                source_namespace: self.source_namespace.clone(),
                world_ref: self.world_ref.clone(),
                observed_cutoff: self.observed_cutoff.clone(),
                availability_clock: self.availability_clock.clone(),
                availability_profile: self.availability_profile.clone(),
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
    availability_clock: &AvailabilityClockMode,
    field_availability: &BTreeMap<String, String>,
    replay_availability: Option<&ReplayRowAvailability>,
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
    if !availability_clock.requires_physical_ingested_at() && object.contains_key(INGESTED_AT_CLOCK)
    {
        return Err(EnrichedHistoryError::UnexpectedAvailabilityClock {
            table: table.to_owned(),
            field: INGESTED_AT_CLOCK.to_owned(),
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
    let required_clocks: &[&str] = if availability_clock.requires_physical_ingested_at() {
        &[EVENT_TIME_CLOCK, INGESTED_AT_CLOCK]
    } else {
        &[EVENT_TIME_CLOCK]
    };
    for field in required_clocks {
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
    if let Some(replay_availability) = replay_availability {
        let event_time = object
            .get(EVENT_TIME_CLOCK)
            .and_then(Value::as_str)
            .expect("replay event_time is validated before row availability");
        for field in object.keys() {
            let available_at = replay_availability.fields.get(field).ok_or_else(|| {
                EnrichedHistoryError::ReplayAvailabilityFieldMismatch {
                    table: table.to_owned(),
                    field: field.clone(),
                }
            })?;
            if !is_rfc3339_utc(available_at) {
                return Err(EnrichedHistoryError::InvalidReplayFieldAvailability {
                    table: table.to_owned(),
                    field: field.clone(),
                });
            }
            if available_at.as_str() > event_time {
                return Err(EnrichedHistoryError::FutureFieldAtEvent {
                    table: table.to_owned(),
                    field: field.clone(),
                });
            }
        }
        for field in replay_availability.fields.keys() {
            if !object.contains_key(field) {
                return Err(EnrichedHistoryError::ReplayAvailabilityFieldMismatch {
                    table: table.to_owned(),
                    field: field.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_replay_projection<'a>(
    table: &str,
    input: &'a TableInput,
    sealed: &PackageFile,
    availability_clock: &AvailabilityClockMode,
) -> Result<Option<&'a [ReplayRowAvailability]>, EnrichedHistoryError> {
    if availability_clock.requires_physical_ingested_at() {
        return Ok(None);
    }
    let availability = input.replay_row_availability.as_deref().ok_or_else(|| {
        EnrichedHistoryError::MissingReplayRowAvailability {
            table: table.to_owned(),
        }
    })?;
    if availability.len() != input.rows.len() {
        return Err(EnrichedHistoryError::ReplayAvailabilityRowCountMismatch {
            table: table.to_owned(),
        });
    }
    let actual_digest = replay_projection_digest(&input.rows, availability);
    if sealed.replay_projection_digest.as_deref() != Some(actual_digest.as_str()) {
        return Err(EnrichedHistoryError::ReplayProjectionDigestMismatch {
            table: table.to_owned(),
        });
    }
    Ok(Some(availability))
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

/// Converts the fixed-width, already validated UTC timestamp to epoch seconds
/// without accepting a caller-provided replay clock. The civil-date algorithm
/// is proleptic Gregorian and intentionally has no timezone/dependency input.
#[allow(dead_code)] // Reached through the future trusted U04-B/U23 composition root.
fn rfc3339_utc_to_unix_seconds(value: &str) -> Option<u64> {
    if !is_rfc3339_utc(value) {
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

fn is_assumption_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn legacy_manifest_version() -> u16 {
    LEGACY_MANIFEST_VERSION
}

fn availability_profile_digest(
    profile_id: &str,
    profile_version: u16,
    availability_clock: &AvailabilityClockMode,
    source_tenant_id: &str,
    source_snapshot_digest: &str,
) -> String {
    let clock = match availability_clock {
        AvailabilityClockMode::ObservedIngestedAt => "observed_ingested_at".to_owned(),
        AvailabilityClockMode::ReplayAtEventTime { assumption_label } => {
            format!("replay_at_event_time:{assumption_label}")
        }
    };
    format!(
        "sha256:{:x}",
        Sha256::digest(
            format!(
                "{profile_id}|{profile_version}|{clock}|{source_tenant_id}|{source_snapshot_digest}"
            )
            .as_bytes()
        )
    )
}

fn canonical_json(value: &Value, output: &mut String) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            output.push_str(&serde_json::to_string(value).expect("JSON value serializes"));
        }
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                canonical_json(value, output);
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                canonical_json(&Value::String(key.clone()), output);
                output.push(':');
                canonical_json(&values[key], output);
            }
            output.push('}');
        }
    }
}
