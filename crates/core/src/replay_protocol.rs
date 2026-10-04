//! State-transition model for frozen and prequential replay.
//!
//! The campaign API accepts a canonical digest-shaped seal token from a
//! trusted caller; its lower-level protocol primitive stores an opaque string.
//! Neither API authenticates an evaluator or proves that the token commits to
//! a particular result. Evaluator rows and per-case outcomes are deliberately
//! outside this protocol boundary.

use crate::replay_clock::ReplayPlan;
use crate::replay_clock::ReplayProtocol;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Eq, PartialEq)]
pub struct CohortLease {
    cohort_id: String,
    opened_at: i64,
    case_ids: Vec<String>,
    revision: u64,
}

impl fmt::Debug for CohortLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CohortLease")
            .field("cohort_id", &"[REDACTED]")
            .field("opened_at", &self.opened_at)
            .field("case_count", &self.case_ids.len())
            .field("revision", &self.revision)
            .finish()
    }
}

impl CohortLease {
    pub fn cohort_id(&self) -> &str {
        &self.cohort_id
    }
    pub fn opened_at(&self) -> i64 {
        self.opened_at
    }
    pub fn case_ids(&self) -> &[String] {
        &self.case_ids
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum ReplayProtocolError {
    EmptyCohortId,
    EmptyCohort,
    EmptyCaseId,
    DuplicateCaseId(String),
    CohortNotSealed(String),
    StaleLease(String),
    ConflictingSeal(String),
    FrozenProtocol,
    InvalidRevisionTransition { active: u64, requested: u64 },
    NonMonotonicOpenedAt { previous: i64, requested: i64 },
    CohortIdConflict(String),
    CaseAlreadyReplayed,
    EmptyUpdateId,
    ConflictingUpdate(String),
    UpdateAlreadyPending(String),
}

impl fmt::Debug for ReplayProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ReplayProtocolError as Error;
        match self {
            Error::EmptyCohortId => formatter.write_str("EmptyCohortId"),
            Error::EmptyCohort => formatter.write_str("EmptyCohort"),
            Error::EmptyCaseId => formatter.write_str("EmptyCaseId"),
            Error::DuplicateCaseId(_) => formatter
                .debug_tuple("DuplicateCaseId")
                .field(&"[REDACTED]")
                .finish(),
            Error::CohortNotSealed(_) => formatter
                .debug_tuple("CohortNotSealed")
                .field(&"[REDACTED]")
                .finish(),
            Error::StaleLease(_) => formatter
                .debug_tuple("StaleLease")
                .field(&"[REDACTED]")
                .finish(),
            Error::ConflictingSeal(_) => formatter
                .debug_tuple("ConflictingSeal")
                .field(&"[REDACTED]")
                .finish(),
            Error::FrozenProtocol => formatter.write_str("FrozenProtocol"),
            Error::InvalidRevisionTransition { active, requested } => formatter
                .debug_struct("InvalidRevisionTransition")
                .field("active", active)
                .field("requested", requested)
                .finish(),
            Error::NonMonotonicOpenedAt {
                previous,
                requested,
            } => formatter
                .debug_struct("NonMonotonicOpenedAt")
                .field("previous", previous)
                .field("requested", requested)
                .finish(),
            Error::CohortIdConflict(_) => formatter
                .debug_tuple("CohortIdConflict")
                .field(&"[REDACTED]")
                .finish(),
            Error::CaseAlreadyReplayed => formatter.write_str("CaseAlreadyReplayed"),
            Error::EmptyUpdateId => formatter.write_str("EmptyUpdateId"),
            Error::ConflictingUpdate(_) => formatter
                .debug_tuple("ConflictingUpdate")
                .field(&"[REDACTED]")
                .finish(),
            Error::UpdateAlreadyPending(_) => formatter
                .debug_tuple("UpdateAlreadyPending")
                .field(&"[REDACTED]")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayUpdate {
    cohort_id: String,
    next_revision: u64,
    activated_at: Option<i64>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ReplayProtocolCheckpoint {
    protocol: ReplayProtocol,
    active_revision: u64,
    last_opened_at: Option<i64>,
    active_lease: Option<CohortLease>,
    sealed_receipt: Option<String>,
    updates: std::collections::BTreeMap<String, ReplayUpdate>,
    pending_update_id: Option<String>,
    seen_cohort_ids: BTreeSet<String>,
    seen_case_ids: BTreeSet<String>,
}

impl fmt::Debug for ReplayProtocolCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplayProtocolCheckpoint")
            .field("protocol", &self.protocol)
            .field("active_revision", &self.active_revision)
            .field("last_opened_at", &self.last_opened_at)
            .field("has_active_lease", &self.active_lease.is_some())
            .field("has_sealed_receipt", &self.sealed_receipt.is_some())
            .field("update_count", &self.updates.len())
            .field("has_pending_update", &self.pending_update_id.is_some())
            .field("seen_cohort_count", &self.seen_cohort_ids.len())
            .field("seen_case_count", &self.seen_case_ids.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ReplayProtocolMachine {
    protocol: ReplayProtocol,
    active_revision: u64,
    last_opened_at: Option<i64>,
    active_lease: Option<CohortLease>,
    sealed_receipt: Option<String>,
    updates: std::collections::BTreeMap<String, ReplayUpdate>,
    pending_update_id: Option<String>,
    seen_cohort_ids: BTreeSet<String>,
    seen_case_ids: BTreeSet<String>,
}

impl fmt::Debug for ReplayProtocolMachine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplayProtocolMachine")
            .field("protocol", &self.protocol)
            .field("active_revision", &self.active_revision)
            .field("last_opened_at", &self.last_opened_at)
            .field("has_active_lease", &self.active_lease.is_some())
            .field("has_sealed_receipt", &self.sealed_receipt.is_some())
            .field("update_count", &self.updates.len())
            .field("has_pending_update", &self.pending_update_id.is_some())
            .field("seen_cohort_count", &self.seen_cohort_ids.len())
            .field("seen_case_count", &self.seen_case_ids.len())
            .finish()
    }
}

/// One indivisible unit of detector work. IDs are available only to the
/// in-process consumer; the public campaign summary contains commitments and
/// aggregate counts rather than per-case or per-event identifiers.
#[derive(Clone, Eq, PartialEq)]
pub struct ReplayCampaignWork {
    campaign_digest: String,
    cohort_index: usize,
    opened_at: i64,
    case_ids: Vec<String>,
    visible_event_ids: Vec<String>,
    revision: u64,
}

impl fmt::Debug for ReplayCampaignWork {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplayCampaignWork")
            .field("campaign_digest", &self.campaign_digest)
            .field("cohort_index", &self.cohort_index)
            .field("opened_at", &self.opened_at)
            .field("case_count", &self.case_ids.len())
            .field("visible_event_count", &self.visible_event_ids.len())
            .field("revision", &self.revision)
            .finish()
    }
}

impl ReplayCampaignWork {
    pub fn cohort_index(&self) -> usize {
        self.cohort_index
    }
    pub fn opened_at(&self) -> i64 {
        self.opened_at
    }
    pub fn case_ids(&self) -> &[String] {
        &self.case_ids
    }
    pub fn visible_event_ids(&self) -> &[String] {
        &self.visible_event_ids
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SealedCohort {
    work: ReplayCampaignWork,
    lease: CohortLease,
    receipt_digest: String,
}

#[derive(Clone, Eq, PartialEq)]
pub enum ReplayCampaignError {
    InvalidSnapshotDigest,
    InvalidConfigDigest,
    InvalidReceiptDigest,
    EmptyPlan,
    CampaignComplete,
    CheckpointBindingMismatch,
    WorkBindingMismatch,
    CheckpointCursorInvalid,
    NoPendingCohort,
    StaleCohort,
    ConflictingSeal,
    Protocol(ReplayProtocolError),
}

impl fmt::Debug for ReplayCampaignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ReplayCampaignError as Error;
        match self {
            Error::InvalidSnapshotDigest => formatter.write_str("InvalidSnapshotDigest"),
            Error::InvalidConfigDigest => formatter.write_str("InvalidConfigDigest"),
            Error::InvalidReceiptDigest => formatter.write_str("InvalidReceiptDigest"),
            Error::EmptyPlan => formatter.write_str("EmptyPlan"),
            Error::CampaignComplete => formatter.write_str("CampaignComplete"),
            Error::CheckpointBindingMismatch => formatter.write_str("CheckpointBindingMismatch"),
            Error::WorkBindingMismatch => formatter.write_str("WorkBindingMismatch"),
            Error::CheckpointCursorInvalid => formatter.write_str("CheckpointCursorInvalid"),
            Error::NoPendingCohort => formatter.write_str("NoPendingCohort"),
            Error::StaleCohort => formatter.write_str("StaleCohort"),
            Error::ConflictingSeal => formatter.write_str("ConflictingSeal"),
            Error::Protocol(error) => formatter.debug_tuple("Protocol").field(error).finish(),
        }
    }
}

/// Process-local checkpoint value. It is not serialized or durable; durable
/// crash recovery requires a versioned store and atomic cursor/receipt commit.
#[derive(Clone, Eq, PartialEq)]
pub struct ReplayCampaignCheckpoint {
    source_snapshot_digest: String,
    run_config_digest: String,
    schedule_digest: String,
    campaign_digest: String,
    initial_revision: u64,
    next_cohort_index: usize,
    pending: Option<(ReplayCampaignWork, CohortLease)>,
    last_sealed: Option<SealedCohort>,
    sealed_receipts: BTreeMap<usize, (String, String)>,
    protocol: ReplayProtocolCheckpoint,
}

impl fmt::Debug for ReplayCampaignCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplayCampaignCheckpoint")
            .field("source_snapshot_digest", &self.source_snapshot_digest)
            .field("run_config_digest", &self.run_config_digest)
            .field("schedule_digest", &self.schedule_digest)
            .field("campaign_digest", &self.campaign_digest)
            .field("initial_revision", &self.initial_revision)
            .field("next_cohort_index", &self.next_cohort_index)
            .field("has_pending_work", &self.pending.is_some())
            .field("has_last_sealed_work", &self.last_sealed.is_some())
            .field("sealed_cohort_count", &self.sealed_receipts.len())
            .field("protocol", &self.protocol)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ReplayCampaign {
    plan: ReplayPlan,
    source_snapshot_digest: String,
    run_config_digest: String,
    campaign_digest: String,
    initial_revision: u64,
    next_cohort_index: usize,
    pending: Option<(ReplayCampaignWork, CohortLease)>,
    last_sealed: Option<SealedCohort>,
    sealed_receipts: BTreeMap<usize, (String, String)>,
    protocol: ReplayProtocolMachine,
}

impl fmt::Debug for ReplayCampaign {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplayCampaign")
            .field("campaign_digest", &self.campaign_digest)
            .field("schedule_digest", &self.plan.schedule_digest())
            .field("next_cohort_index", &self.next_cohort_index)
            .field("has_pending_work", &self.pending.is_some())
            .field("has_last_sealed_work", &self.last_sealed.is_some())
            .field("sealed_cohort_count", &self.sealed_receipts.len())
            .field("protocol", &self.protocol)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReplayCampaignSummary {
    campaign_digest: String,
    schedule_digest: String,
    protocol: &'static str,
    cohort_count: usize,
    sealed_cohort_count: usize,
    active_revision: u64,
    complete: bool,
}

impl ReplayCampaignSummary {
    pub fn campaign_digest(&self) -> &str {
        &self.campaign_digest
    }
    pub fn schedule_digest(&self) -> &str {
        &self.schedule_digest
    }
    pub fn protocol(&self) -> &'static str {
        self.protocol
    }
    pub fn cohort_count(&self) -> usize {
        self.cohort_count
    }
    pub fn sealed_cohort_count(&self) -> usize {
        self.sealed_cohort_count
    }
    pub fn active_revision(&self) -> u64 {
        self.active_revision
    }
    pub fn is_complete(&self) -> bool {
        self.complete
    }
}

impl ReplayCampaign {
    pub fn new(
        plan: ReplayPlan,
        source_snapshot_digest: String,
        run_config_digest: String,
        initial_revision: u64,
    ) -> Result<Self, ReplayCampaignError> {
        if !canonical_sha256(&source_snapshot_digest) {
            return Err(ReplayCampaignError::InvalidSnapshotDigest);
        }
        if !canonical_sha256(&run_config_digest) {
            return Err(ReplayCampaignError::InvalidConfigDigest);
        }
        if plan.cohorts().is_empty() {
            return Err(ReplayCampaignError::EmptyPlan);
        }
        let campaign_digest = campaign_digest(
            &source_snapshot_digest,
            &run_config_digest,
            plan.schedule_digest(),
            plan.protocol(),
            initial_revision,
        );
        let protocol = ReplayProtocolMachine::new(plan.protocol(), initial_revision);
        Ok(Self {
            plan,
            source_snapshot_digest,
            run_config_digest,
            campaign_digest,
            initial_revision,
            next_cohort_index: 0,
            pending: None,
            last_sealed: None,
            sealed_receipts: BTreeMap::new(),
            protocol,
        })
    }

    /// Returns the same work item on retry until it is sealed; it never skips
    /// an unsealed cohort or starts more than one cohort at a time.
    pub fn next_work(&mut self) -> Result<Option<ReplayCampaignWork>, ReplayCampaignError> {
        if let Some((work, _)) = &self.pending {
            return Ok(Some(work.clone()));
        }
        let Some(cohort) = self.plan.cohorts().get(self.next_cohort_index) else {
            return Ok(None);
        };
        let case_ids = cohort
            .case_ids()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let lease = self
            .protocol
            .begin_cohort(
                &format!("cohort-{:08}", self.next_cohort_index),
                cohort.opened_at(),
                &case_ids,
            )
            .map_err(ReplayCampaignError::Protocol)?;
        let work = ReplayCampaignWork {
            campaign_digest: self.campaign_digest.clone(),
            cohort_index: self.next_cohort_index,
            opened_at: cohort.opened_at(),
            case_ids: cohort.case_ids().to_vec(),
            visible_event_ids: cohort.visible_event_ids().to_vec(),
            revision: lease.revision(),
        };
        self.pending = Some((work.clone(), lease));
        Ok(Some(work))
    }

    /// The evaluator stays outside this coordinator. A trusted caller may
    /// advance it by supplying a canonical digest-shaped token. This method
    /// does not authenticate the evaluator or verify that the token commits to
    /// this work; a future evaluator adapter must enforce that contract.
    pub fn seal(
        &mut self,
        work: &ReplayCampaignWork,
        evaluator_receipt_digest: &str,
    ) -> Result<(), ReplayCampaignError> {
        if work.campaign_digest != self.campaign_digest {
            return Err(ReplayCampaignError::WorkBindingMismatch);
        }
        if !canonical_sha256(evaluator_receipt_digest) {
            return Err(ReplayCampaignError::InvalidReceiptDigest);
        }
        if let Some((sealed_work_digest, sealed_receipt_digest)) =
            self.sealed_receipts.get(&work.cohort_index)
        {
            if sealed_work_digest != &work_digest(work) {
                return Err(ReplayCampaignError::StaleCohort);
            }
            return if sealed_receipt_digest == evaluator_receipt_digest {
                Ok(())
            } else {
                Err(ReplayCampaignError::ConflictingSeal)
            };
        }
        if self.pending.is_none() {
            return Err(ReplayCampaignError::NoPendingCohort);
        }
        let (pending_work, lease) = self.pending.as_ref().expect("checked pending");
        if pending_work != work {
            return Err(ReplayCampaignError::StaleCohort);
        }
        self.protocol
            .seal_cohort(lease, evaluator_receipt_digest)
            .map_err(ReplayCampaignError::Protocol)?;
        self.last_sealed = Some(SealedCohort {
            work: pending_work.clone(),
            lease: lease.clone(),
            receipt_digest: evaluator_receipt_digest.to_owned(),
        });
        self.sealed_receipts.insert(
            work.cohort_index,
            (work_digest(work), evaluator_receipt_digest.to_owned()),
        );
        self.pending = None;
        self.next_cohort_index += 1;
        Ok(())
    }

    /// Queues a prequential revision against the last sealed cohort. Frozen
    /// campaigns reject updates in the underlying protocol machine.
    pub fn propose_revision(
        &mut self,
        update_id: &str,
        next_revision: u64,
    ) -> Result<(), ReplayCampaignError> {
        if self.next_cohort_index == self.plan.cohorts().len() {
            return Err(ReplayCampaignError::CampaignComplete);
        }
        let sealed = self
            .last_sealed
            .as_ref()
            .ok_or(ReplayCampaignError::NoPendingCohort)?;
        self.protocol
            .propose_update(&sealed.lease, update_id, next_revision)
            .map_err(ReplayCampaignError::Protocol)
    }

    pub fn checkpoint(&self) -> ReplayCampaignCheckpoint {
        ReplayCampaignCheckpoint {
            source_snapshot_digest: self.source_snapshot_digest.clone(),
            run_config_digest: self.run_config_digest.clone(),
            schedule_digest: self.plan.schedule_digest().to_owned(),
            campaign_digest: self.campaign_digest.clone(),
            initial_revision: self.initial_revision,
            next_cohort_index: self.next_cohort_index,
            pending: self.pending.clone(),
            last_sealed: self.last_sealed.clone(),
            sealed_receipts: self.sealed_receipts.clone(),
            protocol: self.protocol.checkpoint(),
        }
    }

    pub fn restore(
        plan: ReplayPlan,
        source_snapshot_digest: String,
        run_config_digest: String,
        checkpoint: ReplayCampaignCheckpoint,
    ) -> Result<Self, ReplayCampaignError> {
        let mut campaign = Self::new(
            plan,
            source_snapshot_digest,
            run_config_digest,
            checkpoint.initial_revision,
        )?;
        if campaign.campaign_digest != checkpoint.campaign_digest
            || campaign.source_snapshot_digest != checkpoint.source_snapshot_digest
            || campaign.run_config_digest != checkpoint.run_config_digest
            || campaign.plan.schedule_digest() != checkpoint.schedule_digest
        {
            return Err(ReplayCampaignError::CheckpointBindingMismatch);
        }
        if checkpoint.next_cohort_index > campaign.plan.cohorts().len()
            || checkpoint.sealed_receipts.len() != checkpoint.next_cohort_index
            || checkpoint
                .sealed_receipts
                .keys()
                .copied()
                .ne(0..checkpoint.next_cohort_index)
            || checkpoint.pending.is_some()
                != (checkpoint.next_cohort_index < campaign.plan.cohorts().len()
                    && checkpoint.protocol.active_lease.is_some()
                    && checkpoint.protocol.sealed_receipt.is_none())
        {
            return Err(ReplayCampaignError::CheckpointCursorInvalid);
        }
        campaign.next_cohort_index = checkpoint.next_cohort_index;
        campaign.pending = checkpoint.pending;
        campaign.last_sealed = checkpoint.last_sealed;
        campaign.sealed_receipts = checkpoint.sealed_receipts;
        campaign.protocol = ReplayProtocolMachine::restore(checkpoint.protocol);
        Ok(campaign)
    }

    pub fn summary(&self) -> ReplayCampaignSummary {
        ReplayCampaignSummary {
            campaign_digest: self.campaign_digest.clone(),
            schedule_digest: self.plan.schedule_digest().to_owned(),
            protocol: match self.plan.protocol() {
                ReplayProtocol::Frozen => "frozen",
                ReplayProtocol::Prequential => "prequential",
            },
            cohort_count: self.plan.cohorts().len(),
            sealed_cohort_count: self.next_cohort_index,
            active_revision: self.protocol.active_revision(),
            complete: self.next_cohort_index == self.plan.cohorts().len() && self.pending.is_none(),
        }
    }
}

fn canonical_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn campaign_digest(
    source_snapshot_digest: &str,
    run_config_digest: &str,
    schedule_digest: &str,
    protocol: ReplayProtocol,
    initial_revision: u64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pulso-replay-campaign-v1\0");
    for value in [source_snapshot_digest, run_config_digest, schedule_digest] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.update([match protocol {
        ReplayProtocol::Frozen => 0,
        ReplayProtocol::Prequential => 1,
    }]);
    hasher.update(initial_revision.to_be_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn work_digest(work: &ReplayCampaignWork) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pulso-replay-campaign-work-v1\0");
    hasher.update((work.campaign_digest.len() as u64).to_be_bytes());
    hasher.update(work.campaign_digest.as_bytes());
    hasher.update((work.cohort_index as u64).to_be_bytes());
    hasher.update(work.opened_at.to_be_bytes());
    hasher.update(work.revision.to_be_bytes());
    for collection in [&work.case_ids, &work.visible_event_ids] {
        hasher.update((collection.len() as u64).to_be_bytes());
        for value in collection {
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}

impl ReplayProtocolMachine {
    pub fn new(protocol: ReplayProtocol, initial_revision: u64) -> Self {
        Self {
            protocol,
            active_revision: initial_revision,
            last_opened_at: None,
            active_lease: None,
            sealed_receipt: None,
            updates: std::collections::BTreeMap::new(),
            pending_update_id: None,
            seen_cohort_ids: BTreeSet::new(),
            seen_case_ids: BTreeSet::new(),
        }
    }

    pub fn active_revision(&self) -> u64 {
        self.active_revision
    }

    pub fn begin_cohort(
        &mut self,
        cohort_id: &str,
        opened_at: i64,
        case_ids: &[&str],
    ) -> Result<CohortLease, ReplayProtocolError> {
        if cohort_id.trim().is_empty() {
            return Err(ReplayProtocolError::EmptyCohortId);
        }
        if case_ids.is_empty() {
            return Err(ReplayProtocolError::EmptyCohort);
        }
        let mut normalized_ids = case_ids
            .iter()
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>();
        if normalized_ids.iter().any(|id| id.trim().is_empty()) {
            return Err(ReplayProtocolError::EmptyCaseId);
        }
        let mut unique_ids = BTreeSet::new();
        for id in &normalized_ids {
            if !unique_ids.insert(id.as_str()) {
                return Err(ReplayProtocolError::DuplicateCaseId(id.clone()));
            }
        }
        normalized_ids.sort();

        if let Some(active) = &self.active_lease {
            if active.cohort_id == cohort_id {
                if active.opened_at == opened_at && active.case_ids == normalized_ids {
                    return Ok(active.clone());
                }
                return Err(ReplayProtocolError::CohortIdConflict(cohort_id.to_owned()));
            }
            if self.sealed_receipt.is_none() {
                return Err(ReplayProtocolError::CohortNotSealed(
                    active.cohort_id.clone(),
                ));
            }
        }
        if self.seen_cohort_ids.contains(cohort_id) {
            return Err(ReplayProtocolError::CohortIdConflict(cohort_id.to_owned()));
        }
        if normalized_ids
            .iter()
            .any(|case_id| self.seen_case_ids.contains(case_id))
        {
            return Err(ReplayProtocolError::CaseAlreadyReplayed);
        }
        if let Some(previous) = self.last_opened_at {
            if opened_at <= previous {
                return Err(ReplayProtocolError::NonMonotonicOpenedAt {
                    previous,
                    requested: opened_at,
                });
            }
        }

        if let Some(update_id) = self.pending_update_id.take() {
            let update = self
                .updates
                .get_mut(&update_id)
                .expect("pending update must have a record");
            self.active_revision = update.next_revision;
            update.activated_at = Some(opened_at);
        }

        self.seen_case_ids.extend(normalized_ids.iter().cloned());
        let lease = CohortLease {
            cohort_id: cohort_id.to_owned(),
            opened_at,
            case_ids: normalized_ids,
            revision: self.active_revision,
        };
        self.seen_cohort_ids.insert(cohort_id.to_owned());
        self.last_opened_at = Some(opened_at);
        self.active_lease = Some(lease.clone());
        self.sealed_receipt = None;
        Ok(lease)
    }

    pub fn seal_cohort(
        &mut self,
        lease: &CohortLease,
        evaluator_receipt_digest: &str,
    ) -> Result<(), ReplayProtocolError> {
        let active = self
            .active_lease
            .as_ref()
            .filter(|active| *active == lease)
            .ok_or_else(|| ReplayProtocolError::StaleLease(lease.cohort_id.clone()))?;
        let _ = active;
        match &self.sealed_receipt {
            Some(existing) if existing == evaluator_receipt_digest => Ok(()),
            Some(_) => Err(ReplayProtocolError::ConflictingSeal(
                lease.cohort_id.clone(),
            )),
            None => {
                self.sealed_receipt = Some(evaluator_receipt_digest.to_owned());
                Ok(())
            }
        }
    }

    pub fn propose_update(
        &mut self,
        lease: &CohortLease,
        update_id: &str,
        next_revision: u64,
    ) -> Result<(), ReplayProtocolError> {
        if update_id.trim().is_empty() {
            return Err(ReplayProtocolError::EmptyUpdateId);
        }
        if let Some(existing) = self.updates.get(update_id) {
            if existing.cohort_id == lease.cohort_id && existing.next_revision == next_revision {
                return Ok(());
            }
            return Err(ReplayProtocolError::ConflictingUpdate(update_id.to_owned()));
        }
        self.active_lease
            .as_ref()
            .filter(|active| *active == lease)
            .ok_or_else(|| ReplayProtocolError::StaleLease(lease.cohort_id.clone()))?;
        if self.sealed_receipt.is_none() {
            return Err(ReplayProtocolError::CohortNotSealed(
                lease.cohort_id.clone(),
            ));
        }
        if self.protocol == ReplayProtocol::Frozen {
            return Err(ReplayProtocolError::FrozenProtocol);
        }
        if next_revision <= self.active_revision {
            return Err(ReplayProtocolError::InvalidRevisionTransition {
                active: self.active_revision,
                requested: next_revision,
            });
        }
        if next_revision != self.active_revision + 1 {
            return Err(ReplayProtocolError::InvalidRevisionTransition {
                active: self.active_revision,
                requested: next_revision,
            });
        }
        if let Some(pending_id) = &self.pending_update_id {
            return Err(ReplayProtocolError::UpdateAlreadyPending(
                pending_id.clone(),
            ));
        }
        self.updates.insert(
            update_id.to_owned(),
            ReplayUpdate {
                cohort_id: lease.cohort_id.clone(),
                next_revision,
                activated_at: None,
            },
        );
        self.pending_update_id = Some(update_id.to_owned());
        Ok(())
    }

    pub fn checkpoint(&self) -> ReplayProtocolCheckpoint {
        ReplayProtocolCheckpoint {
            protocol: self.protocol,
            active_revision: self.active_revision,
            last_opened_at: self.last_opened_at,
            active_lease: self.active_lease.clone(),
            sealed_receipt: self.sealed_receipt.clone(),
            updates: self.updates.clone(),
            pending_update_id: self.pending_update_id.clone(),
            seen_cohort_ids: self.seen_cohort_ids.clone(),
            seen_case_ids: self.seen_case_ids.clone(),
        }
    }

    pub fn restore(checkpoint: ReplayProtocolCheckpoint) -> Self {
        Self {
            protocol: checkpoint.protocol,
            active_revision: checkpoint.active_revision,
            last_opened_at: checkpoint.last_opened_at,
            active_lease: checkpoint.active_lease,
            sealed_receipt: checkpoint.sealed_receipt,
            updates: checkpoint.updates,
            pending_update_id: checkpoint.pending_update_id,
            seen_cohort_ids: checkpoint.seen_cohort_ids,
            seen_case_ids: checkpoint.seen_case_ids,
        }
    }
}
