//! Consumer-side boundary for one deterministic, pinned Agent Core task.
//!
//! This module is deliberately not an Agent Core runtime, scheduler, HTTP
//! client, LLM/Jev integration or a replacement Agent OS. It expresses the
//! Pulso ownership boundary: pin and authorize one task input, issue it once,
//! and preserve a receipt whose `Unknown` state prevents a blind re-dispatch.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

/// The exact Agent Core snapshot which Pulso has reviewed for this cut.
pub const SUPPORTED_CONTRACT_VERSION: &str = "0.5.0";
pub const SUPPORTED_CONTRACT_SHA: &str = "53e729d624c8284e906249df84c1a1df84cc8d40";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTaskBinding {
    agent_id: String,
    release_id: String,
    contract_version: String,
    contract_sha: String,
    digest: String,
}

impl CoreTaskBinding {
    pub fn new(
        agent_id: impl Into<String>,
        release_id: impl Into<String>,
        contract_version: impl Into<String>,
        contract_sha: impl Into<String>,
    ) -> Result<Self, CoreTaskError> {
        let agent_id = agent_id.into();
        let release_id = release_id.into();
        let contract_version = contract_version.into();
        let contract_sha = contract_sha.into();
        validate_identifier(&agent_id, "agent_id")?;
        validate_identifier(&release_id, "release_id")?;
        if !is_contract_sha(&contract_sha) {
            return Err(CoreTaskError::UnpinnedContract);
        }
        if contract_version.is_empty() {
            return Err(CoreTaskError::InvalidBinding);
        }
        let digest = task_digest(&[
            ("agent_id", &agent_id),
            ("release_id", &release_id),
            ("contract_version", &contract_version),
            ("contract_sha", &contract_sha),
        ]);
        Ok(Self {
            agent_id,
            release_id,
            contract_version,
            contract_sha,
            digest,
        })
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
    pub fn release_id(&self) -> &str {
        &self.release_id
    }
    pub fn contract_version(&self) -> &str {
        &self.contract_version
    }
    pub fn contract_sha(&self) -> &str {
        &self.contract_sha
    }
}

/// Approved task bindings are configured by the Pulso control plane. A valid
/// SHA alone is not authority to execute an arbitrary Agent Core release.
#[derive(Debug, Clone)]
pub struct CoreTaskBindingRegistry {
    approved: BTreeMap<String, CoreTaskBinding>,
}

impl CoreTaskBindingRegistry {
    pub fn new(bindings: Vec<CoreTaskBinding>) -> Result<Self, CoreTaskError> {
        let mut approved = BTreeMap::new();
        for binding in bindings {
            if binding.contract_version != SUPPORTED_CONTRACT_VERSION
                || binding.contract_sha != SUPPORTED_CONTRACT_SHA
            {
                return Err(CoreTaskError::UnsupportedContractSnapshot);
            }
            if approved.insert(binding.digest.clone(), binding).is_some() {
                return Err(CoreTaskError::DuplicateApprovedBinding);
            }
        }
        if approved.is_empty() {
            return Err(CoreTaskError::EmptyApprovedBindings);
        }
        Ok(Self { approved })
    }

