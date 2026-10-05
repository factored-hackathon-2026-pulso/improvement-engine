//! B2 registry writer: the engine side of "we propose, agent-core manages".
//!
//! A compiled proposal of the `reasoning` crate (anchored patch or new-agent closure) is delivered to the REAL agent-core as the
//! `builder` principal, in one of two ways (`Via`):
//! - `RegistryApi`: `POST /v1/registry/proposals`, `PUT .../draft`, `POST .../validate`, then a read-back. The changes are the
//!   engine's byte-exact compiled ones. The registry API of the shared Core must accept the engine key as a builder; on the local
//!   stack that needs the engine kid in the registry's staff keys (see `docs/dev/LOCAL_STACK.md`).
//! - `BuilderRun`: `POST /v1/runs` of the `pulso-builder` agent (an agent-core model run that authors and writes the draft with its
//!   in-process registry tools). The draft is then the agent's, not the engine's: the outcome says so.
//!
//! The engine NEVER approves, publishes, promotes, revokes or rejects: `guard::allowed` is a closed allow-list that every request
//! passes before it reaches the transport. W11 adds `freeze` and `evaluate` (module `eval`) so the engine can PROVE a proposal on
//! throwaway manual-origin evaluation drafts before it announces it (module `proof`). Success is `proposal_id` + `valid` + a non-empty change list read back from the
//! registry. Refusals map to a closed set of reasons (`Reason`). Retries are idempotent per finding key (`Submission::key`): a local
//! receipt store remembers the proposal of a key, because the registry at main has no list route and no `Idempotency-Key` on HTTP.
//! No token is ever printed, stored in a receipt or put in an error.
pub mod baseline;
pub mod closure;
pub mod eval;
pub mod guard;
pub mod proof;
pub mod store;
pub mod transport;
pub mod writer;

use reasoning::finding::Finding;
use reasoning::patch::Compiled;
use serde_json::{Value, json};

pub use store::{FileStore, MemoryStore, Receipt, ReceiptStore};
pub use transport::{HttpTransport, Reply, Request, Transport, TransportError};
pub use writer::{Config, Environment, Via, Writer};

/// Registry quota of proposals opened with origin `auto_detect` per rolling 24 hours (agent-core `DEFAULT_QUOTAS`).
pub const PROPOSALS_PER_DAY: usize = 10;
pub const TITLE_PREFIX: &str = "[improvement-engine]";

/// Closed set of reasons a delivery does not succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// `no_change` proposal or an empty change list: nothing to write.
    NothingToPropose,
    /// The submission is malformed before any byte is sent (blank agent, change without kind/content/id/version, ...).
    InvalidSubmission,
    /// The live text the patch was compiled against is no longer the live registry's current version.
    BaseChanged,
    /// The live registry has no such artifact to patch.
    BaseMissing,
    Unauthorized,
    ForbiddenRole,
    StepUpRequired,
    /// 10 proposals per 24 h (registry answer, or the local guard that predicts it).
    QuotaExceeded,
    /// 422: the registry refused the draft (limits, schema, platform guardrails).
    ValidationFailed,
    /// The draft was stored but the registry's `validate` reports violations.
    DraftInvalid,
    /// The stored draft has no changes (or a different number than sent).
    EmptyDraft,
    ProposalStale,
    /// Nothing reached the registry (connect failed).
    RegistryUnreachable,
    /// The request may have reached the registry: the outcome is unknown, check by hand before retrying.
    OutcomeUnknown,
    RegistryError,
    /// `BuilderRun`: the run did not end `completed` (an agent-core escalation, e.g. a tool denial or quota).
    RunNotCompleted,
    RunMalformed,
    /// A proposal id exists but the registry cannot be read back with this credential, so success cannot be claimed.
    ReadbackUnavailable,
    /// The request is not on the engine's allow-list (approve, publish, promote, ...): never sent.
    ForbiddenOperation,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::NothingToPropose => "nothing_to_propose",
            Reason::InvalidSubmission => "invalid_submission",
            Reason::BaseChanged => "base_changed",
            Reason::BaseMissing => "base_missing",
            Reason::Unauthorized => "unauthorized",
            Reason::ForbiddenRole => "forbidden_role",
            Reason::StepUpRequired => "step_up_required",
            Reason::QuotaExceeded => "quota_exceeded",
            Reason::ValidationFailed => "validation_failed",
            Reason::DraftInvalid => "draft_invalid",
            Reason::EmptyDraft => "empty_draft",
            Reason::ProposalStale => "proposal_stale",
            Reason::RegistryUnreachable => "registry_unreachable",
            Reason::OutcomeUnknown => "outcome_unknown",
            Reason::RegistryError => "registry_error",
            Reason::RunNotCompleted => "run_not_completed",
            Reason::RunMalformed => "run_malformed",
            Reason::ReadbackUnavailable => "readback_unavailable",
            Reason::ForbiddenOperation => "forbidden_operation",
        }
    }
}

