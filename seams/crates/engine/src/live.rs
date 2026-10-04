//! Live wiring points to core-client (TODO hooks). Nothing here talks to Core: the live run on the real image
//! is the next package (K3 + INT). Stubs report honestly instead of faking a result.
//!
//! TODO(K3) dry-run: build `adapters::DryRun` from `core_client::ops` `dry_run(job_id, &DryRunRequest)`; the digest
//!   of the dry-run result replaces the local canonical digest in the compile handler.
//! TODO(K3) arms: replace `Arms` by a handler that builds `core_client::arms::ArmRequest` (base and candidate),
//!   calls `run_arm` (route `/evaluation/arms/run`, idempotency key required: derive it from job_id + step + the
//!   executor fence/attempt) and stores the two `ArmReport`s as the gate's `reports`.
//! TODO(INT) publish: core-client has no publish route today (routes::ALL has none), so `Publish` stays
//!   blocked(core). A live one MUST return `effectful() == true` so the executor records the dispatch before
//!   the call and never re-dispatches after an unacknowledged crash (C-7).
use abi::*;

struct Stub {
    id: &'static str,
    event: &'static str,
}

impl JobHandler for Stub {
    fn id(&self) -> HandlerId {
        HandlerId(self.id.into())
    }
    fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        // payload passes through unchanged: the stub produced nothing and says so
        Ok(OutputEnvelope { payload: i.payload.clone(), events: vec![self.event.into()], effect: EffectState::NoEffect })
    }
}

/// `arms` (not exercised) then `publish` (blocked on core), to append after the five step handlers.
pub fn stub_handlers() -> Vec<Box<dyn JobHandler>> {
    vec![
        Box::new(Stub { id: "arms", event: "thread:arms:not_exercised:needs=core-client(K3)" }),
        Box::new(Stub { id: "publish", event: "thread:publish:blocked(core):needs=core-client(K3)+INT" }),
    ]
}