    fn contains(&self, binding: &CoreTaskBinding) -> bool {
        self.approved
            .get(binding.digest())
            .is_some_and(|approved| approved == binding)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTaskScope {
    tenant_id: String,
    job_id: String,
    grant_id: String,
    authority_ref: String,
}

impl CoreTaskScope {
    pub fn new(
        tenant_id: impl Into<String>,
        job_id: impl Into<String>,
        grant_id: impl Into<String>,
        authority_ref: impl Into<String>,
    ) -> Result<Self, CoreTaskError> {
        let tenant_id = tenant_id.into();
        let job_id = job_id.into();
        let grant_id = grant_id.into();
        let authority_ref = authority_ref.into();
        validate_identifier(&tenant_id, "tenant_id")?;
        validate_identifier(&job_id, "job_id")?;
        validate_identifier(&grant_id, "grant_id")?;
        validate_identifier(&authority_ref, "authority_ref")?;
        Ok(Self {
            tenant_id,
            job_id,
            grant_id,
            authority_ref,
        })
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }
    pub fn authority_ref(&self) -> &str {
        &self.authority_ref
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTaskInvocation {
    scope: CoreTaskScope,
    binding: CoreTaskBinding,
    attempt_id: String,
    input_digest: String,
}

impl CoreTaskInvocation {
    pub fn new(
        scope: CoreTaskScope,
        binding: CoreTaskBinding,
        attempt_id: impl Into<String>,
        input_digest: impl Into<String>,
    ) -> Result<Self, CoreTaskError> {
        let attempt_id = attempt_id.into();
        let input_digest = input_digest.into();
        validate_identifier(&attempt_id, "attempt_id")?;
        if !is_sha256_digest(&input_digest) {
            return Err(CoreTaskError::InvalidInputDigest);
        }
        Ok(Self {
            scope,
            binding,
            attempt_id,
            input_digest,
        })
    }

    pub fn scope(&self) -> &CoreTaskScope {
        &self.scope
    }
    pub fn binding(&self) -> &CoreTaskBinding {
        &self.binding
    }
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
    pub fn input_digest(&self) -> &str {
        &self.input_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreTaskOutcome {
    Succeeded,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTaskReceipt {
    binding_digest: String,
    outcome: CoreTaskOutcome,
    core_run_id: Option<String>,
    output_digest: Option<String>,
}

impl CoreTaskReceipt {
    pub fn binding_digest(&self) -> &str {
        &self.binding_digest
    }
    pub fn outcome(&self) -> &CoreTaskOutcome {
        &self.outcome
    }
    pub fn core_run_id(&self) -> Option<&str> {
        self.core_run_id.as_deref()
    }
    pub fn output_digest(&self) -> Option<&str> {
        self.output_digest.as_deref()
    }
}

/// The only runtime-facing port owned by this slice. A future transport
/// adapter may implement it, but it must retain the same attempt ledger and
/// conservative unknown-outcome semantics.
pub trait CoreTaskPort {
    fn invoke(&mut self, invocation: CoreTaskInvocation) -> Result<CoreTaskReceipt, CoreTaskError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatorDisposition {
    Succeed,
    TimeoutAfterDispatch,
    CrashAfterDispatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttemptRecord {
    scope: CoreTaskScope,
    binding: CoreTaskBinding,
    input_digest: String,
    receipt: CoreTaskReceipt,
}

/// Deterministic contract double. It models a single serialized dispatcher:
/// storing an attempt record before returning makes same-attempt retries and
/// concurrent callers observe one receipt rather than dispatching twice.
pub struct CoreTaskSimulator {
    approved_bindings: CoreTaskBindingRegistry,
    disposition: SimulatorDisposition,
    success: Option<(String, String)>,
    attempts: BTreeMap<(String, String, String), AttemptRecord>,
    dispatch_count: u64,
}

impl CoreTaskSimulator {
    pub fn new(approved_bindings: CoreTaskBindingRegistry) -> Self {
        Self {
            approved_bindings,
            disposition: SimulatorDisposition::Succeed,
            success: None,
            attempts: BTreeMap::new(),
            dispatch_count: 0,
        }
    }

    pub fn script(&mut self, disposition: SimulatorDisposition) {
        self.disposition = disposition;
        self.success = None;
    }

    pub fn script_success(
        &mut self,
        core_run_id: impl Into<String>,
        output_digest: impl Into<String>,
    ) -> Result<(), CoreTaskError> {
        let core_run_id = core_run_id.into();
        let output_digest = output_digest.into();
        validate_identifier(&core_run_id, "core_run_id")?;
        if !is_sha256_digest(&output_digest) {
            return Err(CoreTaskError::InvalidOutputDigest);
        }
        self.disposition = SimulatorDisposition::Succeed;
        self.success = Some((core_run_id, output_digest));
        Ok(())
    }

    pub fn dispatch_count(&self) -> u64 {
        self.dispatch_count
    }
}

impl CoreTaskPort for CoreTaskSimulator {
    fn invoke(&mut self, invocation: CoreTaskInvocation) -> Result<CoreTaskReceipt, CoreTaskError> {
        if !self.approved_bindings.contains(&invocation.binding) {
            return Err(CoreTaskError::UnsupportedBinding {
                binding_digest: invocation.binding.digest.clone(),
            });
        }
        let attempt_key = (
            invocation.scope.tenant_id.clone(),
            invocation.scope.job_id.clone(),
            invocation.attempt_id.clone(),
        );
        if let Some(existing) = self.attempts.get(&attempt_key) {
            if existing.scope != invocation.scope {
                return Err(CoreTaskError::ScopeMismatch {
                    attempt_id: invocation.attempt_id,
                });
            }
            if existing.binding != invocation.binding
                || existing.input_digest != invocation.input_digest
            {
                return Err(CoreTaskError::AttemptConflict {
                    attempt_id: invocation.attempt_id,
                });
            }
            return Ok(existing.receipt.clone());
        }

        self.dispatch_count = self
            .dispatch_count
            .checked_add(1)
            .ok_or(CoreTaskError::DispatchCountExhausted)?;
        let receipt = match self.disposition {
            SimulatorDisposition::Succeed => {
                let (core_run_id, output_digest) = self
                    .success
                    .clone()
                    .ok_or(CoreTaskError::MissingSuccessScript)?;
                CoreTaskReceipt {
                    binding_digest: invocation.binding.digest.clone(),
                    outcome: CoreTaskOutcome::Succeeded,
                    core_run_id: Some(core_run_id),
                    output_digest: Some(output_digest),
                }
            }
            SimulatorDisposition::TimeoutAfterDispatch
            | SimulatorDisposition::CrashAfterDispatch => CoreTaskReceipt {
                binding_digest: invocation.binding.digest.clone(),
                outcome: CoreTaskOutcome::Unknown,
                core_run_id: None,
                output_digest: None,
            },
        };
        self.attempts.insert(
            attempt_key,
            AttemptRecord {
                scope: invocation.scope,
                binding: invocation.binding,
                input_digest: invocation.input_digest,
                receipt: receipt.clone(),
            },
        );
        Ok(receipt)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreTaskError {
    InvalidIdentifier { field: &'static str },
    InvalidBinding,
    UnpinnedContract,
    UnsupportedBinding { binding_digest: String },
    UnsupportedContractSnapshot,
    DuplicateApprovedBinding,
    EmptyApprovedBindings,
    InvalidInputDigest,
    InvalidOutputDigest,
    MissingSuccessScript,
    ScopeMismatch { attempt_id: String },
    AttemptConflict { attempt_id: String },
    DispatchCountExhausted,
}

impl fmt::Display for CoreTaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field } => {
                write!(f, "{field} must be a lowercase identifier")
            }
            Self::InvalidBinding => f.write_str("binding version must not be empty"),
            Self::UnpinnedContract => {
                f.write_str("Agent Core contract SHA must be a 40-character immutable SHA")
            }
            Self::UnsupportedBinding { .. } => {
                f.write_str("task binding is not the configured Agent Core snapshot")
            }
            Self::UnsupportedContractSnapshot => {
                f.write_str("approved binding must use the supported Agent Core contract snapshot")
            }
            Self::DuplicateApprovedBinding => {
                f.write_str("approved task binding digest is duplicated")
            }
            Self::EmptyApprovedBindings => {
                f.write_str("at least one task binding must be approved")
            }
            Self::InvalidInputDigest => f.write_str("task input digest must be sha256"),
            Self::InvalidOutputDigest => f.write_str("task output digest must be sha256"),
            Self::MissingSuccessScript => {
                f.write_str("successful simulator dispatch requires a scripted receipt")
            }
            Self::ScopeMismatch { .. } => {
                f.write_str("attempt identity belongs to a different tenant/grant/job scope")
            }
            Self::AttemptConflict { .. } => {
                f.write_str("attempt identity has a different binding or input")
            }
            Self::DispatchCountExhausted => f.write_str("dispatch count exhausted"),
        }
    }
}
impl std::error::Error for CoreTaskError {}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), CoreTaskError> {
    let mut chars = value.chars();
    if !matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(CoreTaskError::InvalidIdentifier { field });
    }
    Ok(())
}

fn is_contract_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn task_digest(parts: &[(&str, &str)]) -> String {
    let mut canonical = String::new();
    for (name, value) in parts {
        canonical.push_str(name);
        canonical.push(':');
        canonical.push_str(&value.len().to_string());
        canonical.push(':');
        canonical.push_str(value);
        canonical.push('\n');
    }
    format!(
        "core-task:sha256:{:x}",
        Sha256::digest(canonical.as_bytes())
    )
}