/// What the engine hands over: one compiled proposal of one finding.
#[derive(Debug, Clone)]
pub struct Submission {
    pub finding_id: String,
    /// Stable across runs (`Finding::evidence_ref`): the idempotency key derives from it, not from the positional finding id.
    pub evidence_ref: String,
    pub agent_id: String,
    pub target_ref: String,
    pub kind: String,
    pub base_digest: String,
    pub changes: Vec<Value>,
    pub rationale: String,
}

impl Submission {
    pub fn new(f: &Finding, c: &Compiled) -> Submission {
        Submission {
            finding_id: f.id.clone(),
            evidence_ref: f.evidence_ref(),
            agent_id: c.agent_id.clone(),
            target_ref: c.target_ref.clone(),
            kind: c.kind.clone(),
            base_digest: c.base_digest.clone(),
            changes: c.changes.clone(),
            rationale: c.rationale.clone(),
        }
    }

    /// Idempotency key of the finding: same evidence and same target, same key. Opaque, `pulso-` + 24 hex.
    pub fn key(&self) -> String {
        let raw = format!("b2|{}|{}|{}", self.evidence_ref, self.target_ref, self.kind);
        format!("pulso-{}", steps::compile::sha256_hex_calm(raw.as_bytes(), 24))
    }

    /// Deterministic proposal title (`<= 120` chars, the platform announce limit; the target is cut at a word boundary, the key suffix stays).
    pub fn title(&self) -> String {
        let suffix = &self.key()[6..14];
        let fixed = TITLE_PREFIX.chars().count() + suffix.chars().count() + 2;
        let target = reasoning::dossier::cut_title(&self.target_ref, 120usize.saturating_sub(fixed));
        format!("{TITLE_PREFIX} {target} {suffix}")
    }
}

/// Honest label of where the proposal went and who authored the draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labels {
    pub environment: &'static str,
    pub via: &'static str,
    pub authored_by: &'static str,
    /// Whose credential wrote to the registry: the engine's `builder` principal, or a stand-in the stack needed (stated, never hidden).
    pub credential: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// `delivered | denied`
    pub status: &'static str,
    pub reason: Option<Reason>,
    pub detail: String,
    pub proposal_id: Option<String>,
    pub valid: Option<bool>,
    pub changes: usize,
    pub state: Option<String>,
    /// The key found a receipt: no new proposal was created.
    pub replayed: bool,
    pub key: String,
    pub labels: Labels,
}

impl Outcome {
    pub fn delivered(&self) -> bool {
        self.status == "delivered"
    }

    pub fn to_json(&self) -> Value {
        json!({"contract": "registry-writer/b2-0", "status": self.status, "reason": self.reason.map(Reason::code), "detail": self.detail,
               "proposal_id": self.proposal_id, "valid": self.valid, "changes": self.changes, "state": self.state, "replayed": self.replayed,
               "idempotency_key": self.key,
               "labels": {"environment": self.labels.environment, "via": self.labels.via, "authored_by": self.labels.authored_by, "credential": self.labels.credential},
               "engine_never_approves_publishes_or_promotes": true, "management": "agent-core (evaluation, approval, publish)"})
    }
}
