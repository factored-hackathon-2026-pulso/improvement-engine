//! Bounded, read-only local investigation sessions.
//!
//! U08 deliberately uses an in-memory SQLite connection populated only from an
//! already approved representation. Callers cannot submit SQL, paths, URLs, or
//! write operations: the small query AST is compiled by this module into one
//! `SELECT` over an allowlisted table. This is a real local query engine, while
//! keeping the host filesystem, network, raw dataset import, and source writes
//! outside the capability boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{Connection, params_from_iter};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ArtifactReference;

const MAX_PROJECTION_COLUMNS: usize = 16;
const MAX_RESULT_ROWS: usize = 100;
const MAX_SOURCE_TABLES: usize = 16;
const MAX_SOURCE_ROWS: usize = 5_000;
const MAX_SOURCE_VALUES: usize = 50_000;
const MAX_VALUE_BYTES: usize = 4_096;
const MAX_SOURCE_BYTES: usize = 1_048_576;
type QueryRows = Vec<BTreeMap<String, String>>;
static NEXT_LAB_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// Exact authority supplied at every session operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabAccess {
    pub run_id: String,
    pub tenant_id: String,
    pub purpose: String,
    pub grant_id: String,
    pub authority_ref: String,
    pub snapshot_ref: ArtifactReference,
    pub expires_at_unix_seconds: u64,
}

impl LabAccess {
    #[must_use]
    pub fn new(
        run_id: impl Into<String>,
        tenant_id: impl Into<String>,
        purpose: impl Into<String>,
        grant_id: impl Into<String>,
        authority_ref: impl Into<String>,
        snapshot_ref: ArtifactReference,
        expires_at_unix_seconds: u64,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            tenant_id: tenant_id.into(),
            purpose: purpose.into(),
            grant_id: grant_id.into(),
            authority_ref: authority_ref.into(),
            snapshot_ref,
            expires_at_unix_seconds,
        }
    }
}

/// A test/local representation of an exact grant. U05 remains owner of the
/// durable lifecycle, revocation and quota ledger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabGrant(LabAccess);

impl LabGrant {
    #[must_use]
    pub fn from_access(access: &LabAccess) -> Self {
        Self(access.clone())
    }
}

/// Narrow authorization seam for the local lab boundary.
pub trait LabAuthorizationPort {
    fn authorize(&self, access: &LabAccess) -> bool;
}

/// Deterministic authority for tests and local development only.
#[derive(Default)]
pub struct InMemoryLabGrantAuthority {
    grants: BTreeMap<String, LabGrant>,
}

impl InMemoryLabGrantAuthority {
    pub fn issue(&mut self, grant: LabGrant) {
        self.grants.insert(grant.0.grant_id.clone(), grant);
    }
}

impl LabAuthorizationPort for InMemoryLabGrantAuthority {
    fn authorize(&self, access: &LabAccess) -> bool {
        self.grants
            .get(&access.grant_id)
            .is_some_and(|grant| grant.0 == *access)
    }
}

/// One approved table in a source representation already supplied by a source
/// adapter. No file path, URI or external connection appears in this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabTable {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<BTreeMap<String, String>>,
}

/// Classification is set by the source adapter. PII and sources not declared
/// safe for discovery cannot become a lab capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LabDataClassification {
    Treated,
    Pii,
}

/// Adapter-issued provenance used to construct an immutable local source.
/// Fields are grouped to keep the source constructor small and auditable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabSourceManifest {
    pub tenant_id: String,
    pub snapshot_ref: ArtifactReference,
    pub source_contract_digest: String,
    pub source_digest: String,
    pub transform_digest: String,
    pub cutoff_unix_seconds: u64,
    pub classification: LabDataClassification,
    pub safe_for_discovery: bool,
}

impl LabTable {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        columns: Vec<impl Into<String>>,
        rows: Vec<BTreeMap<String, String>>,
    ) -> Self {
        Self {
            name: name.into(),
            columns: columns.into_iter().map(Into::into).collect(),
            rows,
        }
    }
}

/// Immutable provenance and approved local representation for one source
/// snapshot. The builder validates table schemas before SQLite receives rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabSource {
    tenant_id: String,
    snapshot_ref: ArtifactReference,
    source_contract_digest: String,
    source_digest: String,
    transform_digest: String,
    cutoff_unix_seconds: u64,
    classification: LabDataClassification,
    safe_for_discovery: bool,
    tables: Vec<LabTable>,
}

