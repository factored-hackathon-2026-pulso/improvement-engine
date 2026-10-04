//! State-transition model for frozen and prequential replay.
//!
//! The evaluator supplies only an opaque sealed-result digest; evaluator rows
//! and per-case outcomes are deliberately outside this protocol boundary.

use crate::replay_clock::ReplayProtocol;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CohortLease {
    cohort_id: String,
    opened_at: i64,
    case_ids: Vec<String>,
    revision: u64,
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

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayUpdate {
    cohort_id: String,
    next_revision: u64,
    activated_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
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
        let unique_ids = normalized_ids.iter().collect::<BTreeSet<_>>();
        if unique_ids.len() != normalized_ids.len() {
            let duplicate = normalized_ids
                .iter()
                .find(|id| normalized_ids.iter().filter(|other| *other == *id).count() > 1)
                .expect("duplicate id exists");
            return Err(ReplayProtocolError::DuplicateCaseId(duplicate.clone()));
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
