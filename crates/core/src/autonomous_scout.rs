//! U13 autonomous Scout: turns sealed, treated evidence into *candidate*
//! drafts. It cannot publish a detector, proposal, tool, or platform effect.

#[cfg(any(test, feature = "local-simulation"))]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
#[cfg(test)]
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ArtifactReference;
use crate::core_task::{CoreTaskOutcome, CoreTaskReceipt, CoreTaskScope};
use crate::deterministic_sensor::DeterministicSignal;
use crate::e0_deterministic_sensor::{E0DiagnosticSignal, E0ScoutSignalBinding};
use crate::model_provider::{ModelOutcome, ModelReceipt};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CandidateKind {
    Signal,
    Claim,
    Opportunity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScoutCandidateDraft {
    pub kind: CandidateKind,
    pub candidate_id: String,
    pub metric_id: String,
    /// Immutable U08 snapshot that supplied the sealed query receipts.
    pub source_snapshot_ref: crate::ArtifactReference,
    /// The scope is retained verbatim so a candidate cannot be replayed into
    /// another tenant, job, grant, or authority context.
    pub tenant_id: String,
    pub job_id: String,
    pub grant_id: String,
    pub authority_ref: String,
    pub source_data_digest: String,
    pub source_contract_digest: String,
    pub transform_digest: String,
    pub cutoff_unix_seconds: u64,
    pub query_receipt_digests: Vec<String>,
    pub signal_commitment: String,
    /// Present only when the draft crossed the authenticated U12-E boundary.
    pub e0_provenance: Option<E0ScoutCandidateProvenance>,
    pub core_input_commitment: String,
    pub model_input_commitment: String,
    pub model_capability_digest: String,
    pub core_binding_digest: String,
    pub core_attempt_id: String,
    pub core_run_id: Option<String>,
    pub core_output_digest: Option<String>,
    pub model_policy_digest: String,
    pub model_attempt_id: String,
    pub model_receipt_evidence: String,
    pub model_output_digest: Option<String>,
    /// Full canonical SHA-256 commitment to all persisted provenance inputs.
    pub provenance_commitment: String,
    pub digest: String,
}

#[derive(Serialize)]
struct CandidateProvenance<'a> {
    tenant_id: &'a str,
    job_id: &'a str,
    grant_id: &'a str,
    authority_ref: &'a str,
    signal: &'a DeterministicSignal,
    e0_provenance_commitment: Option<&'a str>,
    core_binding_digest: &'a str,
    core_attempt_id: &'a str,
    core_run_id: Option<&'a str>,
    core_output_digest: Option<&'a str>,
    model_policy_digest: &'a str,
    model_capability_digest: &'a str,
    model_input_commitment: &'a str,
    model_attempt_id: &'a str,
    model_evidence: &'a str,
    model_output_digest: Option<&'a str>,
}

/// Canonical, typed E0 provenance persisted with an E0-derived Scout draft.
/// It is descriptive provenance, not an outcome, release or authorization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct E0ScoutCandidateProvenance {
    tenant_id: String,
    job_id: String,
    grant_id: String,
    authority_ref: String,
    /// U08-E execution identity. It is distinct from the U13 job ID.
    run_id: String,
    signal_commitment: String,
    metric_spec_commitment: String,
    metric_policy_id: String,
    metric_policy_version: u16,
    metric_semantics: String,
    numerator: u64,
    denominator: u64,
    missing: u64,
    coverage_basis_points: u16,
    window_start_unix_seconds: u64,
    window_end_unix_seconds: u64,
    source_snapshot_ref: crate::ArtifactReference,
    source_snapshot_binding: String,
    availability_profile_digest: String,
    table: String,
    cutoff_unix_seconds: u64,
    source_contract_digest: String,
    source_digest: String,
    transform_digest: String,
    field_commitment: String,
    replay_projection_digest: String,
    source_evidence_digest: String,
    query_receipt_digests: Vec<String>,
    commitment: String,
}

impl E0ScoutCandidateProvenance {
    #[must_use]
    pub fn commitment(&self) -> &str {
        &self.commitment
    }

    fn from_binding(scope: &CoreTaskScope, binding: &E0ScoutSignalBinding) -> Self {
        let mut value = Self {
            tenant_id: scope.tenant_id().to_owned(),
            job_id: scope.job_id().to_owned(),
            grant_id: scope.grant_id().to_owned(),
            authority_ref: scope.authority_ref().to_owned(),
            run_id: binding.run_id.clone(),
            signal_commitment: binding.signal_digest.clone(),
            metric_spec_commitment: binding.metric_spec_commitment.clone(),
            metric_policy_id: binding.metric_policy_id.clone(),
            metric_policy_version: binding.metric_policy_version,
            metric_semantics: binding.metric_semantics.clone(),
            numerator: binding.numerator,
            denominator: binding.denominator,
            missing: binding.missing,
            coverage_basis_points: binding.coverage_basis_points,
            window_start_unix_seconds: binding.window.start_unix_seconds(),
            window_end_unix_seconds: binding.window.end_unix_seconds(),
            source_snapshot_ref: binding.source_snapshot_ref.clone(),
            source_snapshot_binding: binding.source_snapshot_binding.clone(),
            availability_profile_digest: binding.availability_profile_digest.clone(),
            table: binding.table.clone(),
            cutoff_unix_seconds: binding.cutoff_unix_seconds,
            source_contract_digest: binding.source_contract_digest.clone(),
            source_digest: binding.source_digest.clone(),
            transform_digest: binding.transform_digest.clone(),
            field_commitment: binding.field_commitment.clone(),
            replay_projection_digest: binding.replay_projection_digest.clone(),
            source_evidence_digest: binding.source_evidence_digest.clone(),
            query_receipt_digests: binding.query_receipt_digests.clone(),
            commitment: String::new(),
        };
        value.commitment = e0_provenance_digest(&value);
        value
    }

    fn is_valid_for(&self, scope: &CoreTaskScope, binding: &E0ScoutSignalBinding) -> bool {
        self.tenant_id == scope.tenant_id()
            && self.job_id == scope.job_id()
            && self.grant_id == scope.grant_id()
            && self.authority_ref == scope.authority_ref()
            && self.run_id == binding.run_id
            && self.signal_commitment == binding.signal_digest
            && self.metric_spec_commitment == binding.metric_spec_commitment
            && self.metric_policy_id == binding.metric_policy_id
            && self.metric_policy_version == binding.metric_policy_version
            && self.metric_semantics == binding.metric_semantics
            && self.numerator == binding.numerator
            && self.denominator == binding.denominator
            && self.missing == binding.missing
            && self.coverage_basis_points == binding.coverage_basis_points
            && self.window_start_unix_seconds == binding.window.start_unix_seconds()
            && self.window_end_unix_seconds == binding.window.end_unix_seconds()
            && self.source_snapshot_ref == binding.source_snapshot_ref
            && self.source_snapshot_binding == binding.source_snapshot_binding
            && self.availability_profile_digest == binding.availability_profile_digest
            && self.table == binding.table
            && self.cutoff_unix_seconds == binding.cutoff_unix_seconds
            && self.source_contract_digest == binding.source_contract_digest
            && self.source_digest == binding.source_digest
            && self.transform_digest == binding.transform_digest
            && self.field_commitment == binding.field_commitment
            && self.replay_projection_digest == binding.replay_projection_digest
            && self.source_evidence_digest == binding.source_evidence_digest
            && self.query_receipt_digests == binding.query_receipt_digests
            && self.commitment == e0_provenance_digest(self)
    }
}

fn e0_provenance_digest(value: &E0ScoutCandidateProvenance) -> String {
    let mut unsigned = value.clone();
    unsigned.commitment.clear();
    digest(&unsigned)
}

#[derive(Serialize)]
struct E0CandidateInvocationProvenance<'a> {
    e0: &'a E0ScoutCandidateProvenance,
    core_binding_digest: &'a str,
    core_attempt_id: &'a str,
    core_run_id: Option<&'a str>,
    core_output_digest: Option<&'a str>,
    model_policy_digest: &'a str,
    model_capability_digest: &'a str,
    model_input_commitment: &'a str,
    model_attempt_id: &'a str,
    model_evidence: &'a str,
    model_output_digest: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScoutResult {
    Candidates(Vec<ScoutCandidateDraft>),
    DependencyBlocked { reason: &'static str },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScoutError {
    EvidenceDenied,
    ScopeMismatch,
}

/// Full immutable durable lookup key. It deliberately includes all four scope
/// dimensions plus the independent candidate and provenance commitments.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ScoutCandidateRecordKey {
    tenant_id: String,
    job_id: String,
    grant_id: String,
    authority_ref: String,
    candidate_digest: String,
    provenance_commitment: String,
}

impl ScoutCandidateRecordKey {
    pub(crate) fn for_lookup(
        scope: &CoreTaskScope,
        candidate_digest: impl Into<String>,
        provenance_commitment: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: scope.tenant_id().into(),
            job_id: scope.job_id().into(),
            grant_id: scope.grant_id().into(),
            authority_ref: scope.authority_ref().into(),
            candidate_digest: candidate_digest.into(),
            provenance_commitment: provenance_commitment.into(),
        }
    }

    pub(crate) fn from_candidate(scope: &CoreTaskScope, candidate: &ScoutCandidateDraft) -> Self {
        Self::for_lookup(
            scope,
            candidate.digest.clone(),
            candidate.provenance_commitment.clone(),
        )
    }
}

/// Canonical durable record. Persistence adapters rehydrate only through the
/// validating constructor; the authority still treats the repository itself as
/// a trusted control-plane boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScoutCandidateRecord {
    scope: CoreTaskScope,
    candidate: ScoutCandidateDraft,
}