impl LabSource {
    pub fn new(manifest: LabSourceManifest, tables: Vec<LabTable>) -> Result<Self, LabError> {
        let mut source = Self {
            tenant_id: manifest.tenant_id,
            snapshot_ref: manifest.snapshot_ref,
            source_contract_digest: manifest.source_contract_digest,
            source_digest: manifest.source_digest,
            transform_digest: manifest.transform_digest,
            cutoff_unix_seconds: manifest.cutoff_unix_seconds,
            classification: manifest.classification,
            safe_for_discovery: manifest.safe_for_discovery,
            tables,
        };
        source
            .tables
            .sort_by(|left, right| left.name.cmp(&right.name));
        for table in &mut source.tables {
            table.rows.sort_by_key(canonical_row_bytes);
        }
        source.validate()?;
        Ok(source)
    }

    fn validate(&self) -> Result<(), LabError> {
        if self.tenant_id != self.snapshot_ref.tenant_id
            || !valid_identifier(&self.tenant_id)
            || !valid_digest(&self.snapshot_ref.digest)
            || !valid_digest(&self.source_contract_digest)
            || !valid_digest(&self.source_digest)
            || !valid_digest(&self.transform_digest)
            || self.cutoff_unix_seconds == 0
            || self.tables.is_empty()
            || self.tables.len() > MAX_SOURCE_TABLES
        {
            return Err(LabError::InvalidSource);
        }
        let mut names = BTreeSet::new();
        let mut values = 0_usize;
        let mut bytes = 0_usize;
        for table in &self.tables {
            if !valid_identifier(&table.name)
                || table.columns.is_empty()
                || !names.insert(&table.name)
                || table.columns.len() > MAX_PROJECTION_COLUMNS
            {
                return Err(LabError::InvalidSource);
            }
            let mut columns = BTreeSet::new();
            for column in &table.columns {
                if !valid_identifier(column) || !columns.insert(column) {
                    return Err(LabError::InvalidSource);
                }
            }
            if table.rows.len() > MAX_SOURCE_ROWS {
                return Err(LabError::InvalidSource);
            }
            for row in &table.rows {
                if row.len() != table.columns.len()
                    || table.columns.iter().any(|column| !row.contains_key(column))
                {
                    return Err(LabError::InvalidSource);
                }
                values += row.len();
                bytes += row.values().map(String::len).sum::<usize>();
                if values > MAX_SOURCE_VALUES
                    || bytes > MAX_SOURCE_BYTES
                    || row.values().any(|value| value.len() > MAX_VALUE_BYTES)
                {
                    return Err(LabError::InvalidSource);
                }
            }
        }
        Ok(())
    }
}

/// Opaque capability minted by a source adapter after it has applied the U03/U04
/// source, temporal and discovery-safety gates. A raw `LabSource` cannot open
/// a session directly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovedLabSource {
    approval_id: String,
    source: LabSource,
}

/// Separate authorization seam so U03/U04 adapters can replace the local
/// authority without changing the lab session contract.
pub trait LabSourceApprovalPort {
    fn approve(&mut self, source: LabSource) -> Result<ApprovedLabSource, LabError>;
}

/// Test/local source authority. It accepts only treated, tenant-bound sources
/// explicitly marked safe by the adapter; it is not an ingestion substitute.
#[derive(Default)]
pub struct InMemoryLabSourceAuthority;

impl LabSourceApprovalPort for InMemoryLabSourceAuthority {
    fn approve(&mut self, source: LabSource) -> Result<ApprovedLabSource, LabError> {
        source.validate()?;
        if source.classification != LabDataClassification::Treated
            || !source.safe_for_discovery
            || source.tenant_id != source.snapshot_ref.tenant_id
        {
            return Err(LabError::SourceApprovalDenied);
        }
        let approval_id = digest_of(&(
            &source.tenant_id,
            &source.snapshot_ref,
            &source.source_contract_digest,
            &source.source_digest,
            &source.transform_digest,
            source.cutoff_unix_seconds,
        ));
        Ok(ApprovedLabSource {
            approval_id,
            source,
        })
    }
}

