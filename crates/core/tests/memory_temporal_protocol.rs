//! U23-P exposes an explicit temporal protocol; it does not expose memory bytes.

use improvement_engine_core::memory_store::MemoryScope;
use improvement_engine_core::memory_temporal_protocol::{
    MemoryTemporalProtocol, ReplayMemoryClock, TemporalUseTiming,
};

fn scope(protocol: &str) -> MemoryScope {
    MemoryScope::new(
        "tenant-a",
        "investigation",
        "world-a",
        "campaign-a",
        protocol,
        "train",
    )
}

#[test]
fn frozen_protocol_accepts_only_training_memory_available_at_the_replay_cutoff() {
    let replay = ReplayMemoryClock::new(100);

    assert!(
        MemoryTemporalProtocol::Frozen
            .validate(&scope("frozen"), TemporalUseTiming::training(100), &replay)
            .is_ok()
    );
    assert!(
        MemoryTemporalProtocol::Frozen
            .validate(&scope("frozen"), TemporalUseTiming::training(101), &replay)
            .is_err()
    );
}

#[test]
fn continuous_protocol_requires_an_outcome_that_was_observable_before_the_memory_use() {
    let replay = ReplayMemoryClock::new(100);

    assert!(
        MemoryTemporalProtocol::Continuous
            .validate(
                &scope("continuous"),
                TemporalUseTiming::after_observed_outcome(100, 99),
                &replay,
            )
            .is_ok()
    );
    assert!(
        MemoryTemporalProtocol::Continuous
            .validate(
                &scope("continuous"),
                TemporalUseTiming::training(100),
                &replay
            )
            .is_err()
    );
    assert!(
        MemoryTemporalProtocol::Continuous
            .validate(
                &scope("continuous"),
                TemporalUseTiming::after_observed_outcome(99, 100),
                &replay,
            )
            .is_err()
    );
}

#[test]
fn frozen_protocol_rejects_outcome_feedback_so_training_memory_cannot_learn_mid_replay() {
    let replay = ReplayMemoryClock::new(100);

    assert!(
        MemoryTemporalProtocol::Frozen
            .validate(
                &scope("frozen"),
                TemporalUseTiming::after_observed_outcome(100, 99),
                &replay,
            )
            .is_err()
    );
}