impl ScoutCandidateRecord {
    pub(crate) fn rehydrate(
        scope: CoreTaskScope,
        candidate: ScoutCandidateDraft,
    ) -> Result<Self, ScoutCandidateRepositoryError> {
        if !draft_matches_scope(&scope, &candidate)
            || !has_valid_candidate_digest(&candidate)
            || !is_sha256_digest(&candidate.provenance_commitment)
        {
            return Err(ScoutCandidateRepositoryError::InvalidCanonicalRecord);
        }
        Ok(Self { scope, candidate })
    }

    #[allow(dead_code)] // Constructed by production repository adapters, not this library core.
    pub(crate) fn key(&self) -> ScoutCandidateRecordKey {
        ScoutCandidateRecordKey::from_candidate(&self.scope, &self.candidate)
    }

    #[allow(dead_code)] // Read by production uniqueness constraints, not this library core.
    pub(crate) fn candidate_id(&self) -> &str {
        &self.candidate.candidate_id
    }
}

/// Ordered all-or-nothing discovery write. It is internal to trusted
/// composition so downstream consumers cannot turn caller-built drafts into
/// persistence requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScoutCandidateBatch {
    scope: CoreTaskScope,
    records: Vec<ScoutCandidateRecord>,
    commitment: String,
}

impl ScoutCandidateBatch {
    pub(crate) fn rehydrate(
        scope: CoreTaskScope,
        candidates: Vec<ScoutCandidateDraft>,
    ) -> Result<Self, ScoutCandidateRepositoryError> {
        if candidates.is_empty() {
            return Err(ScoutCandidateRepositoryError::InvalidCanonicalRecord);
        }
        let mut records = candidates
            .into_iter()
            .map(|candidate| ScoutCandidateRecord::rehydrate(scope.clone(), candidate))
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by(|left, right| canonical_member_key(left).cmp(&canonical_member_key(right)));
        let commitment = batch_commitment(&scope, &records);
        Ok(Self {
            scope,
            records,
            commitment,
        })
    }

    #[allow(dead_code)] // Rechecked by the persistence adapter at write time.
    pub(crate) fn is_valid(&self) -> bool {
        !self.records.is_empty()
            && self.records.iter().all(|record| {
                record.scope == self.scope
                    && ScoutCandidateRecord::rehydrate(
                        record.scope.clone(),
                        record.candidate.clone(),
                    )
                    .is_ok()
            })
            && self
                .records
                .iter()
                .map(ScoutCandidateRecord::candidate_id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.records.len()
            && self
                .records
                .windows(2)
                .all(|pair| canonical_member_key(&pair[0]) < canonical_member_key(&pair[1]))
            && self.commitment == batch_commitment(&self.scope, &self.records)
    }
}

fn canonical_member_key(record: &ScoutCandidateRecord) -> (&str, &str, &str) {
    (
        &record.candidate.candidate_id,
        &record.candidate.digest,
        &record.candidate.provenance_commitment,
    )
}

#[derive(Serialize)]
struct ScoutCandidateBatchCommitment<'a> {
    tenant_id: &'a str,
    job_id: &'a str,
    grant_id: &'a str,
    authority_ref: &'a str,
    candidates: Vec<ScoutCandidateBatchMember<'a>>,
}

#[derive(Serialize)]
struct ScoutCandidateBatchMember<'a> {
    candidate_id: &'a str,
    candidate_digest: &'a str,
    provenance_commitment: &'a str,
}

fn batch_commitment(scope: &CoreTaskScope, records: &[ScoutCandidateRecord]) -> String {
    digest(&ScoutCandidateBatchCommitment {
        tenant_id: scope.tenant_id(),
        job_id: scope.job_id(),
        grant_id: scope.grant_id(),
        authority_ref: scope.authority_ref(),
        candidates: records
            .iter()
            .map(|record| ScoutCandidateBatchMember {
                candidate_id: &record.candidate.candidate_id,
                candidate_digest: &record.candidate.digest,
                provenance_commitment: &record.candidate.provenance_commitment,
            })
            .collect(),
    })
}

/// Idempotent write result. Replaying exact discovery is safe and returns the
/// same canonical durable identity; a differing record under that identity is
/// a conflict, never a silent overwrite.
#[allow(dead_code)] // Concrete durable adapters construct these result variants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScoutCandidateRecordOutcome {
    Recorded,
    AlreadyRecorded,
}

#[allow(dead_code)] // Concrete durable adapters construct storage/conflict errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScoutCandidateRepositoryError {
    InvalidCanonicalRecord,
    CandidateIdentityConflict,
    BatchConflict,
    Storage,
}

/// Durable persistence contract. `record_if_absent` must be one atomic
/// operation: unique on the complete [`ScoutCandidateRecordKey`] and on the
/// full scope plus `candidate_id`; it returns `AlreadyRecorded` only for exact
/// canonical equality. `lookup` is tenant/scope-bound and must not leak an
/// adjacent scope's record.
pub(crate) trait ScoutCandidateRepository {
    fn record_batch_if_absent(
        &mut self,
        batch: ScoutCandidateBatch,
    ) -> Result<ScoutCandidateRecordOutcome, ScoutCandidateRepositoryError>;

    fn lookup(
        &mut self,
        key: &ScoutCandidateRecordKey,
    ) -> Result<Option<ScoutCandidateRecord>, ScoutCandidateRepositoryError>;
}

/// Reference adapter only. It establishes the repository semantics but is not
/// restart-durable and must not be selected by production composition.
#[cfg(any(test, feature = "local-simulation"))]
#[derive(Default)]
pub(crate) struct InMemoryScoutCandidateRepository {
    records: BTreeMap<ScoutCandidateRecordKey, ScoutCandidateRecord>,
    identities: BTreeMap<ScoutCandidateIdentity, ScoutCandidateRecordKey>,
    batches: BTreeMap<String, ScoutCandidateBatch>,
}

#[cfg(any(test, feature = "local-simulation"))]
impl ScoutCandidateRepository for InMemoryScoutCandidateRepository {
    fn record_batch_if_absent(
        &mut self,
        batch: ScoutCandidateBatch,
    ) -> Result<ScoutCandidateRecordOutcome, ScoutCandidateRepositoryError> {
        if !batch.is_valid() {
            return Err(ScoutCandidateRepositoryError::InvalidCanonicalRecord);
        }
        if let Some(existing) = self.batches.get(&batch.commitment) {
            return if existing == &batch {
                Ok(ScoutCandidateRecordOutcome::AlreadyRecorded)
            } else {
                Err(ScoutCandidateRepositoryError::BatchConflict)
            };
        }
        let mut records = self.records.clone();
        let mut identities = self.identities.clone();
        for record in &batch.records {
            let key = record.key();
            if records.contains_key(&key) {
                return Err(ScoutCandidateRepositoryError::BatchConflict);
            }
            let identity = ScoutCandidateIdentity::from_record(record);
            if identities.contains_key(&identity) {
                return Err(ScoutCandidateRepositoryError::CandidateIdentityConflict);
            }
            identities.insert(identity, key.clone());
            records.insert(key, record.clone());
        }
        self.records = records;
        self.identities = identities;
        self.batches.insert(batch.commitment.clone(), batch);
        Ok(ScoutCandidateRecordOutcome::Recorded)
    }

    fn lookup(
        &mut self,
        key: &ScoutCandidateRecordKey,
    ) -> Result<Option<ScoutCandidateRecord>, ScoutCandidateRepositoryError> {
        Ok(self.records.get(key).cloned())
    }
}

/// Restart-test adapter. Independently-created recorder/authority instances
/// can rehydrate through the same shared repository state; it is not a claim
/// of production persistence (the production adapter must implement the port).
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct SharedScoutCandidateRepository {
    inner: Arc<Mutex<InMemoryScoutCandidateRepository>>,
    outcomes: Arc<Mutex<Vec<ScoutCandidateRecordOutcome>>>,
}

#[cfg(test)]
impl SharedScoutCandidateRepository {
    #[cfg(feature = "test-support")]
    fn recorded_outcomes(&self) -> Vec<ScoutCandidateRecordOutcome> {
        self.outcomes
            .lock()
            .expect("test repository is unlocked")
            .clone()
    }
}

#[cfg(test)]
impl ScoutCandidateRepository for SharedScoutCandidateRepository {
    fn record_batch_if_absent(
        &mut self,
        batch: ScoutCandidateBatch,
    ) -> Result<ScoutCandidateRecordOutcome, ScoutCandidateRepositoryError> {
        let outcome = self
            .inner
            .lock()
            .map_err(|_| ScoutCandidateRepositoryError::Storage)?
            .record_batch_if_absent(batch)?;
        self.outcomes
            .lock()
            .map_err(|_| ScoutCandidateRepositoryError::Storage)?
            .push(outcome);
        Ok(outcome)
    }

    fn lookup(
        &mut self,
        key: &ScoutCandidateRecordKey,
    ) -> Result<Option<ScoutCandidateRecord>, ScoutCandidateRepositoryError> {
        self.inner
            .lock()
            .map_err(|_| ScoutCandidateRepositoryError::Storage)?
            .lookup(key)
    }
}

/// Test-only serialized adapter. It stores only encoded batches and rebuilds
/// canonical records on a fresh instance, so restart tests prove the persisted
/// full-batch commitment is checked rather than merely sharing an `Arc`.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct SerializedScoutCandidateRepository {
    encoded_batches: Vec<String>,
}

#[cfg(test)]
impl SerializedScoutCandidateRepository {
    fn snapshot(&self) -> Vec<String> {
        self.encoded_batches.clone()
    }

    fn from_snapshot(encoded_batches: Vec<String>) -> Result<Self, ScoutCandidateRepositoryError> {
        let repository = Self { encoded_batches };
        repository.rebuild()?;
        Ok(repository)
    }