/// One bounded read-only query shape. The public API intentionally has no raw
/// SQL variant. Write and external variants exist solely to make denials
/// observable and regression-testable at this boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LabQuery {
    operation: QueryOperation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum QueryOperation {
    Select {
        table: String,
        columns: Vec<String>,
        filter: Option<QueryFilter>,
    },
    SourceWrite {
        table: String,
    },
    ExternalIo {
        destination: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryFilter {
    Equals {
        column: String,
        value: String,
    },
    PriorResult {
        source_column: String,
        receipt_digest: String,
        prior_column: String,
    },
}

impl QueryFilter {
    #[must_use]
    pub fn equals(column: impl Into<String>, value: impl Into<String>) -> Self {
        Self::Equals {
            column: column.into(),
            value: value.into(),
        }
    }
}

impl LabQuery {
    #[must_use]
    pub fn select(
        table: impl Into<String>,
        columns: Vec<impl Into<String>>,
        filter: Option<QueryFilter>,
    ) -> Self {
        Self {
            operation: QueryOperation::Select {
                table: table.into(),
                columns: columns.into_iter().map(Into::into).collect(),
                filter,
            },
        }
    }

    #[must_use]
    pub fn dependent_select(
        table: impl Into<String>,
        columns: Vec<impl Into<String>>,
        source_column: impl Into<String>,
        receipt_digest: impl Into<String>,
        prior_column: impl Into<String>,
    ) -> Self {
        Self::select(
            table,
            columns,
            Some(QueryFilter::PriorResult {
                source_column: source_column.into(),
                receipt_digest: receipt_digest.into(),
                prior_column: prior_column.into(),
            }),
        )
    }

    #[must_use]
    pub fn source_write(table: impl Into<String>) -> Self {
        Self {
            operation: QueryOperation::SourceWrite {
                table: table.into(),
            },
        }
    }

    #[must_use]
    pub fn external_io(destination: impl Into<String>) -> Self {
        Self {
            operation: QueryOperation::ExternalIo {
                destination: destination.into(),
            },
        }
    }
}

/// Opaque session identity, fixed to one run/snapshot/grant scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabSession {
    session_id: String,
}

impl LabSession {
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// Provenance evidence for one fully completed query. It is only appended after
/// SQLite returned every row, so a failed query cannot publish partial state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QueryReceipt {
    pub session_id: String,
    pub run_id: String,
    pub tenant_id: String,
    pub grant_id: String,
    pub authority_ref: String,
    pub sequence: u64,
    pub digest: String,
    pub query_digest: String,
    /// Retained alongside the query digest so E0 can prove a receipt did not
    /// originate from labels or a hidden join. Both remain digest-bound.
    pub queried_table: String,
    pub queried_columns: Vec<String>,
    pub depends_on: Option<String>,
    pub source_snapshot_ref: ArtifactReference,
    pub source_contract_digest: String,
    pub source_digest: String,
    pub transform_digest: String,
    pub source_approval_id: String,
    pub cutoff_unix_seconds: u64,
    pub row_count: usize,
    pub rows_digest: String,
    /// Present only after U08-E has revalidated this governed U08 result
    /// against a sealed U04-B replay projection.
    pub e0_replay: Option<E0ReplayReceiptBinding>,
}

/// Commitments added by the crate-private U08-E adapter. No source rows,
/// labels, paths or source handles cross into this receipt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct E0ReplayReceiptBinding {
    pub source_snapshot_digest: String,
    pub availability_profile_digest: String,
    pub field_commitment: String,
    pub replay_projection_digest: String,
}

impl QueryReceipt {
    /// Verifies the receipt's self-contained canonical digest before another
    /// boundary relies on it.
    #[must_use]
    pub fn has_valid_digest(&self) -> bool {
        let mut unsigned = self.clone();
        let expected = std::mem::take(&mut unsigned.digest);
        expected == digest_of(&unsigned)
    }

    #[must_use]
    pub fn binds_rows(&self, rows: &[BTreeMap<String, String>]) -> bool {
        self.rows_digest == digest_of(&rows)
    }

