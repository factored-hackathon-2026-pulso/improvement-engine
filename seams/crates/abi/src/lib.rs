//! JobHandler ABI. Types mirror contracts/engine-steps C-7 (claim-next v1.1): fence token,
//! attempt count and effect states. Payloads are single-line JSON text (no newlines).

/// Stable identity of a handler (one per pipeline step).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HandlerId(pub String);

/// Effect state of a job (C-7: only `NoEffect` jobs are claimable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectState {
    NoEffect,
    DispatchBegun,
    UnknownPendingReconciliation,
    AppliedAcknowledged,
    CompletedNoEffect,
    CancelledBeforeEffect,
}

/// Lease/fence of the claiming worker (C-7 JobLease + attempt_count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    pub worker_id: String,
    pub fence_token: u64,
    pub attempt: u64,
}

/// Input handed to a handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEnvelope {
    pub job_id: String,
    pub step_index: usize,
    pub payload: String,
}

/// Result of a handler: next payload, events to append to the job log, effect state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputEnvelope {
    pub payload: String,
    pub events: Vec<String>,
    pub effect: EffectState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerError {
    Invalid(String),
    Failed(String),
    /// The fence no longer matches the store (C-7 `StaleFence`).
    StaleFence,
}

pub trait JobHandler {
    fn id(&self) -> HandlerId;
    fn run(&self, fence: &Fence, input: &InputEnvelope) -> Result<OutputEnvelope, HandlerError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effect_states_are_distinct() {
        assert_ne!(EffectState::NoEffect, EffectState::UnknownPendingReconciliation);
    }
}
