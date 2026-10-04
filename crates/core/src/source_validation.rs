//! Read-only validation at the boundary between a governed source and the engine.
//!
//! This module intentionally accepts bytes supplied by an adapter. It neither opens
//! a bank dataset nor sends it outside the process. A future adapter (E01) owns the
//! authenticated source connection and passes only an approved snapshot into this
//! deterministic verifier.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceContract {
    contract_version: ContractVersion,
    pub source_namespace: String,
    source_kind: String,
    pub table: String,
    primary_key: Vec<String>,
    event_clock: String,
    read_only: bool,
    #[serde(rename = "access_policy")]
    access_policy: AccessPolicy,
    columns: Vec<SourceColumn>,
    #[serde(skip)]
    raw_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractVersion {
    major: u64,
    minor: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessPolicy {
    permitted_classifications: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceColumn {
    name: String,
    logical_type: String,
    nullable: bool,
    purpose: String,
    classification: String,
}

impl SourceContract {
    /// Parses the canonical JSON document and remembers the digest of its exact
    /// wire representation, which is what `SourceSnapshot` pins.
    pub fn from_json(raw: &str) -> Result<Self, SourceDefinitionError> {
        let mut contract: Self = serde_json::from_str(raw).map_err(SourceDefinitionError::Json)?;
        contract.validate()?;
        contract.raw_digest = sha256(raw.as_bytes());
        Ok(contract)
    }

    fn validate(&self) -> Result<(), SourceDefinitionError> {
        validate_version(&self.contract_version)?;
        if self.source_kind != "original" || !self.read_only {
            return Err(SourceDefinitionError::Invalid(
                "source contract must be original and readonly",
            ));
        }
        validate_non_empty(&self.source_namespace, "source namespace")?;
        validate_identifier(&self.table, "table")?;
        if self.primary_key.is_empty() || self.primary_key.iter().any(|key| key.is_empty()) {
            return Err(SourceDefinitionError::Invalid(
                "primary key must contain non-empty values",
            ));
        }
        if self
            .primary_key
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != self.primary_key.len()
        {
            return Err(SourceDefinitionError::Invalid(
                "primary key values must be unique",
            ));
        }
        validate_non_empty(&self.event_clock, "event clock")?;
        if self.access_policy.permitted_classifications.is_empty()
            || self
                .access_policy
                .permitted_classifications
                .iter()
                .any(|classification| !is_permitted_classification(classification))
            || self
                .access_policy
                .permitted_classifications
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != self.access_policy.permitted_classifications.len()
        {
            return Err(SourceDefinitionError::Invalid(
                "access policy classifications are invalid",
            ));
        }
        if self.columns.is_empty() {
            return Err(SourceDefinitionError::Invalid(
                "source contract needs columns",
            ));
        }
        for column in &self.columns {
            validate_non_empty(&column.name, "column name")?;
            if !matches!(
                column.logical_type.as_str(),
                "text" | "timestamp" | "date" | "bool" | "decimal" | "int"
            ) || !matches!(
                column.purpose.as_str(),
                "identity" | "join" | "event_clock" | "metric" | "dimension" | "quality"
            ) || !is_classification(&column.classification)
            {
                return Err(SourceDefinitionError::Invalid("column enum is invalid"));
            }
            let _ = column.nullable;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum SourceDefinitionError {
    Json(serde_json::Error),
    Invalid(&'static str),
}

impl fmt::Display for SourceDefinitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(source) => write!(formatter, "invalid source definition JSON: {source}"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for SourceDefinitionError {}

#[derive(Debug)]
pub enum SourceContractLoadError {
    ReadDirectory(std::io::Error),
    ReadFile {
        path: String,
        source: std::io::Error,
    },
    Parse {
        path: String,
        source: SourceDefinitionError,
    },
}

impl fmt::Display for SourceContractLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadDirectory(source) => {
                write!(formatter, "cannot read source contract directory: {source}")
            }
            Self::ReadFile { path, source } => {
                write!(formatter, "cannot read source contract {path}: {source}")
            }
            Self::Parse { path, source } => {
                write!(formatter, "cannot parse source contract {path}: {source}")
            }
        }
    }
}

impl std::error::Error for SourceContractLoadError {}

/// Reads the canonical JSON contracts in filename order. A caller selects the
/// directory; this function does not discover or access bank source data.
pub fn load_canonical_contracts(
    directory: &Path,
) -> Result<Vec<SourceContract>, SourceContractLoadError> {
    let entries = fs::read_dir(directory).map_err(SourceContractLoadError::ReadDirectory)?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(SourceContractLoadError::ReadDirectory)?
            .path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let path_display = path.display().to_string();
            let raw =
                fs::read_to_string(&path).map_err(|source| SourceContractLoadError::ReadFile {
                    path: path_display.clone(),
                    source,
                })?;
            SourceContract::from_json(&raw).map_err(|source| SourceContractLoadError::Parse {
                path: path_display,
                source,
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshot {
    contract_version: ContractVersion,
    tenant_id: String,
    source_namespace: String,
    world_ref: String,
    observed_cutoff: String,
    sources: Vec<SnapshotSource>,
    #[serde(skip)]
    raw_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotSource {
    table: String,
    uri: String,
    file_digest: String,
    #[serde(default, deserialize_with = "deserialize_partition_inventory_digest")]
    partition_inventory_digest: Option<String>,
    header_digest: String,
    row_count: u64,
    source_contract_ref: SourceContractRef,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceContractRef {
    id: String,
    version: String,
    digest: String,
}

fn deserialize_partition_inventory_digest<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

/// Read-only seal for one exact file entry in an immutable source snapshot.
/// It intentionally has no public constructor: callers obtain it only from the
/// parsed `SourceSnapshot` and can use it to bind a downstream projection back
/// to its table, object, header and source-contract commitments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFileSeal {
    table: String,
    uri: String,
    file_digest: String,
    partition_inventory_digest: Option<String>,
    header_digest: String,
    source_contract_id: String,
    source_contract_version: String,
    source_contract_digest: String,
}

/// Opaque mapping emitted only by the immutable source-artifact registry after
/// it reparses the stored snapshot bytes. It deliberately carries both digest
/// domains, rather than pretending they are interchangeable.
#[cfg(feature = "local-simulation")]
pub struct VerifiedSourceArtifactBinding {
    artifact_ref: crate::ArtifactReference,
    snapshot_binding_digest: String,
}

impl VerifiedSourceArtifactBinding {
    pub(crate) fn artifact_ref(&self) -> &crate::ArtifactReference {
        &self.artifact_ref
    }

    pub(crate) fn snapshot_binding_digest(&self) -> &str {
        &self.snapshot_binding_digest
    }

    #[cfg(test)]
    pub(crate) fn deterministic_for_test(
        artifact_ref: crate::ArtifactReference,
        snapshot_binding_digest: String,
    ) -> Self {
        Self {
            artifact_ref,
            snapshot_binding_digest,
        }
    }
}

/// Resolves a U04 mapping only from U02's immutable revision port. The exact
/// reference, kind and stored content digest are rechecked before the exact
/// raw snapshot JSON bytes retained under `source_snapshot_json` in the
/// immutable artifact are parsed again. This is the sole U02 payload key for
/// exact `SourceSnapshot` bytes; parsing a JSON value and serializing it again
/// would create a different U04 binding domain.
#[allow(dead_code)] // Called by the future U04/U08 composition root.
pub(crate) fn resolve_source_snapshot_artifact<R: crate::ArtifactRepository>(
    repository: &mut R,
    reference: &crate::ArtifactReference,
) -> Result<VerifiedSourceArtifactBinding, SourceDefinitionError> {
    let draft = repository
        .get(&reference.tenant_id, &reference.id, reference.revision)
        .map_err(|_| SourceDefinitionError::Invalid("source artifact lookup failed"))?
        .ok_or(SourceDefinitionError::Invalid(
            "source artifact is not persisted",
        ))?;
    if draft.kind != crate::ArtifactKind::SourceSnapshot || draft.reference() != *reference {
        return Err(SourceDefinitionError::Invalid(
            "source artifact reference or kind mismatch",
        ));
    }
    let raw = draft
        .payload
        .get("source_snapshot_json")
        .and_then(serde_json::Value::as_str)
        .ok_or(SourceDefinitionError::Invalid(
            "source artifact omits exact raw snapshot bytes",
        ))?;
    let snapshot = SourceSnapshot::from_json(raw)?;
    if snapshot.tenant_id() != reference.tenant_id {
        return Err(SourceDefinitionError::Invalid(
            "artifact tenant differs from snapshot",
        ));
    }
    Ok(VerifiedSourceArtifactBinding {
        artifact_ref: reference.clone(),
        snapshot_binding_digest: snapshot.binding_digest(),
    })
}

/// Local-simulation-only bridge to U02's immutable artifact reread. The
/// returned value is opaque and can only be consumed by the corresponding
/// U08 binding method; it is not an external or production attestation.
#[cfg(feature = "local-simulation")]
pub fn local_simulation_resolve_source_snapshot_artifact<R: crate::ArtifactRepository>(
    repository: &mut R,
    reference: &crate::ArtifactReference,
) -> Result<VerifiedSourceArtifactBinding, SourceDefinitionError> {
    resolve_source_snapshot_artifact(repository, reference)
}

impl SourceFileSeal {
    #[must_use]
    pub fn table(&self) -> &str {
        &self.table
    }

    #[must_use]
    pub fn file_digest(&self) -> &str {
        &self.file_digest
    }

    #[must_use]
    pub fn partition_inventory_digest(&self) -> Option<&str> {
        self.partition_inventory_digest.as_deref()
    }

    #[must_use]
    pub fn header_digest(&self) -> &str {
        &self.header_digest
    }
}

impl SourceSnapshot {
    /// Identity fields are intentionally immutable after parsing: the binding
    /// digest commits exact snapshot bytes, and downstream consumers must not
    /// be able to mutate provenance without constructing a new snapshot.
    ///
    /// ```compile_fail
    /// # use improvement_engine_core::source_validation::SourceSnapshot;
    /// # let mut snapshot = SourceSnapshot::from_json("{}").unwrap();
    /// snapshot.tenant_id = "another-tenant".to_owned();
    /// snapshot.source_namespace = "another_namespace".to_owned();
    /// snapshot.world_ref = "another-world".to_owned();
    /// snapshot.observed_cutoff = "2026-01-01T00:00:00Z".to_owned();
    /// ```
    pub fn from_json(raw: &str) -> Result<Self, SourceDefinitionError> {
        let mut snapshot: Self = serde_json::from_str(raw).map_err(SourceDefinitionError::Json)?;
        snapshot.validate()?;
        snapshot.raw_digest = sha256(raw.as_bytes());
        Ok(snapshot)
    }

    /// Digest of the immutable snapshot bytes used by availability profiles.
    #[must_use]
    pub fn binding_digest(&self) -> String {
        format!("sha256:{}", self.raw_digest)
    }

    /// Whether this value was parsed through `from_json` and therefore has an
    /// exact, canonical byte binding. A direct serde deserialization has no
    /// trustworthy raw-byte digest and cannot bind downstream artifacts.
    #[must_use]
    pub fn has_canonical_binding(&self) -> bool {
        self.raw_digest.len() == 64
            && self
                .raw_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    #[must_use]
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    #[must_use]
    pub fn source_namespace(&self) -> &str {
        &self.source_namespace
    }

    #[must_use]
    pub fn world_ref(&self) -> &str {
        &self.world_ref
    }

    #[must_use]
    pub fn observed_cutoff(&self) -> &str {
        &self.observed_cutoff
    }

    /// Returns a sealed, read-only commitment for exactly one snapshot table.
    #[must_use]
    pub fn source_file_seal(&self, table: &str) -> Option<SourceFileSeal> {
        self.sources
            .iter()
            .find(|source| source.table == table)
            .map(|source| SourceFileSeal {
                table: source.table.clone(),
                uri: source.uri.clone(),
                file_digest: source.file_digest.clone(),
                partition_inventory_digest: source.partition_inventory_digest.clone(),
                header_digest: source.header_digest.clone(),
                source_contract_id: source.source_contract_ref.id.clone(),
                source_contract_version: source.source_contract_ref.version.clone(),
                source_contract_digest: source.source_contract_ref.digest.clone(),
            })
    }

    fn validate(&self) -> Result<(), SourceDefinitionError> {
        validate_version(&self.contract_version)?;
        validate_non_empty(&self.tenant_id, "tenant id")?;
        if self.tenant_id.len() > 128 {
            return Err(SourceDefinitionError::Invalid(
                "tenant id exceeds 128 characters",
            ));
        }
        validate_non_empty(&self.source_namespace, "source namespace")?;
        validate_non_empty(&self.world_ref, "world reference")?;
        validate_rfc3339_utc(&self.observed_cutoff)?;
        if self.sources.is_empty() {
            return Err(SourceDefinitionError::Invalid("snapshot needs sources"));
        }
        let mut tables = std::collections::HashSet::new();
        for source in &self.sources {
            validate_identifier(&source.table, "snapshot table")?;
            if !tables.insert(&source.table) {
                return Err(SourceDefinitionError::Invalid(
                    "snapshot table must be unique",
                ));
            }
            if !(source.uri.starts_with("file://") || source.uri.starts_with("s3://"))
                || !is_sha256_digest(&source.file_digest)
                || source
                    .partition_inventory_digest
                    .as_deref()
                    .is_some_and(|digest| !is_sha256_digest(digest))
                || !is_sha256_digest(&source.header_digest)
                || !is_contract_id(&source.source_contract_ref.id)
                || !is_contract_version(&source.source_contract_ref.version)
                || !is_sha256_digest(&source.source_contract_ref.digest)
            {
                return Err(SourceDefinitionError::Invalid(
                    "snapshot source field is invalid",
                ));
            }
            let _ = source.row_count;
        }
        Ok(())
    }

    /// Immutable source identity exposed to approved, read-only adapters.
    #[must_use]
    pub fn provenance(&self) -> SourceProvenance {
        SourceProvenance {
            tenant_id: self.tenant_id.clone(),
            source_namespace: self.source_namespace.clone(),
            world_ref: self.world_ref.clone(),
            observed_cutoff: self.observed_cutoff.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceProvenance {
    pub tenant_id: String,
    pub source_namespace: String,
    pub world_ref: String,
    pub observed_cutoff: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualityFindingKind {
    SourceContractDigestMismatch,
    HeaderColumnsMismatch,
    HeaderDigestMismatch,
    FileDigestMismatch,
    PolicyClassificationViolation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityFinding {
    pub kind: QualityFindingKind,
    pub column: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceValidationReport {
    pub provenance: SourceProvenance,
    pub findings: Vec<QualityFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceValidationError {
    NamespaceMismatch,
    MissingSnapshotSource,
    MissingHeader,
    ContractReferenceMismatch,
}

/// Validates one synthetic or adapter-provided immutable source against its
/// already-approved snapshot. Findings are emitted in a fixed order so they can
/// be stored, compared and reviewed without LLM interpretation.
pub fn validate_source_file(
    contract: &SourceContract,
    snapshot: &SourceSnapshot,
    source_bytes: &[u8],
) -> Result<SourceValidationReport, SourceValidationError> {
    if contract.source_namespace != snapshot.source_namespace {
        return Err(SourceValidationError::NamespaceMismatch);
    }

    let snapshot_source = snapshot
        .sources
        .iter()
        .find(|source| source.table == contract.table)
        .ok_or(SourceValidationError::MissingSnapshotSource)?;
    let header = csv_header(source_bytes).ok_or(SourceValidationError::MissingHeader)?;
    let mut findings = Vec::new();

    if snapshot_source.source_contract_ref.id != contract.table
        || snapshot_source.source_contract_ref.version
            != format!("v{}", contract.contract_version.major)
    {
        return Err(SourceValidationError::ContractReferenceMismatch);
    }

    if snapshot_source.source_contract_ref.digest != format!("sha256:{}", contract.raw_digest) {
        findings.push(QualityFinding {
            kind: QualityFindingKind::SourceContractDigestMismatch,
            column: None,
        });
    }
    if !header_matches_contract(header, &contract.columns) {
        findings.push(QualityFinding {
            kind: QualityFindingKind::HeaderColumnsMismatch,
            column: None,
        });
    }
    if snapshot_source.header_digest != format!("sha256:{}", sha256(header)) {
        findings.push(QualityFinding {
            kind: QualityFindingKind::HeaderDigestMismatch,
            column: None,
        });
    }
    if snapshot_source.file_digest != format!("sha256:{}", sha256(source_bytes)) {
        findings.push(QualityFinding {
            kind: QualityFindingKind::FileDigestMismatch,
            column: None,
        });
    }
    for column in &contract.columns {
        if !contract
            .access_policy
            .permitted_classifications
            .iter()
            .any(|classification| classification == &column.classification)
        {
            findings.push(QualityFinding {
                kind: QualityFindingKind::PolicyClassificationViolation,
                column: Some(column.name.clone()),
            });
        }
    }

    Ok(SourceValidationReport {
        provenance: SourceProvenance {
            tenant_id: snapshot.tenant_id.clone(),
            source_namespace: snapshot.source_namespace.clone(),
            world_ref: snapshot.world_ref.clone(),
            observed_cutoff: snapshot.observed_cutoff.clone(),
        },
        findings,
    })
}

fn csv_header(source_bytes: &[u8]) -> Option<&[u8]> {
    let header = source_bytes.split(|byte| *byte == b'\n').next()?;
    let header = header.strip_suffix(b"\r").unwrap_or(header);
    (!header.is_empty()).then_some(header)
}

/// SourceContract column names are non-empty simple identifiers. The Layer 0
/// boundary therefore accepts only an exact, unquoted comma-delimited header;
/// a richer CSV dialect belongs to the future approved adapter, not this
/// no-data-leaving verifier.
fn header_matches_contract(header: &[u8], columns: &[SourceColumn]) -> bool {
    let Ok(header) = std::str::from_utf8(header) else {
        return false;
    };
    header
        .split(',')
        .zip(columns)
        .all(|(observed, column)| observed == column.name)
        && header.split(',').count() == columns.len()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_version(version: &ContractVersion) -> Result<(), SourceDefinitionError> {
    if version.major != 1 {
        return Err(SourceDefinitionError::Invalid(
            "only contract major version 1 is supported",
        ));
    }
    let _ = version.minor;
    Ok(())
}

fn validate_non_empty(value: &str, field: &'static str) -> Result<(), SourceDefinitionError> {
    if value.is_empty() {
        return Err(SourceDefinitionError::Invalid(field));
    }
    Ok(())
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), SourceDefinitionError> {
    let mut characters = value.chars();
    if !matches!(characters.next(), Some(character) if character.is_ascii_lowercase())
        || !characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(SourceDefinitionError::Invalid(field));
    }
    Ok(())
}

fn validate_rfc3339_utc(value: &str) -> Result<(), SourceDefinitionError> {
    if !value.ends_with('Z')
        || value.len() < 20
        || !value.as_bytes().get(10).is_some_and(|byte| *byte == b'T')
    {
        return Err(SourceDefinitionError::Invalid(
            "observed cutoff must be RFC3339 UTC",
        ));
    }
    Ok(())
}

fn is_classification(value: &str) -> bool {
    matches!(
        value,
        "internal" | "pseudonymized" | "aggregated" | "restricted"
    )
}

fn is_permitted_classification(value: &str) -> bool {
    matches!(value, "internal" | "pseudonymized" | "aggregated")
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_contract_id(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(character) if character.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '.' | '-')
        })
}

fn is_contract_version(value: &str) -> bool {
    value.strip_prefix('v').is_some_and(|digits| {
        !digits.is_empty()
            && !digits.starts_with('0')
            && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}
