//! Mutation testing of the PGC conformance suite: every deliberately broken repository must fail it.
use pg::conformance::run_suite;
use pg::repo::{Fault, MemRepo};
use std::sync::Arc;

const MUTANTS: &[(Fault, &str)] = &[
    (Fault::IgnoreFence, "stale-fence commit"),
    (Fault::IgnoreExpiry, "lost lease (expired holder commits)"),
    (Fault::IgnoreWorker, "wrong worker commits"),
    (Fault::ClaimIgnoresEffect, "effect-state skip"),
    (Fault::EffectNotRecorded, "begin_effect not recorded"),
    (Fault::AttemptNotBumped, "attempt not counted"),
    (Fault::AttemptBumpedTwice, "attempt over-counted"),
    (Fault::FenceNotBumped, "fence not bumped on reclaim"),
    (Fault::ReclaimOffByOne, "reclaim off by one"),
    (Fault::ReclaimLiveLease, "live lease reclaimed"),
    (Fault::ClaimNewestFirst, "not oldest first"),
    (Fault::ClaimIgnoresTenant, "tenant leak on claim"),
    (Fault::ForeignTenantOps, "tenant leak on holder ops"),
    (Fault::OverwriteOutput, "out/N overwritten"),
    (Fault::TouchNoExtend, "lease renewal is a no-op"),
];

#[test]
fn every_mutant_fails_the_suite() {
    assert!(run_suite(&|| Arc::new(MemRepo::with_fault(Fault::None))).is_empty());
    let survivors: Vec<_> = MUTANTS
        .iter()
        .filter(|(f, _)| run_suite(&|| Arc::new(MemRepo::with_fault(*f))).is_empty())
        .map(|(f, what)| format!("{f:?} ({what})"))
        .collect();
    assert!(survivors.is_empty(), "suite does not catch: {survivors:#?}");
}