    fn rebuild(&self) -> Result<InMemoryScoutCandidateRepository, ScoutCandidateRepositoryError> {
        let mut rebuilt = InMemoryScoutCandidateRepository::default();
        for encoded in &self.encoded_batches {
            let batch = deserialize_batch(encoded)?;
            rebuilt.record_batch_if_absent(batch)?;
        }
        Ok(rebuilt)
    }
}

#[cfg(test)]
impl ScoutCandidateRepository for SerializedScoutCandidateRepository {
    fn record_batch_if_absent(
        &mut self,
        batch: ScoutCandidateBatch,
    ) -> Result<ScoutCandidateRecordOutcome, ScoutCandidateRepositoryError> {
        let encoded = serialize_batch(&batch)?;
        let mut rebuilt = self.rebuild()?;
        let outcome = rebuilt.record_batch_if_absent(batch)?;
        if outcome == ScoutCandidateRecordOutcome::Recorded {
            self.encoded_batches.push(encoded);
        }
        Ok(outcome)
    }

    fn lookup(
        &mut self,
        key: &ScoutCandidateRecordKey,
    ) -> Result<Option<ScoutCandidateRecord>, ScoutCandidateRepositoryError> {
        self.rebuild()?.lookup(key)
    }
}

#[cfg(test)]
#[derive(Deserialize, Serialize)]
struct PersistedScoutCandidateBatch {
    tenant_id: String,
    job_id: String,
    grant_id: String,
    authority_ref: String,
    candidates: Vec<ScoutCandidateDraft>,
    commitment: String,
}

#[cfg(test)]
fn serialize_batch(batch: &ScoutCandidateBatch) -> Result<String, ScoutCandidateRepositoryError> {
    serde_json::to_string(&PersistedScoutCandidateBatch {
        tenant_id: batch.scope.tenant_id().into(),
        job_id: batch.scope.job_id().into(),
        grant_id: batch.scope.grant_id().into(),
        authority_ref: batch.scope.authority_ref().into(),
        candidates: batch
            .records
            .iter()
            .map(|record| record.candidate.clone())
            .collect(),
        commitment: batch.commitment.clone(),
    })
    .map_err(|_| ScoutCandidateRepositoryError::Storage)
}

#[cfg(test)]
fn deserialize_batch(encoded: &str) -> Result<ScoutCandidateBatch, ScoutCandidateRepositoryError> {
    let persisted: PersistedScoutCandidateBatch = serde_json::from_str(encoded)
        .map_err(|_| ScoutCandidateRepositoryError::InvalidCanonicalRecord)?;
    let scope = CoreTaskScope::new(
        persisted.tenant_id,
        persisted.job_id,
        persisted.grant_id,
        persisted.authority_ref,
    )
    .map_err(|_| ScoutCandidateRepositoryError::InvalidCanonicalRecord)?;
    let batch = ScoutCandidateBatch::rehydrate(scope, persisted.candidates)?;
    if batch.commitment != persisted.commitment {
        return Err(ScoutCandidateRepositoryError::InvalidCanonicalRecord);
    }
    Ok(batch)
}

#[cfg(any(test, feature = "local-simulation"))]
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ScoutCandidateIdentity {
    tenant_id: String,
    job_id: String,
    grant_id: String,
    authority_ref: String,
    candidate_id: String,
}

#[cfg(any(test, feature = "local-simulation"))]
impl ScoutCandidateIdentity {
    fn from_record(record: &ScoutCandidateRecord) -> Self {
        Self {
            tenant_id: record.scope.tenant_id().into(),
            job_id: record.scope.job_id().into(),
            grant_id: record.scope.grant_id().into(),
            authority_ref: record.scope.authority_ref().into(),
            candidate_id: record.candidate.candidate_id.clone(),
        }
    }
}

/// Read-only admission capability for downstream stages such as U14. Its
/// private record prevents construction, deserialization, mutation and draft
/// conversion. Replaying admission is explicitly read-only and may mint an
/// equivalent fresh capability for the same durable record.
///
/// ```compile_fail
/// use improvement_engine_core::autonomous_scout::ScoutCandidateRecord;
/// let _ = ScoutCandidateRecord::rehydrate;
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::autonomous_scout::VerifiedScoutCandidate;
///
/// fn diagnostic_leak(candidate: VerifiedScoutCandidate) -> String {
///     format!("{candidate:?}")
/// }
/// ```
///
/// The capability deliberately has no `Debug` implementation. A generic
/// diagnostic formatter would otherwise recurse into its private canonical
/// record and expose the caller-provided candidate and provenance commitments.
#[derive(Eq, PartialEq)]
pub struct VerifiedScoutCandidate {
    record: ScoutCandidateRecord,
}

impl VerifiedScoutCandidate {
    #[must_use]
    pub fn scope(&self) -> &CoreTaskScope {
        &self.record.scope
    }

    #[must_use]
    pub fn candidate_digest(&self) -> &str {
        &self.record.candidate.digest
    }

    #[must_use]
    pub fn provenance_commitment(&self) -> &str {
        &self.record.candidate.provenance_commitment
    }

    #[must_use]
    pub fn source_snapshot_ref(&self) -> &crate::ArtifactReference {
        &self.record.candidate.source_snapshot_ref
    }

    /// Crate-private U14-E boundary. It rehydrates the canonical U13-A record
    /// before exposing an opaque Frozen E0 capability; a generic U13 draft or
    /// a caller-created E0 provenance can never cross this boundary.
    pub(crate) fn rehydrate_frozen_e0(
        &self,
    ) -> Result<VerifiedFrozenE0ScoutCandidate, FrozenE0ScoutCandidateError> {
        let record = ScoutCandidateRecord::rehydrate(
            self.record.scope.clone(),
            self.record.candidate.clone(),
        )
        .map_err(|_| FrozenE0ScoutCandidateError::InvalidCanonicalRecord)?;
        let Some(e0) = record.candidate.e0_provenance.clone() else {
            return Err(FrozenE0ScoutCandidateError::NotE0Candidate);
        };
        let expected_metric = crate::e0_deterministic_sensor::DiagnosticMetricSpec::from_policy(
            crate::e0_deterministic_sensor::DiagnosticMetricPolicy::TechnicalErrorRateV1,
        );
        if e0.commitment != e0_provenance_digest(&e0)
            || e0.tenant_id != record.scope.tenant_id()
            || e0.job_id != record.scope.job_id()
            || e0.grant_id != record.scope.grant_id()
            || e0.authority_ref != record.scope.authority_ref()
            || e0.source_snapshot_ref != record.candidate.source_snapshot_ref
            || e0.source_digest != record.candidate.source_data_digest
            || e0.source_contract_digest != record.candidate.source_contract_digest
            || e0.transform_digest != record.candidate.transform_digest
            || e0.cutoff_unix_seconds != record.candidate.cutoff_unix_seconds
            || e0.query_receipt_digests != record.candidate.query_receipt_digests
            || e0.signal_commitment != record.candidate.signal_commitment
            || record.candidate.metric_id != expected_metric.metric_id()
            || e0.metric_spec_commitment != expected_metric.commitment()
        {
            return Err(FrozenE0ScoutCandidateError::ProvenanceMismatch);
        }
        Ok(VerifiedFrozenE0ScoutCandidate {
            scope: record.scope,
            candidate_digest: record.candidate.digest,
            provenance_commitment: record.candidate.provenance_commitment,
            e0,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FrozenE0ScoutCandidateError {
    NotE0Candidate,
    InvalidCanonicalRecord,
    ProvenanceMismatch,
}

/// Rehydrated, opaque E0-specific input for U14-E. It is intentionally
/// crate-private and has no conversion from a public draft or receipt.
pub(crate) struct VerifiedFrozenE0ScoutCandidate {
    scope: CoreTaskScope,
    candidate_digest: String,
    provenance_commitment: String,
    e0: E0ScoutCandidateProvenance,
}

impl VerifiedFrozenE0ScoutCandidate {
    #[allow(dead_code)] // Consumed by U15-EQ's crate-private composition root.
    pub(crate) fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }
    pub(crate) fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }
    pub(crate) fn provenance_commitment(&self) -> &str {
        &self.provenance_commitment
    }
    pub(crate) fn e0_commitment(&self) -> &str {
        &self.e0.commitment
    }
    pub(crate) fn source_snapshot_ref(&self) -> ArtifactReference {
        self.e0.source_snapshot_ref.clone()
    }
    #[allow(dead_code)] // Consumed by U15-EQ's crate-private composition root.
    pub(crate) fn cutoff_unix_seconds(&self) -> u64 {
        self.e0.cutoff_unix_seconds
    }
    #[allow(dead_code)] // Consumed by U15-EQ's crate-private composition root.
    pub(crate) fn source_snapshot_binding(&self) -> &str {
        &self.e0.source_snapshot_binding
    }
    #[allow(dead_code)] // Consumed by U15-EQ's crate-private composition root.
    pub(crate) fn availability_profile_digest(&self) -> &str {
        &self.e0.availability_profile_digest
    }
    #[allow(dead_code)] // Consumed by U15-EQ's crate-private composition root.
    pub(crate) fn run_id(&self) -> &str {
        &self.e0.run_id
    }
    pub(crate) fn metric_policy(&self) -> (&str, u16, &str) {
        (
            &self.e0.metric_policy_id,
            self.e0.metric_policy_version,
            &self.e0.metric_semantics,
        )
    }
    pub(crate) fn metric_spec_commitment(&self) -> &str {
        &self.e0.metric_spec_commitment
    }
    pub(crate) fn frozen_bounds_are_consistent(&self) -> bool {
        let Some(total) = self.e0.denominator.checked_add(self.e0.missing) else {
            return false;
        };
        let coverage_basis_points = self
            .e0
            .denominator
            .checked_mul(10_000)
            .and_then(|scaled| scaled.checked_div(total))
            .unwrap_or(0) as u16;
        self.e0.window_start_unix_seconds <= self.e0.window_end_unix_seconds
            && self.e0.window_end_unix_seconds <= self.e0.cutoff_unix_seconds
            && self.e0.numerator <= self.e0.denominator
            && self.e0.coverage_basis_points == coverage_basis_points
            && !self.e0.run_id.is_empty()
            && !self.e0.table.is_empty()
            && !self.e0.field_commitment.is_empty()
            && !self.e0.source_snapshot_binding.is_empty()
            && !self.e0.availability_profile_digest.is_empty()
            && !self.e0.replay_projection_digest.is_empty()
            && !self.e0.source_evidence_digest.is_empty()
            && !self.e0.query_receipt_digests.is_empty()
    }
}

#[cfg(all(test, feature = "test-support"))]
impl VerifiedFrozenE0ScoutCandidate {
    pub(crate) fn all_missing_for_frozen_verifier_test(mut self) -> Self {
        self.e0.numerator = 0;
        self.e0.denominator = 0;
        self.e0.missing = 1;
        self.e0.coverage_basis_points = 0;
        self
    }

    pub(crate) fn inconsistent_coverage_for_frozen_verifier_test(mut self) -> Self {
        self.e0.coverage_basis_points = self.e0.coverage_basis_points.saturating_add(1);
        self
    }

    pub(crate) fn invalid_policy_for_frozen_verifier_test(mut self) -> Self {
        self.e0.metric_policy_id = "other_policy".into();
        self
    }

    pub(crate) fn invalid_semantics_for_frozen_verifier_test(mut self) -> Self {
        self.e0.metric_semantics = "other_semantics".into();
        self
    }

    pub(crate) fn invalid_bounds_for_frozen_verifier_test(mut self) -> Self {
        self.e0.numerator = self.e0.denominator.saturating_add(1);
        self
    }
}

#[cfg(all(test, feature = "test-support"))]
pub(crate) fn corrupt_e0_metric_spec_for_frozen_verifier_test(
    candidate: &mut VerifiedScoutCandidate,
) {
    let e0 = candidate
        .record
        .candidate
        .e0_provenance
        .as_mut()
        .expect("real E0 test candidate");
    e0.metric_spec_commitment = format!("sha256:{}", "0".repeat(64));
    e0.commitment = e0_provenance_digest(e0);
    candidate.record.candidate.digest = candidate_digest(&candidate.record.candidate);
}

#[cfg(all(test, feature = "test-support"))]
pub(crate) fn corrupt_e0_candidate_metric_for_frozen_verifier_test(
    candidate: &mut VerifiedScoutCandidate,
) {
    candidate.record.candidate.metric_id = "other_metric".into();
    candidate.record.candidate.digest = candidate_digest(&candidate.record.candidate);
}

/// Error at the single boundary from a public Scout draft to an opaque,
/// downstream-verifiable capability. It never publishes or promotes anything.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScoutCandidateAdmissionError {
    ScopeMismatch,
    InvalidCandidateDigest,
    UnknownCandidate,
    CandidateMismatch,
    Discovery(ScoutError),
    Repository,
}

/// Trusted control-plane port that mints [`VerifiedScoutCandidate`] only after
/// reading a canonical durable U13 record. No `From<ScoutCandidateDraft>` or
/// deserialization path exists.
pub trait ScoutCandidateAdmissionAuthority {
    fn admit(
        &mut self,
        scope: &CoreTaskScope,
        draft: &ScoutCandidateDraft,
    ) -> Result<VerifiedScoutCandidate, ScoutCandidateAdmissionError>;
}

/// Recorder composition root. It has no draft-insert API: canonical records
/// are created only by running U13 discovery with its opaque expectation.
pub(crate) struct ScoutCandidateRecorder<R> {
    repository: R,
}

impl<R: ScoutCandidateRepository> ScoutCandidateRecorder<R> {
    pub(crate) fn new(repository: R) -> Self {
        Self { repository }
    }

