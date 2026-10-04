//! Versioned durable checkpoints for the source-agnostic replay coordinator.
//!
//! The wire document stores commitments and protocol metadata only. Replay
//! input IDs stay in the caller's immutable source snapshot and are never
//! copied into the checkpoint payload.

use super::{ReplayCampaign, ReplayCampaignError, campaign_digest, canonical_sha256, work_digest};
use crate::replay_clock::{ReplayPlan, ReplayProtocol};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const WIRE_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointConflict {
    Storage,
    CompareAndSwap,
    ConflictingCommit,
    InvalidCheckpoint,
    Campaign(ReplayCampaignError),
}

impl From<ReplayCampaignError> for CheckpointConflict {
    fn from(error: ReplayCampaignError) -> Self {
        Self::Campaign(error)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
struct CheckpointWire {
    wire_version: u16,
    campaign_digest: String,
    schedule_digest: String,
    source_snapshot_digest: String,
    run_config_digest: String,
    protocol: String,
    initial_revision: u64,
    next_cohort_index: usize,
    active_revision: u64,
    pending_work_digest: Option<String>,
    seals: Vec<SealWire>,
    updates: Vec<UpdateWire>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
struct SealWire {
    cohort_index: usize,
    work_digest: String,
    receipt_digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
struct UpdateWire {
    cohort_index: usize,
    update_id_digest: String,
    next_revision: u64,
    activated_at: Option<i64>,
}

/// PostgreSQL checkpoint repository. Each commit appends a complete canonical
/// wire snapshot and advances its head in the same transaction using a
/// revision+cursor compare-and-swap.
pub struct PostgresReplayCampaignStore;

impl PostgresReplayCampaignStore {
    /// Encode the campaign without serializing its plan IDs or private leases.
    pub fn encode(campaign: &ReplayCampaign) -> Result<Vec<u8>, CheckpointConflict> {
        let wire = CheckpointWire::from_campaign(campaign)?;
        serde_json::to_vec(&wire).map_err(|_| CheckpointConflict::InvalidCheckpoint)
    }

    /// Decode only canonical v1 documents and rebuild state by replaying each
    /// committed cohort seal and revision update against the exact supplied
    /// plan/source/config bindings.
    pub fn decode(
        bytes: &[u8],
        plan: ReplayPlan,
        source_snapshot_digest: &str,
        run_config_digest: &str,
    ) -> Result<ReplayCampaign, CheckpointConflict> {
        let wire: CheckpointWire =
            serde_json::from_slice(bytes).map_err(|_| CheckpointConflict::InvalidCheckpoint)?;
        if wire.wire_version != WIRE_VERSION
            || serde_json::to_vec(&wire).map_err(|_| CheckpointConflict::InvalidCheckpoint)?
                != bytes
            || !canonical_sha256(&wire.campaign_digest)
            || !canonical_sha256(&wire.schedule_digest)
            || !canonical_sha256(&wire.source_snapshot_digest)
            || !canonical_sha256(&wire.run_config_digest)
        {
            return Err(CheckpointConflict::InvalidCheckpoint);
        }
        let expected_protocol = match plan.protocol() {
            ReplayProtocol::Frozen => "frozen",
            ReplayProtocol::Prequential => "prequential",
        };
        if wire.protocol != expected_protocol
            || wire.schedule_digest != plan.schedule_digest()
            || wire.source_snapshot_digest != source_snapshot_digest
            || wire.run_config_digest != run_config_digest
            || campaign_digest(
                source_snapshot_digest,
                run_config_digest,
                plan.schedule_digest(),
                plan.protocol(),
                wire.initial_revision,
            ) != wire.campaign_digest
        {
            return Err(CheckpointConflict::Campaign(
                ReplayCampaignError::CheckpointBindingMismatch,
            ));
        }
        if wire.next_cohort_index != wire.seals.len()
            || wire
                .seals
                .iter()
                .enumerate()
                .any(|(index, seal)| index != seal.cohort_index)
            || wire
                .updates
                .iter()
                .any(|update| update.cohort_index >= wire.next_cohort_index)
        {
            return Err(CheckpointConflict::InvalidCheckpoint);
        }

        let mut campaign = ReplayCampaign::new(
            plan,
            source_snapshot_digest.to_owned(),
            run_config_digest.to_owned(),
            wire.initial_revision,
        )?;
        for seal in &wire.seals {
            let work = campaign
                .next_work()?
                .ok_or(CheckpointConflict::InvalidCheckpoint)?;
            if work_digest(&work) != seal.work_digest {
                return Err(CheckpointConflict::InvalidCheckpoint);
            }
            campaign.seal(&work, &seal.receipt_digest)?;
            if let Some(update) = wire
                .updates
                .iter()
                .find(|update| update.cohort_index == seal.cohort_index)
            {
                campaign
                    .propose_persisted_revision(&update.update_id_digest, update.next_revision)?;
            }
        }
        if wire.pending_work_digest.is_some() {
            let pending = campaign
                .next_work()?
                .ok_or(CheckpointConflict::InvalidCheckpoint)?;
            if Some(work_digest(&pending)) != wire.pending_work_digest {
                return Err(CheckpointConflict::InvalidCheckpoint);
            }
        }
        let rebuilt = CheckpointWire::from_campaign(&campaign)?;
        if rebuilt != wire {
            return Err(CheckpointConflict::InvalidCheckpoint);
        }
        Ok(campaign)
    }

    /// Commit an immutable checkpoint revision. `expected_revision` and
    /// `expected_cursor` describe the caller's last observed head. A retry of
    /// the exact resulting payload is idempotent; a different payload at that
    /// revision is rejected.
    pub fn commit(
        client: &mut postgres::Client,
        campaign: &ReplayCampaign,
        expected_revision: u64,
        expected_cursor: usize,
    ) -> Result<u64, CheckpointConflict> {
        let payload = Self::encode(campaign)?;
        let payload_digest = digest_bytes(&payload);
        let wire: CheckpointWire =
            serde_json::from_slice(&payload).map_err(|_| CheckpointConflict::InvalidCheckpoint)?;
        if wire.next_cohort_index < expected_cursor
            || wire.next_cohort_index > expected_cursor.saturating_add(1)
        {
            return Err(CheckpointConflict::CompareAndSwap);
        }
        let expected_revision_i64 =
            i64::try_from(expected_revision).map_err(|_| CheckpointConflict::CompareAndSwap)?;
        let expected_cursor_i64 =
            i64::try_from(expected_cursor).map_err(|_| CheckpointConflict::CompareAndSwap)?;
        let next_cursor_i64 = i64::try_from(wire.next_cohort_index)
            .map_err(|_| CheckpointConflict::InvalidCheckpoint)?;
        let next_revision = expected_revision_i64
            .checked_add(1)
            .ok_or(CheckpointConflict::CompareAndSwap)?;

        let mut transaction = client
            .transaction()
            .map_err(|_| CheckpointConflict::Storage)?;
        transaction
            .execute(
                "INSERT INTO pulso_replay_campaign_checkpoint_heads \
                 (campaign_digest, current_revision, cursor, payload_digest) \
                 VALUES ($1, 0, 0, $2) ON CONFLICT (campaign_digest) DO NOTHING",
                &[&wire.campaign_digest, &payload_digest],
            )
            .map_err(|_| CheckpointConflict::Storage)?;
        let head = transaction
            .query_one(
                "SELECT current_revision, cursor, payload_digest \
                 FROM pulso_replay_campaign_checkpoint_heads \
                 WHERE campaign_digest=$1 FOR UPDATE",
                &[&wire.campaign_digest],
            )
            .map_err(|_| CheckpointConflict::Storage)?;
        let current_revision: i64 = head.get(0);
        let current_cursor: i64 = head.get(1);
        let current_digest: String = head.get(2);

        if current_revision == next_revision {
            if current_digest == payload_digest {
                transaction
                    .commit()
                    .map_err(|_| CheckpointConflict::Storage)?;
                return Ok(next_revision as u64);
            }
            if same_cursor_seal_conflicts(
                &mut transaction,
                &wire.campaign_digest,
                current_revision,
                &payload,
            )? {
                return Err(CheckpointConflict::ConflictingCommit);
            }
            return Err(CheckpointConflict::CompareAndSwap);
        }
        if current_revision != expected_revision_i64 || current_cursor != expected_cursor_i64 {
            return Err(CheckpointConflict::CompareAndSwap);
        }
        if current_revision > 0 {
            validate_journal_extension(
                &mut transaction,
                &wire.campaign_digest,
                current_revision,
                &wire,
            )?;
        }

        transaction
            .execute(
                "INSERT INTO pulso_replay_campaign_checkpoint_revisions \
                 (campaign_digest, revision, cursor, wire_version, checkpoint_payload, payload_digest) \
                 VALUES ($1,$2,$3,$4,$5,$6)",
                &[
                    &wire.campaign_digest,
                    &next_revision,
                    &next_cursor_i64,
                    &(WIRE_VERSION as i16),
                    &payload,
                    &payload_digest,
                ],
            )
            .map_err(|_| CheckpointConflict::Storage)?;
        let updated = transaction
            .execute(
                "UPDATE pulso_replay_campaign_checkpoint_heads \
                 SET current_revision=$3, cursor=$4, payload_digest=$5, updated_at=now() \
                 WHERE campaign_digest=$1 AND current_revision=$2 AND cursor=$6",
                &[
                    &wire.campaign_digest,
                    &expected_revision_i64,
                    &next_revision,
                    &next_cursor_i64,
                    &payload_digest,
                    &expected_cursor_i64,
                ],
            )
            .map_err(|_| CheckpointConflict::Storage)?;
        if updated != 1 {
            return Err(CheckpointConflict::CompareAndSwap);
        }
        transaction
            .commit()
            .map_err(|_| CheckpointConflict::Storage)?;
        Ok(next_revision as u64)
    }

    /// Load and validate the current immutable revision for an exact campaign.
    pub fn load(
        client: &mut postgres::Client,
        plan: ReplayPlan,
        source_snapshot_digest: &str,
        run_config_digest: &str,
        initial_revision: u64,
    ) -> Result<Option<ReplayCampaign>, CheckpointConflict> {
        let expected = ReplayCampaign::new(
            plan,
            source_snapshot_digest.to_owned(),
            run_config_digest.to_owned(),
            initial_revision,
        )?;
        let row = client
            .query_opt(
                "SELECT r.checkpoint_payload, r.revision, r.cursor, r.wire_version, \
                        r.payload_digest, h.current_revision, h.cursor, h.payload_digest \
                 FROM pulso_replay_campaign_checkpoint_heads h \
                 JOIN pulso_replay_campaign_checkpoint_revisions r \
                 ON r.campaign_digest=h.campaign_digest AND r.revision=h.current_revision \
                 WHERE h.campaign_digest=$1",
                &[&expected.campaign_digest],
            )
            .map_err(|_| CheckpointConflict::Storage)?;
        let Some(row) = row else { return Ok(None) };
        let payload: Vec<u8> = row.get(0);
        let revision: i64 = row.get(1);
        let cursor: i64 = row.get(2);
        let wire_version: i16 = row.get(3);
        let revision_digest: String = row.get(4);
        let head_revision: i64 = row.get(5);
        let head_cursor: i64 = row.get(6);
        let head_digest: String = row.get(7);
        let wire: CheckpointWire =
            serde_json::from_slice(&payload).map_err(|_| CheckpointConflict::InvalidCheckpoint)?;
        let actual_digest = digest_bytes(&payload);
        if revision != head_revision
            || cursor != head_cursor
            || revision_digest != head_digest
            || revision_digest != actual_digest
            || wire_version != wire.wire_version as i16
            || cursor < 0
            || cursor as usize != wire.next_cohort_index
        {
            return Err(CheckpointConflict::InvalidCheckpoint);
        }
        Self::decode(
            &payload,
            expected.plan,
            source_snapshot_digest,
            run_config_digest,
        )
        .map(Some)
    }
}

impl CheckpointWire {
    fn from_campaign(campaign: &ReplayCampaign) -> Result<Self, CheckpointConflict> {
        let checkpoint = campaign.checkpoint();
        let updates = checkpoint
            .protocol
            .updates
            .iter()
            .map(|(id, update)| {
                let cohort_index = update
                    .cohort_id
                    .strip_prefix("cohort-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .ok_or(CheckpointConflict::InvalidCheckpoint)?;
                Ok(UpdateWire {
                    cohort_index,
                    update_id_digest: update_id_digest(id)?,
                    next_revision: update.next_revision,
                    activated_at: update.activated_at,
                })
            })
            .collect::<Result<Vec<_>, CheckpointConflict>>()?;
        let seals = checkpoint
            .sealed_receipts
            .iter()
            .map(|(cohort_index, (work_digest, receipt_digest))| SealWire {
                cohort_index: *cohort_index,
                work_digest: work_digest.clone(),
                receipt_digest: receipt_digest.clone(),
            })
            .collect::<Vec<_>>();
        let mut updates = updates;
        updates.sort_by_key(|update| update.cohort_index);
        Ok(Self {
            wire_version: WIRE_VERSION,
            campaign_digest: checkpoint.campaign_digest,
            schedule_digest: checkpoint.schedule_digest,
            source_snapshot_digest: checkpoint.source_snapshot_digest,
            run_config_digest: checkpoint.run_config_digest,
            protocol: match campaign.plan.protocol() {
                ReplayProtocol::Frozen => "frozen".to_owned(),
                ReplayProtocol::Prequential => "prequential".to_owned(),
            },
            initial_revision: checkpoint.initial_revision,
            next_cohort_index: checkpoint.next_cohort_index,
            active_revision: checkpoint.protocol.active_revision,
            pending_work_digest: checkpoint
                .pending
                .as_ref()
                .map(|(work, _)| work_digest(work)),
            seals,
            updates,
        })
    }
}

fn update_id_digest(id: &str) -> Result<String, CheckpointConflict> {
    if let Some(hex_digest) = id.strip_prefix("raw-update-v1:") {
        if hex_digest.len() == 64
            && hex_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Ok(format!("sha256:{hex_digest}"));
        }
    }
    Err(CheckpointConflict::InvalidCheckpoint)
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn same_cursor_seal_conflicts(
    transaction: &mut postgres::Transaction<'_>,
    campaign_digest: &str,
    revision: i64,
    incoming: &[u8],
) -> Result<bool, CheckpointConflict> {
    let row = transaction
        .query_opt(
            "SELECT checkpoint_payload FROM pulso_replay_campaign_checkpoint_revisions \
             WHERE campaign_digest=$1 AND revision=$2",
            &[&campaign_digest, &revision],
        )
        .map_err(|_| CheckpointConflict::Storage)?;
    let Some(row) = row else { return Ok(false) };
    let existing_bytes: Vec<u8> = row.get(0);
    let existing: CheckpointWire = serde_json::from_slice(&existing_bytes)
        .map_err(|_| CheckpointConflict::InvalidCheckpoint)?;
    let candidate: CheckpointWire =
        serde_json::from_slice(incoming).map_err(|_| CheckpointConflict::InvalidCheckpoint)?;
    Ok(existing.next_cohort_index == candidate.next_cohort_index
        && existing
            .seals
            .iter()
            .zip(&candidate.seals)
            .any(|(left, right)| {
                left.cohort_index == right.cohort_index
                    && left.work_digest == right.work_digest
                    && left.receipt_digest != right.receipt_digest
            }))
}

fn validate_journal_extension(
    transaction: &mut postgres::Transaction<'_>,
    campaign_digest: &str,
    revision: i64,
    incoming: &CheckpointWire,
) -> Result<(), CheckpointConflict> {
    let row = transaction
        .query_one(
            "SELECT checkpoint_payload FROM pulso_replay_campaign_checkpoint_revisions \
             WHERE campaign_digest=$1 AND revision=$2",
            &[&campaign_digest, &revision],
        )
        .map_err(|_| CheckpointConflict::Storage)?;
    let payload: Vec<u8> = row.get(0);
    let existing: CheckpointWire =
        serde_json::from_slice(&payload).map_err(|_| CheckpointConflict::InvalidCheckpoint)?;
    if incoming.seals.len() < existing.seals.len()
        || incoming.seals[..existing.seals.len()] != existing.seals
    {
        if existing
            .seals
            .iter()
            .zip(&incoming.seals)
            .any(|(old, new)| {
                old.cohort_index == new.cohort_index
                    && old.work_digest == new.work_digest
                    && old.receipt_digest != new.receipt_digest
            })
        {
            return Err(CheckpointConflict::ConflictingCommit);
        }
        return Err(CheckpointConflict::InvalidCheckpoint);
    }
    for old in &existing.updates {
        let Some(new) = incoming
            .updates
            .iter()
            .find(|update| update.cohort_index == old.cohort_index)
        else {
            return Err(CheckpointConflict::InvalidCheckpoint);
        };
        if old.update_id_digest != new.update_id_digest
            || old.next_revision != new.next_revision
            || (old.activated_at.is_some() && old.activated_at != new.activated_at)
        {
            return Err(CheckpointConflict::InvalidCheckpoint);
        }
    }
    if incoming.updates.len() < existing.updates.len()
        || incoming.updates.iter().any(|update| {
            !existing
                .updates
                .iter()
                .any(|old| old.cohort_index == update.cohort_index)
                && update.cohort_index >= incoming.next_cohort_index
        })
    {
        return Err(CheckpointConflict::InvalidCheckpoint);
    }
    Ok(())
}
