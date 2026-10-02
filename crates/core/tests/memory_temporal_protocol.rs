//! The U23 temporal boundary deliberately exports no clock, timing or evidence
//! constructors. Only the trusted U04-B/future runner composition can emit it.

/// ```compile_fail
/// use improvement_engine_core::memory_temporal_protocol::TemporalMemoryEvidence;
/// let _ = TemporalMemoryEvidence { commitment: "forged".into() };
/// ```
#[allow(dead_code)]
fn temporal_evidence_is_not_caller_constructible() {}

/// ```compile_fail
/// use improvement_engine_core::memory_temporal_protocol::ReplayMemoryClock;
/// let _ = ReplayMemoryClock::new(99);
/// ```
#[allow(dead_code)]
fn public_clock_is_not_an_authority_bypass() {}
