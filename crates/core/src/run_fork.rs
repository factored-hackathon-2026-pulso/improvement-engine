//! Immutable, operator-triggered replay forks.
//!
//! A fork is not a retry and cannot edit an historical run. It copies the
//! parent's already-authorized snapshot, configuration, memory reference and
//! cutoff into a new run, recording `replay_of`. Production persistence must
//! make the idempotency lookup and insert one transaction.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImmutableRunBinding {
    tenant_id: String,
    run_id: String,
    snapshot_ref: String,
    config_ref: String,
    memory_ref: String,
    cutoff_unix_seconds: u64,
    replay_of: Option<String>,
}

impl ImmutableRunBinding {
    pub fn new(
        tenant_id: impl Into<String>,
        run_id: impl Into<String>,
        snapshot_ref: impl Into<String>,
        config_ref: impl Into<String>,
        memory_ref: impl Into<String>,
        cutoff_unix_seconds: u64,
    ) -> Result<Self, RunForkError> {
        let binding = Self {
            tenant_id: tenant_id.into(),
            run_id: run_id.into(),
            snapshot_ref: snapshot_ref.into(),
            config_ref: config_ref.into(),
            memory_ref: memory_ref.into(),
            cutoff_unix_seconds,
            replay_of: None,
        };
        validate_identifier(&binding.tenant_id, "tenant_id")?;
        validate_identifier(&binding.run_id, "run_id")?;
        validate_identifier(&binding.snapshot_ref, "snapshot_ref")?;
        validate_identifier(&binding.config_ref, "config_ref")?;
        validate_identifier(&binding.memory_ref, "memory_ref")?;
        if cutoff_unix_seconds == 0 {
            return Err(RunForkError::InvalidCutoff);
        }
        Ok(binding)
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn snapshot_ref(&self) -> &str {
        &self.snapshot_ref
    }
    pub fn config_ref(&self) -> &str {
        &self.config_ref
    }
    pub fn memory_ref(&self) -> &str {
        &self.memory_ref
    }
    pub fn cutoff_unix_seconds(&self) -> u64 {
        self.cutoff_unix_seconds
    }
    pub fn replay_of(&self) -> Option<&str> {
        self.replay_of.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkRequest {
    tenant_id: String,
    replay_of: String,
    idempotency_key: String,
    reason: String,
}

impl ForkRequest {
    pub fn new(
        tenant_id: impl Into<String>,
        replay_of: impl Into<String>,
        idempotency_key: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, RunForkError> {
        let request = Self {
            tenant_id: tenant_id.into(),
            replay_of: replay_of.into(),
            idempotency_key: idempotency_key.into(),
            reason: reason.into(),
        };
        validate_identifier(&request.tenant_id, "tenant_id")?;
        validate_identifier(&request.replay_of, "replay_of")?;
        validate_identifier(&request.idempotency_key, "idempotency_key")?;
        validate_identifier(&request.reason, "reason")?;
        Ok(request)
    }
}

#[derive(Debug, Default)]
pub struct RunForkStore {
    runs: BTreeMap<(String, String), ImmutableRunBinding>,
    unavailable_refs: BTreeSet<String>,
    idempotency: BTreeMap<(String, String), (String, ImmutableRunBinding)>,
}

impl RunForkStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a completed or inspectable historical run. Registration is
    /// deliberately separate from fork creation: a debug endpoint may only
    /// fork a record that the durable run repository has already attested.
    pub fn register_original(&mut self, binding: ImmutableRunBinding) -> Result<(), RunForkError> {
        let key = (binding.tenant_id.clone(), binding.run_id.clone());
        if self.runs.contains_key(&key) {
            return Err(RunForkError::RunAlreadyRegistered);
        }
        self.runs.insert(key, binding);
        Ok(())
    }

    pub fn revoke_reference(&mut self, reference: impl Into<String>) -> Result<(), RunForkError> {
        let reference = reference.into();
        validate_identifier(&reference, "reference")?;
        self.unavailable_refs.insert(reference);
        Ok(())
    }

    pub fn fork(&mut self, request: ForkRequest) -> Result<ImmutableRunBinding, RunForkError> {
        let fingerprint = fingerprint(&request);
        let key = (request.tenant_id.clone(), request.idempotency_key.clone());
        if let Some((existing_fingerprint, run)) = self.idempotency.get(&key) {
            return if existing_fingerprint == &fingerprint {
                Ok(run.clone())
            } else {
                Err(RunForkError::IdempotencyConflict)
            };
        }
        // Do not reveal whether another tenant owns an otherwise valid id.
        let parent = self
            .runs
            .get(&(request.tenant_id.clone(), request.replay_of.clone()))
            .cloned()
            .ok_or(RunForkError::ParentNotFound)?;
        for reference in [&parent.snapshot_ref, &parent.config_ref, &parent.memory_ref] {
            if self.unavailable_refs.contains(reference) {
                return Err(RunForkError::ReferenceUnavailable {
                    reference: reference.clone(),
                });
            }
        }
        let child_id = format!("fork_{}", short_digest(&fingerprint));
        let child_key = (parent.tenant_id.clone(), child_id.clone());
        if self.runs.contains_key(&child_key) {
            return Err(RunForkError::ForkCollision);
        }
        let child = ImmutableRunBinding {
            tenant_id: parent.tenant_id.clone(),
            run_id: child_id,
            snapshot_ref: parent.snapshot_ref,
            config_ref: parent.config_ref,
            memory_ref: parent.memory_ref,
            cutoff_unix_seconds: parent.cutoff_unix_seconds,
            replay_of: Some(request.replay_of),
        };
        self.runs.insert(child_key, child.clone());
        self.idempotency.insert(key, (fingerprint, child.clone()));
        Ok(child)
    }

    pub fn run(&self, tenant_id: &str, run_id: &str) -> Result<&ImmutableRunBinding, RunForkError> {
        self.runs
            .get(&(tenant_id.to_owned(), run_id.to_owned()))
            .ok_or(RunForkError::ParentNotFound)
    }

    pub fn run_count(&self, tenant_id: &str) -> usize {
        self.runs
            .keys()
            .filter(|(tenant, _)| tenant == tenant_id)
            .count()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RunForkError {
    InvalidIdentifier { field: &'static str },
    InvalidCutoff,
    ParentNotFound,
    RunAlreadyRegistered,
    ReferenceUnavailable { reference: String },
    IdempotencyConflict,
    ForkCollision,
}

impl fmt::Display for RunForkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field } => write!(f, "invalid {field}"),
            Self::InvalidCutoff => f.write_str("cutoff must be nonzero"),
            Self::ParentNotFound => f.write_str("parent run not found"),
            Self::RunAlreadyRegistered => f.write_str("run is already registered"),
            Self::ReferenceUnavailable { reference } => {
                write!(f, "reference unavailable: {reference}")
            }
            Self::IdempotencyConflict => {
                f.write_str("idempotency key was reused with different input")
            }
            Self::ForkCollision => f.write_str("fork id collision"),
        }
    }
}

impl std::error::Error for RunForkError {}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), RunForkError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(RunForkError::InvalidIdentifier { field });
    }
    Ok(())
}

fn fingerprint(request: &ForkRequest) -> String {
    let mut hasher = Sha256::new();
    for value in [
        &request.tenant_id,
        &request.replay_of,
        &request.idempotency_key,
        &request.reason,
    ] {
        hasher.update(value.len().to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn short_digest(value: &str) -> String {
    value[7..23].to_owned()
}
