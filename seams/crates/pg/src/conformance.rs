//! Reusable conformance suite for a `JobRepository`, written against the frozen C-7 v1.1 claim-next
//! signature and traces (contracts/engine-steps/pack/parts/c7_claim_next) and the semantics of Codex's
//! durable_jobs.rs. `run_suite` returns one line per failing scenario (empty = conforms).
use crate::repo::{Claimed, JobRepository, RepoError};
use std::sync::{Arc, Barrier};

pub type Make<'a> = &'a (dyn Fn() -> Arc<dyn JobRepository> + Sync);
type Scenario = fn(Make) -> Result<(), String>;

const A: &str = "tenant-a";

pub const SCENARIOS: &[(&str, Scenario)] = &[
    ("claim_oldest_first", claim_oldest_first),
    ("expired_lease_reclaimed_with_higher_fence", expired_lease_reclaimed_with_higher_fence),
    ("reclaim_chain_bumps_fence_and_attempt_by_one", reclaim_chain),
    ("tenant_isolation", tenant_isolation),
    ("unknown_effect_not_claimable", unknown_effect_not_claimable),
    ("invalid_lease_duration_and_ids", invalid_inputs),
    ("single_winner_under_threads", single_winner_under_threads),
    ("superseded_worker_cannot_commit_even_after_renewal", superseded_cannot_commit),
    ("expired_unclaimed_lease_cannot_commit", expired_cannot_commit),
    ("one_output_per_step", one_output_per_step),
    ("crash_before_commit_then_reclaim_is_attempt_plus_one", crash_then_reclaim),
    ("touch_lease_extends_only_for_current_holder", touch_lease),
];

pub fn run_suite(make: Make) -> Vec<String> {
    let mut failures = vec![];
    for (name, f) in SCENARIOS {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(make)));
        match r {
            Ok(Ok(())) => {}
            Ok(Err(e)) => failures.push(format!("{name}: {e}")),
            Err(_) => failures.push(format!("{name}: panicked")),
        }
    }
    failures
}

fn eq<T: PartialEq + std::fmt::Debug>(what: &str, got: T, want: T) -> Result<(), String> {
    if got == want { Ok(()) } else { Err(format!("{what}: got {got:?}, want {want:?}")) }
}
fn st<T>(r: Result<T, RepoError>) -> Result<T, String> {
    r.map_err(|e| format!("unexpected {e:?}"))
}
fn claim(r: &dyn JobRepository, t: &str, w: &str, now: u64) -> Result<Option<Claimed>, String> {
    st(r.claim_next(t, w, now, 30))
}

fn claim_oldest_first(make: Make) -> Result<(), String> {
    let r = make();
    let (a, b) = (st(r.admit(A))?, st(r.admit(A))?);
    eq("w1", claim(&*r, A, "w1", 100)?, Some(Claimed { job: a, fence_token: 1, attempt: 1, expires_at: 130 }))?;
    eq("w2", claim(&*r, A, "w2", 101)?, Some(Claimed { job: b, fence_token: 1, attempt: 1, expires_at: 131 }))?;
    eq("w3 none", claim(&*r, A, "w3", 102)?, None)
}

fn expired_lease_reclaimed_with_higher_fence(make: Make) -> Result<(), String> {
    let r = make();
    let a = st(r.admit(A))?;
    eq("w1", claim(&*r, A, "w1", 100)?.map(|c| c.fence_token), Some(1))?;
    eq("w2 before expiry", claim(&*r, A, "w2", 129)?, None)?;
    // reclaimable exactly at now == expires
    eq("w2 at expiry", claim(&*r, A, "w2", 130)?, Some(Claimed { job: a.clone(), fence_token: 2, attempt: 2, expires_at: 160 }))?;
    eq("w1 begin_effect", r.begin_effect(A, &a, "w1", 1, 131), Err(RepoError::StaleFence))
}

fn reclaim_chain(make: Make) -> Result<(), String> {
    let r = make();
    st(r.admit(A))?;
    for k in 0..4u64 {
        let c = claim(&*r, A, &format!("w{k}"), 100 + 30 * k)?.ok_or("no claim")?;
        eq("fence", c.fence_token, k + 1)?;
        eq("attempt", c.attempt, k + 1)?;
    }
    Ok(())
}

fn tenant_isolation(make: Make) -> Result<(), String> {
    let r = make();
    let a = st(r.admit("tenant-a"))?;
    eq("b sees nothing", claim(&*r, "tenant-b", "w1", 100)?, None)?;
    eq("a claims", claim(&*r, "tenant-a", "w1", 100)?.map(|c| c.job), Some(a.clone()))?;
    eq("b cannot touch a's job", r.begin_effect("tenant-b", &a, "w1", 1, 101), Err(RepoError::StaleFence))
}

fn unknown_effect_not_claimable(make: Make) -> Result<(), String> {
    let r = make();
    let a = st(r.admit(A))?;
    let c = claim(&*r, A, "w1", 100)?.ok_or("no claim")?;
    st(r.begin_effect(A, &a, "w1", c.fence_token, 101))?;
    eq("claim after expiry", claim(&*r, A, "w2", 201)?, None)
}

fn invalid_inputs(make: Make) -> Result<(), String> {
    let r = make();
    st(r.admit(A))?;
    eq("lease 0", r.claim_next(A, "w1", 100, 0), Err(RepoError::InvalidLeaseDuration))?;
    for bad in ["", "a|b", "a\nb"] {
        if !matches!(r.claim_next(A, bad, 100, 30), Err(RepoError::InvalidId(_))) {
            return Err(format!("worker {bad:?} accepted"));
        }
        if !matches!(r.claim_next(bad, "w1", 100, 30), Err(RepoError::InvalidId(_))) {
            return Err(format!("tenant {bad:?} accepted"));
        }
    }
    // rejected calls changed nothing
    eq("still claimable", claim(&*r, A, "w1", 100)?.map(|c| c.fence_token), Some(1))
}