    pub(crate) fn bind_e0_replay(mut self, binding: E0ReplayReceiptBinding) -> Self {
        self.e0_replay = Some(binding);
        self.digest.clear();
        self.digest = digest_of(&self);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryResult {
    rows: QueryRows,
    receipt: QueryReceipt,
}

impl QueryResult {
    #[must_use]
    pub fn rows(&self) -> &[BTreeMap<String, String>] {
        &self.rows
    }

    #[must_use]
    pub fn receipt(&self) -> &QueryReceipt {
        &self.receipt
    }

    /// Decomposes a result for a downstream verification boundary.
    #[must_use]
    pub fn into_parts(self) -> (QueryRows, QueryReceipt) {
        (self.rows, self.receipt)
    }

    /// Reconstructs untrusted query output. Consumers must verify the receipt
    /// and must never treat this constructor as authority to execute a query.
    #[must_use]
    pub fn untrusted(rows: QueryRows, receipt: QueryReceipt) -> Self {
        Self { rows, receipt }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LabError {
    AccessDenied,
    SessionNotFound,
    SessionExpired,
    SessionAlreadyOpen,
    InvalidSource,
    SourceApprovalDenied,
    QueryShapeDenied { reason: &'static str },
    SourceWriteDenied,
    ExternalIoDenied,
    DependencyDenied,
    QueryFailed,
}

struct StoredSession {
    access: LabAccess,
    lab_instance_nonce: u64,
    source: LabSource,
    approval_id: String,
    connection: Connection,
    receipts: Vec<QueryReceipt>,
    results: BTreeMap<String, QueryRows>,
    consumed_dependencies: BTreeSet<String>,
}

/// One process-local lab. It does not open files, call the network, expose the
/// SQLite connection, or persist sessions. U28 replaces this with an isolated
/// remote sandbox; U08 makes the same capability boundary executable locally.
pub struct LocalInvestigationLab<A = InMemoryLabGrantAuthority> {
    authority: A,
    lab_instance_nonce: u64,
    sessions: BTreeMap<String, StoredSession>,
}

impl<A: LabAuthorizationPort> LocalInvestigationLab<A> {
    #[must_use]
    pub fn new(authority: A) -> Self {
        Self {
            authority,
            lab_instance_nonce: NEXT_LAB_INSTANCE.fetch_add(1, Ordering::Relaxed),
            sessions: BTreeMap::new(),
        }
    }

    pub fn open(
        &mut self,
        access: LabAccess,
        approved_source: ApprovedLabSource,
        now_unix_seconds: u64,
    ) -> Result<LabSession, LabError> {
        if !self.authority.authorize(&access)
            || approved_source.source.snapshot_ref != access.snapshot_ref
            || approved_source.source.tenant_id != access.tenant_id
        {
            return Err(LabError::AccessDenied);
        }
        if now_unix_seconds >= access.expires_at_unix_seconds {
            return Err(LabError::SessionExpired);
        }
        if approved_source.source.cutoff_unix_seconds > now_unix_seconds {
            return Err(LabError::InvalidSource);
        }
        let session_id = session_id(&access, self.lab_instance_nonce);
        if self.sessions.contains_key(&session_id) {
            return Err(LabError::SessionAlreadyOpen);
        }
        let connection = load_source(&approved_source.source)?;
        self.sessions.insert(
            session_id.clone(),
            StoredSession {
                access,
                lab_instance_nonce: self.lab_instance_nonce,
                source: approved_source.source,
                approval_id: approved_source.approval_id,
                connection,
                receipts: Vec::new(),
                results: BTreeMap::new(),
                consumed_dependencies: BTreeSet::new(),
            },
        );
        Ok(LabSession { session_id })
    }

    pub fn query(
        &mut self,
        session_id: &str,
        access: &LabAccess,
        query: LabQuery,
        now_unix_seconds: u64,
    ) -> Result<QueryResult, LabError> {
        let session = self.session_mut(session_id, access, now_unix_seconds)?;
        let (table, columns, filter) = match query.operation {
            QueryOperation::Select {
                table,
                columns,
                filter,
            } => (table, columns, filter),
            QueryOperation::SourceWrite { .. } => return Err(LabError::SourceWriteDenied),
            QueryOperation::ExternalIo { .. } => return Err(LabError::ExternalIoDenied),
        };
        validate_query_shape(&session.source, &table, &columns, filter.as_ref())?;
        let query_digest = digest_of(&LabQuery::select(
            table.clone(),
            columns.clone(),
            filter.clone(),
        ));
        let (rows, dependency) = execute_select(session, &table, &columns, filter.as_ref())?;
        let sequence = session.receipts.len() as u64 + 1;
        let mut receipt = QueryReceipt {
            session_id: session_id.to_owned(),
            run_id: session.access.run_id.clone(),
            tenant_id: session.access.tenant_id.clone(),
            grant_id: session.access.grant_id.clone(),
            authority_ref: session.access.authority_ref.clone(),
            sequence,
            digest: String::new(),
            query_digest,
            queried_table: table,
            queried_columns: columns,
            depends_on: dependency,
            source_snapshot_ref: session.source.snapshot_ref.clone(),
            source_contract_digest: session.source.source_contract_digest.clone(),
            source_digest: session.source.source_digest.clone(),
            transform_digest: session.source.transform_digest.clone(),
            source_approval_id: session.approval_id.clone(),
            cutoff_unix_seconds: session.source.cutoff_unix_seconds,
            row_count: rows.len(),
            rows_digest: digest_of(&rows),
            e0_replay: None,
        };
        receipt.digest = digest_of(&receipt);
        session.results.insert(receipt.digest.clone(), rows.clone());
        if let Some(dependency) = &receipt.depends_on {
            session.consumed_dependencies.insert(dependency.clone());
        }
        session.receipts.push(receipt.clone());
        Ok(QueryResult { rows, receipt })
    }

    pub fn receipts(
        &self,
        session_id: &str,
        access: &LabAccess,
        now_unix_seconds: u64,
    ) -> Result<&[QueryReceipt], LabError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or(LabError::SessionNotFound)?;
        if !self.authority.authorize(access) || session.access != *access {
            return Err(LabError::AccessDenied);
        }
        // Receipt reads intentionally deny after TTL. A stale caller cannot use
        // a session as a durable discovery cache.
        if now_unix_seconds >= session.access.expires_at_unix_seconds {
            return Err(LabError::SessionExpired);
        }
        Ok(&session.receipts)
    }

    /// Explicitly closes one matching session, dropping its SQLite connection,
    /// derived rows and receipts. Closing never publishes a source mutation.
    pub fn close(&mut self, session_id: &str, access: &LabAccess) -> Result<(), LabError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or(LabError::SessionNotFound)?;
        if !self.authority.authorize(access) || session.access != *access {
            return Err(LabError::AccessDenied);
        }
        self.sessions.remove(session_id);
        Ok(())
    }

    /// Evicts expired ephemeral state and returns the number of destroyed labs.
    pub fn purge_expired(&mut self, now_unix_seconds: u64) -> usize {
        let before = self.sessions.len();
        self.sessions
            .retain(|_, session| now_unix_seconds < session.access.expires_at_unix_seconds);
        before - self.sessions.len()
    }

    fn session_mut(
        &mut self,
        session_id: &str,
        access: &LabAccess,
        now_unix_seconds: u64,
    ) -> Result<&mut StoredSession, LabError> {
        let authorized = self.authority.authorize(access);
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or(LabError::SessionNotFound)?;
        if !authorized || session.access != *access {
            return Err(LabError::AccessDenied);
        }
        if now_unix_seconds >= session.access.expires_at_unix_seconds {
            return Err(LabError::SessionExpired);
        }
        Ok(session)
    }
}

fn load_source(source: &LabSource) -> Result<Connection, LabError> {
    let connection = Connection::open_in_memory().map_err(|_| LabError::QueryFailed)?;
    for table in &source.tables {
        let quoted_columns = table
            .columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>();
        let create = format!(
            "CREATE TABLE {} ({})",
            quote_identifier(&table.name),
            quoted_columns
                .iter()
                .map(|column| format!("{column} TEXT NOT NULL"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        connection
            .execute(&create, [])
            .map_err(|_| LabError::QueryFailed)?;
        let insert = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_identifier(&table.name),
            quoted_columns.join(", "),
            (0..table.columns.len())
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(", ")
        );
        for row in &table.rows {
            let values = table
                .columns
                .iter()
                .map(|column| row.get(column).expect("source schema was validated"))
                .collect::<Vec<_>>();
            connection
                .execute(&insert, params_from_iter(values))
                .map_err(|_| LabError::QueryFailed)?;
        }
    }
    connection
        .execute_batch("PRAGMA query_only = ON;")
        .map_err(|_| LabError::QueryFailed)?;
    Ok(connection)
}

fn execute_select(
    session: &mut StoredSession,
    table: &str,
    columns: &[String],
    filter: Option<&QueryFilter>,
) -> Result<(QueryRows, Option<String>), LabError> {
    let projection = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let mut dependency = None;
    let mut values = Vec::new();
    let where_clause = match filter {
        None => String::new(),
        Some(QueryFilter::Equals { column, value }) => {
            values.push(value.clone());
            format!(" WHERE {} = ?", quote_identifier(column))
        }
        Some(QueryFilter::PriorResult {
            source_column,
            receipt_digest,
            prior_column,
        }) => {
            if session.consumed_dependencies.contains(receipt_digest) {
                return Err(LabError::DependencyDenied);
            }
            let receipt = session
                .receipts
                .iter()
                .find(|receipt| receipt.digest == *receipt_digest)
                .ok_or(LabError::DependencyDenied)?;
            if !receipt.has_valid_digest()
                || receipt.session_id != session_id(&session.access, session.lab_instance_nonce)
                || receipt.run_id != session.access.run_id
                || receipt.source_snapshot_ref != session.source.snapshot_ref
                || receipt.source_approval_id != session.approval_id
            {
                return Err(LabError::DependencyDenied);
            }
            let prior = session
                .results
                .get(receipt_digest)
                .ok_or(LabError::DependencyDenied)?;
            let prior_values = prior
                .iter()
                .map(|row| {
                    row.get(prior_column)
                        .cloned()
                        .ok_or(LabError::DependencyDenied)
                })
                .collect::<Result<Vec<_>, _>>()?;
            dependency = Some(receipt_digest.clone());
            if prior_values.is_empty() {
                " WHERE 1 = 0".to_owned()
            } else {
                values.extend(prior_values);
                format!(
                    " WHERE {} IN ({})",
                    quote_identifier(source_column),
                    std::iter::repeat_n("?", values.len())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
    };
    let sql = format!(
        "SELECT {projection} FROM {}{where_clause} ORDER BY {projection} LIMIT {}",
        quote_identifier(table),
        MAX_RESULT_ROWS
    );
    let mut statement = session
        .connection
        .prepare(&sql)
        .map_err(|_| LabError::QueryFailed)?;
    let mut cursor = statement
        .query(params_from_iter(values.iter()))
        .map_err(|_| LabError::QueryFailed)?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.next().map_err(|_| LabError::QueryFailed)? {
        let mut output = BTreeMap::new();
        for (index, column) in columns.iter().enumerate() {
            output.insert(
                column.clone(),
                row.get(index).map_err(|_| LabError::QueryFailed)?,
            );
        }
        rows.push(output);
    }
    Ok((rows, dependency))
}

fn validate_query_shape(
    source: &LabSource,
    table: &str,
    columns: &[String],
    filter: Option<&QueryFilter>,
) -> Result<(), LabError> {
    let Some(approved) = source
        .tables
        .iter()
        .find(|candidate| candidate.name == table)
    else {
        return Err(LabError::QueryShapeDenied {
            reason: "table is not approved",
        });
    };
    if columns.is_empty() {
        return Err(LabError::QueryShapeDenied {
            reason: "projection cannot be empty",
        });
    }
    if columns.len() > MAX_PROJECTION_COLUMNS
        || columns
            .iter()
            .any(|column| !approved.columns.contains(column))
        || columns.iter().collect::<BTreeSet<_>>().len() != columns.len()
    {
        return Err(LabError::QueryShapeDenied {
            reason: "projection is not approved",
        });
    }
    let filter_column = match filter {
        None => return Ok(()),
        Some(QueryFilter::Equals { column, value }) => {
            if value.len() > MAX_VALUE_BYTES {
                return Err(LabError::QueryShapeDenied {
                    reason: "filter value exceeds bound",
                });
            }
            column
        }
        Some(QueryFilter::PriorResult {
            source_column,
            prior_column,
            receipt_digest,
        }) => {
            if !valid_identifier(prior_column) || !valid_digest(receipt_digest) {
                return Err(LabError::QueryShapeDenied {
                    reason: "dependency is malformed",
                });
            }
            source_column
        }
    };
    if !approved.columns.contains(filter_column) {
        return Err(LabError::QueryShapeDenied {
            reason: "filter is not approved",
        });
    }
    Ok(())
}

fn session_id(access: &LabAccess, lab_instance_nonce: u64) -> String {
    digest_of(&(
        &access.run_id,
        &access.tenant_id,
        &access.grant_id,
        &access.snapshot_ref,
        lab_instance_nonce,
    ))
}

fn quote_identifier(value: &str) -> String {
    format!("\"{value}\"")
}

fn canonical_row_bytes(row: &BTreeMap<String, String>) -> Vec<u8> {
    serde_json::to_vec(row).expect("row maps are serializable")
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn digest_of<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("lab evidence is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}