    pub(crate) fn record_discovery(
        &mut self,
        scope: &CoreTaskScope,
        expectation: &ScoutInvocationExpectation,
        signal: &DeterministicSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<ScoutResult, ScoutCandidateAdmissionError> {
        let result = AutonomousScout::discover(scope, expectation, signal, core, model)
            .map_err(ScoutCandidateAdmissionError::Discovery)?;
        if let ScoutResult::Candidates(candidates) = &result {
            let batch = ScoutCandidateBatch::rehydrate(scope.clone(), candidates.clone())
                .map_err(|_| ScoutCandidateAdmissionError::Repository)?;
            self.repository
                .record_batch_if_absent(batch)
                .map_err(|_| ScoutCandidateAdmissionError::Repository)?;
        }
        Ok(result)
    }

    pub(crate) fn record_e0_discovery(
        &mut self,
        scope: &CoreTaskScope,
        evidence: &AuthenticatedE0ScoutSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<ScoutResult, ScoutCandidateAdmissionError> {
        let result = AutonomousScout::discover_e0(scope, evidence, core, model)
            .map_err(ScoutCandidateAdmissionError::Discovery)?;
        if let ScoutResult::Candidates(candidates) = &result {
            let batch = ScoutCandidateBatch::rehydrate(scope.clone(), candidates.clone())
                .map_err(|_| ScoutCandidateAdmissionError::Repository)?;
            self.repository
                .record_batch_if_absent(batch)
                .map_err(|_| ScoutCandidateAdmissionError::Repository)?;
        }
        Ok(result)
    }

    pub(crate) fn into_admission_authority(self) -> TrustedScoutCandidateAdmissionAuthority<R> {
        TrustedScoutCandidateAdmissionAuthority {
            repository: self.repository,
        }
    }
}

/// Internal composition seam for the future service runtime. It is the only
/// production route that couples sealed discovery, all-or-nothing persistence
/// and the downstream admission authority; HTTP/UI consumers receive neither
/// repository nor record constructors.
#[allow(dead_code)]
pub(crate) fn record_scout_discovery<R: ScoutCandidateRepository>(
    repository: R,
    scope: &CoreTaskScope,
    expectation: &ScoutInvocationExpectation,
    signal: &DeterministicSignal,
    core: &CoreTaskReceipt,
    model: &ModelReceipt,
) -> Result<(ScoutResult, TrustedScoutCandidateAdmissionAuthority<R>), ScoutCandidateAdmissionError>
{
    let mut recorder = ScoutCandidateRecorder::new(repository);
    let result = recorder.record_discovery(scope, expectation, signal, core, model)?;
    Ok((result, recorder.into_admission_authority()))
}

/// U13-E's only persistence path. It shares U13-A's canonical batch/lookup
/// boundary, so downstream U14 still receives only `VerifiedScoutCandidate`.
#[allow(dead_code)]
pub(crate) fn record_e0_scout_discovery<R: ScoutCandidateRepository>(
    repository: R,
    scope: &CoreTaskScope,
    evidence: &AuthenticatedE0ScoutSignal,
    core: &CoreTaskReceipt,
    model: &ModelReceipt,
) -> Result<(ScoutResult, TrustedScoutCandidateAdmissionAuthority<R>), ScoutCandidateAdmissionError>
{
    let mut recorder = ScoutCandidateRecorder::new(repository);
    let result = recorder.record_e0_discovery(scope, evidence, core, model)?;
    Ok((result, recorder.into_admission_authority()))
}

/// Authority over a repository selected by production composition.
pub(crate) struct TrustedScoutCandidateAdmissionAuthority<R> {
    repository: R,
}

impl<R: ScoutCandidateRepository> ScoutCandidateAdmissionAuthority
    for TrustedScoutCandidateAdmissionAuthority<R>
{
    fn admit(
        &mut self,
        scope: &CoreTaskScope,
        draft: &ScoutCandidateDraft,
    ) -> Result<VerifiedScoutCandidate, ScoutCandidateAdmissionError> {
        if !draft_matches_scope(scope, draft) {
            return Err(ScoutCandidateAdmissionError::ScopeMismatch);
        }
        if !has_valid_candidate_digest(draft) {
            return Err(ScoutCandidateAdmissionError::InvalidCandidateDigest);
        }
        let key = ScoutCandidateRecordKey::from_candidate(scope, draft);
        let Some(record) = self
            .repository
            .lookup(&key)
            .map_err(|_| ScoutCandidateAdmissionError::Repository)?
        else {
            return Err(ScoutCandidateAdmissionError::UnknownCandidate);
        };
        if record.scope != *scope {
            return Err(ScoutCandidateAdmissionError::ScopeMismatch);
        }
        if record.candidate != *draft {
            return Err(ScoutCandidateAdmissionError::CandidateMismatch);
        }
        Ok(VerifiedScoutCandidate { record })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScoutInvocationExpectation {
    scope: CoreTaskScope,
    signal_digest: String,
    core_binding_digest: String,
    core_attempt_id: String,
    core_run_id: Option<String>,
    core_output_digest: Option<String>,
    model_policy_digest: String,
    model_capability_digest: String,
    model_input_commitment: String,
    model_attempt_id: String,
    model_evidence: String,
    model_output_digest: Option<String>,
}

/// Boundary owned by the improvement-control plane.  It issues an opaque
/// expectation only after it has authenticated the U08/U09/U10 receipts. The
/// Scout deliberately accepts no caller-provided expected strings.
pub trait ScoutInvocationAuthority {
    fn seal(
        &mut self,
        scope: &CoreTaskScope,
        signal: &DeterministicSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<ScoutInvocationExpectation, ScoutError>;
}

/// Explicitly non-production composition used only by the local test harness.
/// Production must provide the control-plane authority through the port above.
#[cfg(any(feature = "test-support", feature = "local-simulation"))]
pub struct NonProductionScoutInvocationAuthority;

#[cfg(any(feature = "test-support", feature = "local-simulation"))]
impl ScoutInvocationAuthority for NonProductionScoutInvocationAuthority {
    fn seal(
        &mut self,
        scope: &CoreTaskScope,
        signal: &DeterministicSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<ScoutInvocationExpectation, ScoutError> {
        if core.scope() != scope
            || model.scope() != scope
            || !signal.has_valid_digest()
            || signal.tenant_id != scope.tenant_id()
            || signal.grant_id != scope.grant_id()
            || signal.authority_ref != scope.authority_ref()
        {
            return Err(ScoutError::EvidenceDenied);
        }
        Ok(ScoutInvocationExpectation {
            scope: scope.clone(),
            signal_digest: signal.digest.clone(),
            core_binding_digest: core.binding_digest().into(),
            core_attempt_id: core.attempt_id().into(),
            core_run_id: core.core_run_id().map(str::to_owned),
            core_output_digest: core.output_digest().map(str::to_owned),
            model_policy_digest: model.policy_digest().into(),
            model_capability_digest: model.capability_digest().into(),
            model_input_commitment: model.input_commitment().into(),
            model_attempt_id: model.attempt_id().into(),
            model_evidence: model.evidence().into(),
            model_output_digest: model.output_digest().map(str::to_owned),
        })
    }
}

#[cfg(feature = "test-support")]
pub fn sealed_expectation_for_test(
    scope: CoreTaskScope,
    signal: &DeterministicSignal,
    core: &CoreTaskReceipt,
    model: &ModelReceipt,
) -> ScoutInvocationExpectation {
    NonProductionScoutInvocationAuthority
        .seal(&scope, signal, core, model)
        .expect("test fixture is valid")
}

/// Opaque U13-E capability. It proves that an immutable U12-E diagnostic was
/// bound to the exact U09/U10 receipts and full task scope; it is not a
/// candidate, a proposal, an authority to record, or an execution permit.
#[derive(Clone)]
pub struct AuthenticatedE0ScoutSignal {
    binding: E0ScoutSignalBinding,
    binding_commitment: String,
    core_binding_digest: String,
    core_attempt_id: String,
    core_run_id: Option<String>,
    core_output_digest: Option<String>,
    model_policy_digest: String,
    model_capability_digest: String,
    model_input_commitment: String,
    model_attempt_id: String,
    model_evidence: String,
    model_output_digest: Option<String>,
}

/// Trusted composition for U12-E → U13-E. It is crate-private so neither a
/// public `QueryResult` nor a caller-created descriptive signal can enter.
pub(crate) struct TrustedE0ScoutComposer;

/// ```compile_fail
/// use improvement_engine_core::autonomous_scout::{AuthenticatedE0ScoutSignal, TrustedE0ScoutComposer};
/// let _ = AuthenticatedE0ScoutSignal {};
/// let _ = TrustedE0ScoutComposer;
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::autonomous_scout::AutonomousScout;
/// use improvement_engine_core::e0_deterministic_sensor::E0DiagnosticSignal;
/// # let scope = todo!(); let core = todo!(); let model = todo!();
/// # let signal: E0DiagnosticSignal = todo!();
/// let _ = AutonomousScout::discover_e0(&scope, &signal, &core, &model);
/// ```
const _E0_SCOUT_CAPABILITY_IS_NOT_PUBLICLY_CONSTRUCTIBLE: () = ();

impl TrustedE0ScoutComposer {
    #[allow(dead_code)]
    pub(crate) fn seal(
        scope: &CoreTaskScope,
        signal: &E0DiagnosticSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<AuthenticatedE0ScoutSignal, ScoutError> {
        let binding = signal.scout_binding();
        if !signal.has_valid_digest()
            || binding.query_receipt_digests.is_empty()
            || binding.tenant_id != scope.tenant_id()
            || binding.grant_id != scope.grant_id()
            || binding.authority_ref != scope.authority_ref()
            || binding.source_snapshot_ref.tenant_id != scope.tenant_id()
            || binding.source_snapshot_binding.is_empty()
            || binding.availability_profile_digest.is_empty()
            || binding.replay_projection_digest.is_empty()
            || binding.source_evidence_digest.is_empty()
            || core.scope() != scope
            || model.scope() != scope
            || core.input_digest() != binding.signal_digest
            || model.input_commitment().is_empty()
            || !model
                .input_commitment()
                .starts_with(&format!("e0_signal:{}:", binding.signal_digest))
            || model.evidence().is_empty()
        {
            return Err(ScoutError::EvidenceDenied);
        }
        Ok(AuthenticatedE0ScoutSignal {
            binding_commitment: digest(&binding),
            binding,
            core_binding_digest: core.binding_digest().to_owned(),
            core_attempt_id: core.attempt_id().to_owned(),
            core_run_id: core.core_run_id().map(str::to_owned),
            core_output_digest: core.output_digest().map(str::to_owned),
            model_policy_digest: model.policy_digest().to_owned(),
            model_capability_digest: model.capability_digest().to_owned(),
            model_input_commitment: model.input_commitment().to_owned(),
            model_attempt_id: model.attempt_id().to_owned(),
            model_evidence: model.evidence().to_owned(),
            model_output_digest: model.output_digest().map(str::to_owned),
        })
    }
}

/// Pure, deterministic publication gate. U09/U10 receipts are supplied by
/// their owners; model output is never copied into a candidate or treated as
/// evidence of causality.
pub struct AutonomousScout;
impl AutonomousScout {
    /// Emits descriptive candidate drafts from only a U12-E-authenticated
    /// capability. This method neither records them nor promotes/releases one.
    pub fn discover_e0(
        scope: &CoreTaskScope,
        evidence: &AuthenticatedE0ScoutSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<ScoutResult, ScoutError> {
        let b = &evidence.binding;
        if b.tenant_id != scope.tenant_id()
            || evidence.binding_commitment != digest(b)
            || b.grant_id != scope.grant_id()
            || b.authority_ref != scope.authority_ref()
            || b.source_snapshot_ref.tenant_id != scope.tenant_id()
            || b.query_receipt_digests.is_empty()
            || core.scope() != scope
            || model.scope() != scope
            || core.input_digest() != b.signal_digest
            || !model
                .input_commitment()
                .starts_with(&format!("e0_signal:{}:", b.signal_digest))
            || core.binding_digest() != evidence.core_binding_digest
            || core.attempt_id() != evidence.core_attempt_id
            || core.core_run_id() != evidence.core_run_id.as_deref()
            || core.output_digest() != evidence.core_output_digest.as_deref()
            || model.policy_digest() != evidence.model_policy_digest
            || model.capability_digest() != evidence.model_capability_digest
            || model.input_commitment() != evidence.model_input_commitment
            || model.attempt_id() != evidence.model_attempt_id
            || model.evidence() != evidence.model_evidence
            || model.output_digest() != evidence.model_output_digest.as_deref()
        {
            return Err(ScoutError::EvidenceDenied);
        }
        if core.outcome() != &CoreTaskOutcome::Succeeded {
            return Ok(ScoutResult::DependencyBlocked {
                reason: "core_task_unknown",
            });
        }
        if model.outcome() != &ModelOutcome::Succeeded {
            return Ok(ScoutResult::DependencyBlocked {
                reason: "model_dependency_unavailable",
            });
        }
        let e0_provenance = E0ScoutCandidateProvenance::from_binding(scope, b);
        if !e0_provenance.is_valid_for(scope, b) {
            return Err(ScoutError::EvidenceDenied);
        }
        let provenance_commitment = digest(&E0CandidateInvocationProvenance {
            e0: &e0_provenance,
            core_binding_digest: core.binding_digest(),
            core_attempt_id: core.attempt_id(),
            core_run_id: core.core_run_id(),
            core_output_digest: core.output_digest(),
            model_policy_digest: model.policy_digest(),
            model_capability_digest: model.capability_digest(),
            model_input_commitment: model.input_commitment(),
            model_attempt_id: model.attempt_id(),
            model_evidence: model.evidence(),
            model_output_digest: model.output_digest(),
        });
        let mut candidates = Vec::new();
        for kind in [
            CandidateKind::Signal,
            CandidateKind::Claim,
            CandidateKind::Opportunity,
        ] {
            let mut draft = ScoutCandidateDraft {
                kind,
                candidate_id: format!(
                    "candidate_{}_{}_{}",
                    b.metric_id,
                    candidate_name(kind),
                    provenance_commitment
                ),
                metric_id: b.metric_id.clone(),
                source_snapshot_ref: b.source_snapshot_ref.clone(),
                tenant_id: scope.tenant_id().to_owned(),
                job_id: scope.job_id().to_owned(),
                grant_id: scope.grant_id().to_owned(),
                authority_ref: scope.authority_ref().to_owned(),
                source_data_digest: b.source_digest.clone(),
                source_contract_digest: b.source_contract_digest.clone(),
                transform_digest: b.transform_digest.clone(),
                cutoff_unix_seconds: b.cutoff_unix_seconds,
                query_receipt_digests: b.query_receipt_digests.clone(),
                signal_commitment: b.signal_digest.clone(),
                e0_provenance: Some(e0_provenance.clone()),
                core_input_commitment: core.input_digest().to_owned(),
                model_input_commitment: model.input_commitment().to_owned(),
                model_capability_digest: model.capability_digest().to_owned(),
                core_binding_digest: core.binding_digest().to_owned(),
                core_attempt_id: core.attempt_id().to_owned(),
                core_run_id: core.core_run_id().map(str::to_owned),
                core_output_digest: core.output_digest().map(str::to_owned),
                model_policy_digest: model.policy_digest().to_owned(),
                model_attempt_id: model.attempt_id().to_owned(),
                model_receipt_evidence: model.evidence().to_owned(),
                model_output_digest: model.output_digest().map(str::to_owned),
                provenance_commitment: provenance_commitment.clone(),
                digest: String::new(),
            };
            draft.digest = candidate_digest(&draft);
            candidates.push(draft);
        }
        Ok(ScoutResult::Candidates(candidates))
    }

    pub fn discover(
        scope: &CoreTaskScope,
        expectation: &ScoutInvocationExpectation,
        signal: &DeterministicSignal,
        core: &CoreTaskReceipt,
        model: &ModelReceipt,
    ) -> Result<ScoutResult, ScoutError> {
        if !signal.has_valid_digest()
            || signal.query_receipts.is_empty()
            || signal.query_receipts.iter().any(|r| {
                !r.has_valid_digest()
                    || r.tenant_id != signal.tenant_id
                    || r.grant_id != signal.grant_id
                    || r.authority_ref != signal.authority_ref
                    || r.source_digest != signal.source_digest
                    || r.source_contract_digest != signal.source_contract_digest
                    || r.transform_digest != signal.transform_digest
                    || r.cutoff_unix_seconds != signal.cutoff_unix_seconds
            })
        {
            return Err(ScoutError::EvidenceDenied);
        }
        if &expectation.scope != scope
            || core.scope() != scope
            || model.scope() != scope
            || signal.tenant_id != scope.tenant_id()
            || signal.grant_id != scope.grant_id()
            || signal.authority_ref != scope.authority_ref()
        {
            return Err(ScoutError::ScopeMismatch);
        }
        if expectation.signal_digest != signal.digest
            || core.input_digest() != signal.digest
            || core.binding_digest() != expectation.core_binding_digest
            || core.attempt_id() != expectation.core_attempt_id
            || core.core_run_id() != expectation.core_run_id.as_deref()
            || core.output_digest() != expectation.core_output_digest.as_deref()
            || model.input_commitment().is_empty()
            || model.policy_digest() != expectation.model_policy_digest
            || model.capability_digest() != expectation.model_capability_digest
            || model.input_commitment() != expectation.model_input_commitment
            || model.attempt_id() != expectation.model_attempt_id
            || model.evidence().is_empty()
            || model.evidence() != expectation.model_evidence
            || model.output_digest() != expectation.model_output_digest.as_deref()
        {
            return Err(ScoutError::EvidenceDenied);
        }
        if core.outcome() != &CoreTaskOutcome::Succeeded {
            return Ok(ScoutResult::DependencyBlocked {
                reason: "core_task_unknown",
            });
        }
        if model.outcome() != &ModelOutcome::Succeeded {
            return Ok(ScoutResult::DependencyBlocked {
                reason: "model_dependency_unavailable",
            });
        }
        let receipts = signal
            .query_receipts
            .iter()
            .map(|r| r.digest.clone())
            .collect::<Vec<_>>();
        let provenance_commitment = digest(&CandidateProvenance {
            tenant_id: scope.tenant_id(),
            job_id: scope.job_id(),
            grant_id: scope.grant_id(),
            authority_ref: scope.authority_ref(),
            signal,
            e0_provenance_commitment: None,
            core_binding_digest: core.binding_digest(),
            core_attempt_id: core.attempt_id(),
            core_run_id: core.core_run_id(),
            core_output_digest: core.output_digest(),
            model_policy_digest: model.policy_digest(),
            model_capability_digest: model.capability_digest(),
            model_input_commitment: model.input_commitment(),
            model_attempt_id: model.attempt_id(),
            model_evidence: model.evidence(),
            model_output_digest: model.output_digest(),
        });
        let mut output = Vec::new();
        for kind in [
            CandidateKind::Signal,
            CandidateKind::Claim,
            CandidateKind::Opportunity,
        ] {
            let mut draft = ScoutCandidateDraft {
                kind,
                candidate_id: format!(
                    "candidate_{}_{}_{}",
                    signal.metric_id,
                    candidate_name(kind),
                    provenance_commitment
                ),
                metric_id: signal.metric_id.clone(),
                source_snapshot_ref: signal.source_snapshot_ref.clone(),
                tenant_id: scope.tenant_id().into(),
                job_id: scope.job_id().into(),
                grant_id: scope.grant_id().into(),
                authority_ref: scope.authority_ref().into(),
                source_data_digest: signal.source_digest.clone(),
                source_contract_digest: signal.source_contract_digest.clone(),
                transform_digest: signal.transform_digest.clone(),
                cutoff_unix_seconds: signal.cutoff_unix_seconds,
                query_receipt_digests: receipts.clone(),
                signal_commitment: signal.digest.clone(),
                e0_provenance: None,
                core_input_commitment: core.input_digest().to_owned(),
                model_input_commitment: model.input_commitment().to_owned(),
                model_capability_digest: model.capability_digest().to_owned(),
                core_binding_digest: core.binding_digest().to_owned(),
                core_attempt_id: core.attempt_id().to_owned(),
                core_run_id: core.core_run_id().map(str::to_owned),
                core_output_digest: core.output_digest().map(str::to_owned),
                model_policy_digest: model.policy_digest().to_owned(),
                model_attempt_id: model.attempt_id().to_owned(),
                model_receipt_evidence: model.evidence().to_owned(),
                model_output_digest: model.output_digest().map(str::to_owned),
                provenance_commitment: provenance_commitment.clone(),
                digest: String::new(),
            };
            draft.digest = candidate_digest(&draft);
            output.push(draft);
        }
        Ok(ScoutResult::Candidates(output))
    }
}
fn candidate_name(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Signal => "signal",
        CandidateKind::Claim => "claim",
        CandidateKind::Opportunity => "opportunity",
    }
}
fn digest<T: Serialize>(value: &T) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("candidate is serializable"))
    )
}

fn draft_matches_scope(scope: &CoreTaskScope, draft: &ScoutCandidateDraft) -> bool {
    draft.tenant_id == scope.tenant_id()
        && draft.job_id == scope.job_id()
        && draft.grant_id == scope.grant_id()
        && draft.authority_ref == scope.authority_ref()
        && draft.source_snapshot_ref.tenant_id == scope.tenant_id()
}

fn has_valid_candidate_digest(draft: &ScoutCandidateDraft) -> bool {
    draft.digest == candidate_digest(draft)
        && draft.e0_provenance.as_ref().is_none_or(|e0| {
            e0.commitment == e0_provenance_digest(e0)
                && e0.tenant_id == draft.tenant_id
                && e0.job_id == draft.job_id
                && e0.grant_id == draft.grant_id
                && e0.authority_ref == draft.authority_ref
                && !e0.run_id.is_empty()
                && !e0.table.is_empty()
                && !e0.field_commitment.is_empty()
                && e0.signal_commitment == draft.signal_commitment
                && e0.source_snapshot_ref == draft.source_snapshot_ref
                && e0.cutoff_unix_seconds == draft.cutoff_unix_seconds
                && e0.source_contract_digest == draft.source_contract_digest
                && e0.source_digest == draft.source_data_digest
                && e0.transform_digest == draft.transform_digest
                && e0.query_receipt_digests == draft.query_receipt_digests
        })
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Computes the candidate commitment with its self-referential `digest` field
/// empty. This is the same canonical convention used by U13 discovery and is
/// checked again at admission before any ledger lookup.
fn candidate_digest(draft: &ScoutCandidateDraft) -> String {
    let mut content = draft.clone();
    content.digest.clear();
    digest(&content)
}

/// Narrow test-only fixture for U14's capability-only boundary. It is kept
/// inside this module because a sibling cannot construct `VerifiedScoutCandidate`.
/// Production composition must obtain the capability through the durable U13-A
/// admission authority instead.
#[cfg(test)]
pub(crate) fn verified_candidate_for_independent_verifier_test() -> VerifiedScoutCandidate {
    let scope = CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a")
        .expect("fixed test scope is valid");
    let mut draft = ScoutCandidateDraft {
        kind: CandidateKind::Claim,
        candidate_id: "candidate_u14_test".into(),
        metric_id: "contact_rate".into(),
        source_snapshot_ref: crate::ArtifactReference {
            tenant_id: "tenant_a".into(),
            id: "018f3a54-7eaf-7c83-8a04-5bf4ec1a9d26".into(),
            revision: 1,
            digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .into(),
        },
        tenant_id: "tenant_a".into(),
        job_id: "job_a".into(),
        grant_id: "grant_a".into(),
        authority_ref: "authority_a".into(),
        source_data_digest:
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        source_contract_digest:
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
        transform_digest: "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            .into(),
        cutoff_unix_seconds: 100,
        query_receipt_digests: vec![
            "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
        ],
        signal_commitment:
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
        e0_provenance: None,
        core_input_commitment:
            "sha256:1111111111111111111111111111111111111111111111111111111111111111".into(),
        model_input_commitment:
            "sha256:2222222222222222222222222222222222222222222222222222222222222222".into(),
        model_capability_digest:
            "sha256:3333333333333333333333333333333333333333333333333333333333333333".into(),
        core_binding_digest:
            "sha256:4444444444444444444444444444444444444444444444444444444444444444".into(),
        core_attempt_id: "attempt_a".into(),
        core_run_id: Some("run_a".into()),
        core_output_digest: Some(
            "sha256:5555555555555555555555555555555555555555555555555555555555555555".into(),
        ),
        model_policy_digest:
            "sha256:6666666666666666666666666666666666666666666666666666666666666666".into(),
        model_attempt_id: "attempt_model_a".into(),
        model_receipt_evidence:
            "sha256:7777777777777777777777777777777777777777777777777777777777777777".into(),
        model_output_digest: Some(
            "sha256:8888888888888888888888888888888888888888888888888888888888888888".into(),
        ),
        provenance_commitment:
            "sha256:9999999999999999999999999999999999999999999999999999999999999999".into(),
        digest: String::new(),
    };
    draft.digest = candidate_digest(&draft);
    let record = ScoutCandidateRecord::rehydrate(scope, draft).expect("test draft is canonical");
    VerifiedScoutCandidate { record }
}

/// Test-only complete U02→U04-B→U08→U12-E→U13-E→U13-A chain for the
/// downstream U14-E contract. Production never receives this fixture.
#[cfg(all(test, feature = "test-support"))]
pub(crate) fn verified_real_e0_candidate_for_frozen_verifier_test() -> VerifiedScoutCandidate {
    candidate_admission_tests::verified_real_e0_candidate_for_frozen_verifier_test()
}

#[cfg(test)]
mod candidate_admission_tests {
    use super::*;
    #[cfg(feature = "test-support")]
    use crate::core_task::{
        CoreTaskBinding, CoreTaskBindingRegistry, CoreTaskInvocation, CoreTaskPort,
        CoreTaskSimulator,
    };
    #[cfg(feature = "test-support")]
    use crate::model_provider::{
        HmacProjectionBroker, ModelBudgetLimits, ModelCapability, ModelInvocation, ModelPolicy,
        ModelPort, ModelProvider, ModelProviderSimulator, ProjectionBrokerPort, RedactionPolicy,
    };

    fn d(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn scope() -> CoreTaskScope {
        CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap()
    }

    #[cfg(feature = "test-support")]
    fn e0_scope(signal: &E0DiagnosticSignal) -> CoreTaskScope {
        let binding = signal.scout_binding();
        CoreTaskScope::new(
            binding.tenant_id,
            "job_e0",
            binding.grant_id,
            binding.authority_ref,
        )
        .unwrap()
    }

    #[cfg(feature = "test-support")]
    fn e0_core(scope: CoreTaskScope, input: &str, attempt: &str) -> CoreTaskReceipt {
        let binding = CoreTaskBinding::new(
            "scout",
            "rel_scout_1",
            "0.5.0",
            "53e729d624c8284e906249df84c1a1df84cc8d40",
        )
        .unwrap();
        let mut port =
            CoreTaskSimulator::new(CoreTaskBindingRegistry::new(vec![binding.clone()]).unwrap());
        port.script_success("core_e0", d('6')).unwrap();
        port.invoke(CoreTaskInvocation::new(scope, binding, attempt, input).unwrap())
            .unwrap()
    }

    #[cfg(feature = "test-support")]
    fn e0_model(scope: CoreTaskScope, attempt: &str, signal_digest: &str) -> ModelReceipt {
        let capability = ModelCapability::new(
            ModelProvider::OpenRouter,
            "https://openrouter.ai/api/v1",
            "openai/gpt-4.1-mini",
            "secret://pulso/key",
            "rev_a",
        )
        .unwrap();
        let policy = ModelPolicy::with_budget(
            "policy_a",
            capability,
            "investigate",
            RedactionPolicy::TokenizeKnownMarkers,
            1,
            100,
            ModelBudgetLimits::new(256, 10, 100).unwrap(),
        )
        .unwrap();
        let mut broker =
            HmacProjectionBroker::new_for_test(b"test-only-projection-authority-key-32b").unwrap();
        let projection = broker
            .authorize_projection(
                &scope,
                &policy,
                format!("signal_digest={signal_digest}\nmetric=e0_technical_error_rate"),
            )
            .unwrap();
        let invocation = ModelInvocation::from_verified_for_e0_signal(
            scope,
            policy,
            attempt,
            projection,
            signal_digest,
        )
        .unwrap();
        let mut port = ModelProviderSimulator::new(invocation.policy().clone());
        port.script_success("ok", "request_e0");
        port.invoke(invocation).unwrap()
    }

    fn candidate(candidate_id: &str) -> ScoutCandidateDraft {
        let scope = scope();
        let mut candidate = ScoutCandidateDraft {
            kind: CandidateKind::Signal,
            candidate_id: candidate_id.into(),
            metric_id: "contact_rate".into(),
            source_snapshot_ref: crate::ArtifactReference {
                tenant_id: scope.tenant_id().into(),
                id: "018f3a54-7eaf-7c83-8a04-5bf4ec1a9d26".into(),
                revision: 1,
                digest: d('a'),
            },
            tenant_id: scope.tenant_id().into(),
            job_id: scope.job_id().into(),
            grant_id: scope.grant_id().into(),
            authority_ref: scope.authority_ref().into(),
            source_data_digest: d('b'),
            source_contract_digest: d('c'),
            transform_digest: d('d'),
            cutoff_unix_seconds: 100,
            query_receipt_digests: vec![d('e')],
            signal_commitment: d('f'),
            e0_provenance: None,
            core_input_commitment: d('f'),
            model_input_commitment: "projection:commitment".into(),
            model_capability_digest: "model-capability:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            core_binding_digest: "core-task:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            core_attempt_id: "core_attempt_a".into(),
            core_run_id: Some("core_run_a".into()),
            core_output_digest: Some(d('1')),
            model_policy_digest: "model-policy:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
            model_attempt_id: "model_attempt_a".into(),
            model_receipt_evidence: "model_operation:succeeded:retries=0".into(),
            model_output_digest: Some("model-output:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into()),
            provenance_commitment: d('2'),
            digest: String::new(),
        };
        candidate.digest = candidate_digest(&candidate);
        candidate
    }

    fn authority_with(
        repository: SharedScoutCandidateRepository,
        candidates: Vec<ScoutCandidateDraft>,
    ) -> TrustedScoutCandidateAdmissionAuthority<SharedScoutCandidateRepository> {
        let batch = ScoutCandidateBatch::rehydrate(scope(), candidates).unwrap();
        let mut recorder = ScoutCandidateRecorder::new(repository);
        assert_eq!(
            recorder.repository.record_batch_if_absent(batch).unwrap(),
            ScoutCandidateRecordOutcome::Recorded
        );
        recorder.into_admission_authority()
    }

    #[cfg(feature = "test-support")]
    pub(super) fn verified_real_e0_candidate_for_frozen_verifier_test() -> VerifiedScoutCandidate {
        let signal = crate::e0_deterministic_sensor::real_signal_for_scout_test();
        let e0_scope = e0_scope(&signal);
        let core = e0_core(
            e0_scope.clone(),
            &signal.scout_binding().signal_digest,
            "attempt_e0_u14",
        );
        let model = e0_model(
            e0_scope.clone(),
            "attempt_model_e0_u14",
            &signal.scout_binding().signal_digest,
        );
        let evidence = TrustedE0ScoutComposer::seal(&e0_scope, &signal, &core, &model)
            .expect("real E0 evidence seals");
        let (result, mut authority) = record_e0_scout_discovery(
            InMemoryScoutCandidateRepository::default(),
            &e0_scope,
            &evidence,
            &core,
            &model,
        )
        .expect("real E0 discovery records");
        let ScoutResult::Candidates(candidates) = result else {
            panic!("real E0 discovery must produce drafts")
        };
        authority
            .admit(&e0_scope, &candidates[0])
            .expect("U13-A admits real E0 candidate")
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn authenticated_e0_signal_records_then_admits_only_through_u13a_boundary() {
        let signal = crate::e0_deterministic_sensor::real_signal_for_scout_test();
        let e0_scope = e0_scope(&signal);
        let core = e0_core(
            e0_scope.clone(),
            &signal.scout_binding().signal_digest,
            "attempt_e0",
        );
        let model = e0_model(
            e0_scope.clone(),
            "attempt_model_e0",
            &signal.scout_binding().signal_digest,
        );
        let evidence = TrustedE0ScoutComposer::seal(&e0_scope, &signal, &core, &model).unwrap();
        let repository = SharedScoutCandidateRepository::default();
        let (result, mut authority) =
            record_e0_scout_discovery(repository.clone(), &e0_scope, &evidence, &core, &model)
                .unwrap();
        let ScoutResult::Candidates(candidates) = result else {
            panic!("expected drafts")
        };
        assert_eq!(candidates.len(), 3);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.e0_provenance.is_some())
        );
        let verified = authority.admit(&e0_scope, &candidates[0]).unwrap();
        assert_eq!(verified.candidate_digest(), candidates[0].digest);
        let mut tampered = candidates[0].clone();
        tampered.e0_provenance.as_mut().unwrap().commitment = d('0');
        tampered.digest = candidate_digest(&tampered);
        assert!(matches!(
            authority.admit(&e0_scope, &tampered),
            Err(ScoutCandidateAdmissionError::InvalidCandidateDigest)
        ));

        let (replayed, _) =
            record_e0_scout_discovery(repository.clone(), &e0_scope, &evidence, &core, &model)
                .unwrap();
        assert_eq!(replayed, ScoutResult::Candidates(candidates));
        assert_eq!(
            repository.recorded_outcomes(),
            vec![
                ScoutCandidateRecordOutcome::Recorded,
                ScoutCandidateRecordOutcome::AlreadyRecorded,
            ]
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn u10_receipt_bound_to_different_e0_signal_is_rejected_before_u13() {
        let signal = crate::e0_deterministic_sensor::real_signal_for_scout_test();
        let e0_scope = e0_scope(&signal);
        let core = e0_core(
            e0_scope.clone(),
            &signal.scout_binding().signal_digest,
            "attempt_e0",
        );
        let other_signal_digest = d('f');
        let model = e0_model(
            e0_scope.clone(),
            "attempt_model_other_signal",
            &other_signal_digest,
        );

        assert!(matches!(
            TrustedE0ScoutComposer::seal(&e0_scope, &signal, &core, &model),
            Err(ScoutError::EvidenceDenied)
        ));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn e0_scope_or_receipt_drift_is_denied_without_recording() {
        let signal = crate::e0_deterministic_sensor::real_signal_for_scout_test();
        let e0_scope = e0_scope(&signal);
        let core = e0_core(
            e0_scope.clone(),
            &signal.scout_binding().signal_digest,
            "attempt_e0",
        );
        let model = e0_model(
            e0_scope.clone(),
            "attempt_model_e0",
            &signal.scout_binding().signal_digest,
        );
        let evidence = TrustedE0ScoutComposer::seal(&e0_scope, &signal, &core, &model).unwrap();
        let changed_core = e0_core(
            e0_scope.clone(),
            &signal.scout_binding().signal_digest,
            "attempt_other",
        );
        assert_eq!(
            AutonomousScout::discover_e0(&e0_scope, &evidence, &changed_core, &model),
            Err(ScoutError::EvidenceDenied)
        );
        let wrong_scope =
            CoreTaskScope::new("tenant_b", "job_e0", "grant_e0", "authority_e0").unwrap();
        assert_eq!(
            AutonomousScout::discover_e0(&wrong_scope, &evidence, &core, &model),
            Err(ScoutError::EvidenceDenied)
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn e0_binding_drift_matrix_fails_closed_before_any_recording() {
        let signal = crate::e0_deterministic_sensor::real_signal_for_scout_test();
        let e0_scope = e0_scope(&signal);
        let core = e0_core(
            e0_scope.clone(),
            &signal.scout_binding().signal_digest,
            "attempt_e0",
        );
        let model = e0_model(
            e0_scope.clone(),
            "attempt_model_e0",
            &signal.scout_binding().signal_digest,
        );
        let evidence = TrustedE0ScoutComposer::seal(&e0_scope, &signal, &core, &model).unwrap();

        macro_rules! assert_drift_denied {
            ($field:ident, $value:expr) => {{
                let mut drifted = evidence.clone();
                drifted.binding.$field = $value;
                let repository = SharedScoutCandidateRepository::default();
                assert!(matches!(
                    record_e0_scout_discovery(
                        repository.clone(),
                        &e0_scope,
                        &drifted,
                        &core,
                        &model,
                    ),
                    Err(ScoutCandidateAdmissionError::Discovery(
                        ScoutError::EvidenceDenied
                    ))
                ));
                assert!(repository.recorded_outcomes().is_empty());
            }};
        }

        // Every value is covered by the capability's canonical binding
        // commitment. No altered raw U04/source/profile/cutoff/query evidence
        // can cross into the U13-A recorder.
        assert_drift_denied!(source_snapshot_binding, d('0'));
        assert_drift_denied!(availability_profile_digest, d('1'));
        assert_drift_denied!(cutoff_unix_seconds, 99);
        assert_drift_denied!(source_digest, d('2'));
        assert_drift_denied!(transform_digest, d('3'));
        assert_drift_denied!(replay_projection_digest, d('4'));
        assert_drift_denied!(source_evidence_digest, d('5'));
        assert_drift_denied!(query_receipt_digests, vec![d('6')]);
    }

    #[test]
    fn admission_requires_a_durable_canonical_record_and_is_read_only_on_replay() {
        let repository = SharedScoutCandidateRepository::default();
        let candidate = candidate("candidate_a");
        let mut authority = authority_with(repository, vec![candidate.clone()]);

        let first = authority.admit(&scope(), &candidate).unwrap();
        let replay = authority.admit(&scope(), &candidate).unwrap();
        assert!(first == replay);
    }

    #[test]
    fn batch_commitment_rehydrates_across_process_instances() {
        let repository = SharedScoutCandidateRepository::default();
        let candidate = candidate("candidate_a");
        let first = authority_with(repository.clone(), vec![candidate.clone()]);
        drop(first);

        let mut restarted = ScoutCandidateRecorder::new(repository).into_admission_authority();
        assert_eq!(
            restarted
                .admit(&scope(), &candidate)
                .unwrap()
                .candidate_digest(),
            candidate.digest
        );
    }

    #[test]
    fn reordered_same_members_replay_the_one_canonical_batch() {
        let mut repository = InMemoryScoutCandidateRepository::default();
        let first = candidate("candidate_a");
        let second = candidate("candidate_b");

        let original =
            ScoutCandidateBatch::rehydrate(scope(), vec![second.clone(), first.clone()]).unwrap();
        assert_eq!(
            repository.record_batch_if_absent(original).unwrap(),
            ScoutCandidateRecordOutcome::Recorded
        );

        let reordered = ScoutCandidateBatch::rehydrate(scope(), vec![first, second]).unwrap();
        assert_eq!(
            repository.record_batch_if_absent(reordered).unwrap(),
            ScoutCandidateRecordOutcome::AlreadyRecorded
        );
    }

    #[test]
    fn serialized_repository_reloads_batch_commitment_and_rejects_corruption() {
        let candidate = candidate("candidate_a");
        let batch = ScoutCandidateBatch::rehydrate(scope(), vec![candidate.clone()]).unwrap();
        let mut repository = SerializedScoutCandidateRepository::default();
        assert_eq!(
            repository.record_batch_if_absent(batch).unwrap(),
            ScoutCandidateRecordOutcome::Recorded
        );

        let snapshot = repository.snapshot();
        let restarted =
            SerializedScoutCandidateRepository::from_snapshot(snapshot.clone()).unwrap();
        let mut authority = TrustedScoutCandidateAdmissionAuthority {
            repository: restarted,
        };
        assert_eq!(
            authority
                .admit(&scope(), &candidate)
                .unwrap()
                .candidate_digest(),
            candidate.digest
        );

        let mut corrupted = snapshot;
        let mut encoded: serde_json::Value = serde_json::from_str(&corrupted[0]).unwrap();
        encoded["commitment"] = serde_json::Value::String(d('0'));
        corrupted[0] = serde_json::to_string(&encoded).unwrap();
        assert!(matches!(
            SerializedScoutCandidateRepository::from_snapshot(corrupted),
            Err(ScoutCandidateRepositoryError::InvalidCanonicalRecord)
        ));
    }

    #[test]
    fn duplicate_member_batch_is_rejected_without_persisting_any_member() {
        let mut repository = InMemoryScoutCandidateRepository::default();
        let duplicate = candidate("candidate_a");
        let other = candidate("candidate_b");
        let batch = ScoutCandidateBatch::rehydrate(
            scope(),
            vec![other.clone(), duplicate.clone(), duplicate],
        )
        .unwrap();

        assert_eq!(
            repository.record_batch_if_absent(batch),
            Err(ScoutCandidateRepositoryError::InvalidCanonicalRecord)
        );
        assert_eq!(
            repository
                .lookup(&ScoutCandidateRecordKey::from_candidate(&scope(), &other))
                .unwrap(),
            None
        );
    }

    #[test]
    fn serialized_repository_rejects_a_corrupted_member_even_with_its_old_batch_commitment() {
        let candidate = candidate("candidate_a");
        let batch = ScoutCandidateBatch::rehydrate(scope(), vec![candidate]).unwrap();
        let mut repository = SerializedScoutCandidateRepository::default();
        repository.record_batch_if_absent(batch).unwrap();

        let mut corrupted = repository.snapshot();
        let mut encoded: serde_json::Value = serde_json::from_str(&corrupted[0]).unwrap();
        encoded["candidates"][0]["metric_id"] = serde_json::Value::String("tampered".into());
        corrupted[0] = serde_json::to_string(&encoded).unwrap();
        assert!(matches!(
            SerializedScoutCandidateRepository::from_snapshot(corrupted),
            Err(ScoutCandidateRepositoryError::InvalidCanonicalRecord)
        ));
    }

    #[test]
    fn batch_conflict_rolls_back_every_member_and_rehashed_public_draft_is_denied() {
        let mut repository = InMemoryScoutCandidateRepository::default();
        let original = candidate("candidate_a");
        let original_batch =
            ScoutCandidateBatch::rehydrate(scope(), vec![original.clone()]).unwrap();
        assert_eq!(
            repository.record_batch_if_absent(original_batch).unwrap(),
            ScoutCandidateRecordOutcome::Recorded
        );

        let mut new_member = candidate("candidate_new");
        new_member.metric_id = "new_metric".into();
        new_member.digest = candidate_digest(&new_member);
        let mut conflicting_member = original.clone();
        conflicting_member.metric_id = "tampered_metric".into();
        conflicting_member.digest = candidate_digest(&conflicting_member);
        let conflicting_batch = ScoutCandidateBatch::rehydrate(
            scope(),
            vec![new_member.clone(), conflicting_member.clone()],
        )
        .unwrap();
        assert_eq!(
            repository.record_batch_if_absent(conflicting_batch),
            Err(ScoutCandidateRepositoryError::CandidateIdentityConflict)
        );
        assert_eq!(
            repository
                .lookup(&ScoutCandidateRecordKey::from_candidate(
                    &scope(),
                    &new_member
                ))
                .unwrap(),
            None
        );

        let mut authority = TrustedScoutCandidateAdmissionAuthority { repository };
        assert!(matches!(
            authority.admit(&scope(), &conflicting_member),
            Err(ScoutCandidateAdmissionError::UnknownCandidate)
        ));
    }

    #[test]
    fn scope_and_provenance_mutation_cannot_cross_the_admission_boundary() {
        let repository = SharedScoutCandidateRepository::default();
        let original = candidate("candidate_a");
        let mut authority = authority_with(repository, vec![original.clone()]);
        let mut rehashed = original.clone();
        rehashed.provenance_commitment = d('9');
        rehashed.digest = candidate_digest(&rehashed);
        assert!(matches!(
            authority.admit(&scope(), &rehashed),
            Err(ScoutCandidateAdmissionError::UnknownCandidate)
        ));
        let other_scope =
            CoreTaskScope::new("tenant_b", "job_a", "grant_a", "authority_a").unwrap();
        assert!(matches!(
            authority.admit(&other_scope, &original),
            Err(ScoutCandidateAdmissionError::ScopeMismatch)
        ));
    }
}