fn single_winner_under_threads(make: Make) -> Result<(), String> {
    for round in 0..5 {
        let r = make();
        let jobs: Vec<String> = (0..3).map(|_| st(r.admit(A))).collect::<Result<_, _>>()?;
        let threads = 8usize;
        let gate = Arc::new(Barrier::new(threads));
        let handles: Vec<_> = (0..threads)
            .map(|i| {
                let (r, gate) = (r.clone(), gate.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    r.claim_next(A, &format!("w{i}"), 100, 30)
                })
            })
            .collect();
        let mut won: Vec<Claimed> = vec![];
        for h in handles {
            if let Some(c) = st(h.join().map_err(|_| "thread panicked".to_string())?)? {
                won.push(c);
            }
        }
        let mut ids: Vec<&String> = won.iter().map(|c| &c.job).collect();
        ids.sort();
        ids.dedup();
        eq(&format!("round {round}: winners"), won.len(), jobs.len())?;
        eq(&format!("round {round}: distinct jobs claimed (double claim)"), ids.len(), jobs.len())?;
        if won.iter().any(|c| c.fence_token != 1 || c.attempt != 1) {
            return Err(format!("round {round}: fence/attempt not 1: {won:?}"));
        }
    }
    Ok(())
}

fn superseded_cannot_commit(make: Make) -> Result<(), String> {
    let r = make();
    let a = st(r.admit(A))?;
    let c1 = claim(&*r, A, "w1", 100)?.ok_or("no claim")?;
    let c2 = claim(&*r, A, "w2", 130)?.ok_or("no reclaim")?;
    eq("fence", c2.fence_token, 2)?;
    // w2 renews, then the old worker (whose clock is skewed: it still believes now < 130) reaches the store
    eq("renew", r.touch_lease(A, &a, "w2", 2, 140, 30), Ok(170))?;
    for skewed_now in [120, 129, 135, 141] {
        eq("stale commit", r.commit_output(A, &a, 0, "w1", c1.fence_token, skewed_now, "P stale"), Err(RepoError::StaleFence))?;
    }
    eq("out/0 untouched", st(r.output(A, &a, 0))?, None)?;
    st(r.commit_output(A, &a, 0, "w2", 2, 141, "P new"))?;
    eq("out/0 is the new worker's", st(r.output(A, &a, 0))?, Some("P new".to_string()))
}

fn expired_cannot_commit(make: Make) -> Result<(), String> {
    let r = make();
    let a = st(r.admit(A))?;
    let c = claim(&*r, A, "w1", 100)?.ok_or("no claim")?;
    eq("commit at expiry", r.commit_output(A, &a, 0, "w1", c.fence_token, 130, "P x"), Err(RepoError::StaleFence))?;
    eq("wrong worker, right fence", r.commit_output(A, &a, 0, "w9", c.fence_token, 110, "P x"), Err(RepoError::StaleFence))?;
    eq("none", st(r.output(A, &a, 0))?, None)?;
    st(r.commit_output(A, &a, 0, "w1", c.fence_token, 129, "P ok"))
}

fn one_output_per_step(make: Make) -> Result<(), String> {
    let r = make();
    let a = st(r.admit(A))?;
    let c = claim(&*r, A, "w1", 100)?.ok_or("no claim")?;
    st(r.commit_output(A, &a, 0, "w1", c.fence_token, 101, "P first"))?;
    if !matches!(r.commit_output(A, &a, 0, "w1", c.fence_token, 102, "P second"), Err(RepoError::Conflict(_))) {
        return Err("second commit of the same step was not a Conflict".into());
    }
    st(r.commit_output(A, &a, 1, "w1", c.fence_token, 102, "P other"))?;
    eq("first wins", st(r.output(A, &a, 0))?, Some("P first".to_string()))
}

fn crash_then_reclaim(make: Make) -> Result<(), String> {
    let r = make();
    let a = st(r.admit(A))?;
    let c1 = claim(&*r, A, "w1", 100)?.ok_or("no claim")?;
    st(r.commit_output(A, &a, 0, "w1", c1.fence_token, 101, "P zero"))?;
    // w1 crashes before committing step 1
    eq("held", claim(&*r, A, "w2", 129)?, None)?;
    let c2 = claim(&*r, A, "w2", 130)?.ok_or("no reclaim")?;
    eq("attempt+1", (c2.attempt, c2.fence_token), (c1.attempt + 1, c1.fence_token + 1))?;
    eq("out/0 survives", st(r.output(A, &a, 0))?, Some("P zero".to_string()))?;
    st(r.commit_output(A, &a, 1, "w2", c2.fence_token, 131, "P one"))
}

fn touch_lease(make: Make) -> Result<(), String> {
    let r = make();
    let a = st(r.admit(A))?;
    let c = claim(&*r, A, "w1", 100)?.ok_or("no claim")?;
    eq("renew", r.touch_lease(A, &a, "w1", c.fence_token, 120, 30), Ok(150))?;
    eq("not reclaimable at old expiry", claim(&*r, A, "w2", 130)?, None)?;
    eq("other worker cannot renew", r.touch_lease(A, &a, "w2", c.fence_token, 130, 30), Err(RepoError::StaleFence))?;
    eq("expired cannot renew", r.touch_lease(A, &a, "w1", c.fence_token, 150, 30), Err(RepoError::StaleFence))
}
