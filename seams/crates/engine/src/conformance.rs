//! Executor conformance suite, reusable over any `JobStore` backend (FileStore here, PgJobStore in the pg
//! crate). Scenarios are the E2 executor contract: C-7 lease/fence/effect semantics plus the fence-tied
//! `out/N` commit. `run_suite` returns one line per failing scenario (empty = conforms).
use crate::executor::{execute, read_lease, ExecError, ExecOptions};
use crate::{demo, event_log, CommitGuard, JobStore};
use abi::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

pub const GOLDEN: &str = include_str!("../tests/golden/demo3.events");

pub trait Backend {
    /// A handle on a brand-new, empty job named `name`.
    fn fresh(&self, name: &str) -> Box<dyn JobStore>;
    /// A `'static` opener of further handles (other connections) on the job last created as `name`.
    fn reopener(&self, name: &str) -> Arc<dyn Fn() -> Box<dyn JobStore>>;
    fn reopen(&self, name: &str) -> Box<dyn JobStore> {
        (self.reopener(name))()
    }
}

type Scenario = fn(&dyn Backend) -> Result<(), String>;

pub const SCENARIOS: &[(&str, Scenario)] = &[
    ("golden_3_handler_sequence", golden),
    ("lease_reclaimable_exactly_at_expiry_with_new_fence_and_attempt", lease_reclaim),
    ("superseded_worker_cannot_commit", superseded),
    ("superseded_worker_cannot_commit_after_its_last_check", superseded_after_last_check),
    ("commit_guard_rejects_older_fence_expired_and_duplicate", commit_guard),
    ("worker_past_its_own_lease_cannot_commit", past_own_lease),
    ("committed_effect_is_skipped_on_resume", effect_skipped),
    ("uncommitted_effect_dispatch_blocks_resume", effect_blocks),
    ("failed_pure_handler_is_retried", retry_pure),
    ("max_attempts_exhausted", max_attempts),
];

pub fn run_suite(b: &dyn Backend) -> Vec<String> {
    let mut failures = vec![];
    for (name, f) in SCENARIOS {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(b))) {
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
fn opts(worker: &str, now: u64) -> ExecOptions {
    ExecOptions::new("job-1", worker, now)
}
fn pure(payload: &str) -> OutputEnvelope {
    OutputEnvelope { payload: payload.into(), events: vec![], effect: EffectState::NoEffect }
}

fn golden(b: &dyn Backend) -> Result<(), String> {
    let s = b.fresh("golden");
    eq("payload", execute(&*s, &demo::handlers(), "x", &opts("w1", 1000)), Ok("xabc".to_string()))?;
    eq("log", event_log(&*s, 3)?.join("\n") + "\n", GOLDEN.to_string())
}

fn lease_reclaim(b: &dyn Backend) -> Result<(), String> {
    let s = b.fresh("lease");
    let mut o = opts("w1", 100);
    o.crash_after = Some(0);
    eq("crash", execute(&*s, &demo::handlers(), "x", &o), Err(ExecError::Crashed(0)))?;
    let l1 = read_lease(&*s).map_err(|e| format!("{e:?}"))?.ok_or("no lease")?;
    eq("l1", (l1.fence_token, l1.attempt, l1.expires_at), (1, 1, 160))?;
    eq("held", execute(&*s, &demo::handlers(), "x", &opts("w2", 159)), Err(ExecError::LeaseHeld { expires_at: 160 }))?;
    eq("resumed", execute(&*s, &demo::handlers(), "x", &opts("w2", 160)), Ok("xabc".to_string()))?;
    let l2 = read_lease(&*s).map_err(|e| format!("{e:?}"))?.ok_or("no lease")?;
    eq("l2", (l2.worker_id.as_str(), l2.fence_token, l2.attempt), ("w2", 2, 2))
}

/// Handler that, mid-run, lets a second worker reclaim (after expiry) and finish the job.
struct Usurped(Arc<dyn Fn() -> Box<dyn JobStore>>);
impl JobHandler for Usurped {
    fn id(&self) -> HandlerId { HandlerId("usurped".into()) }
    fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        let b = (self.0)();
        let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(demo::Append("B"))];
        execute(&*b, &hs, "x", &opts("w2", 500)).unwrap();
        Ok(OutputEnvelope { payload: format!("{}A", i.payload), events: vec!["from-a".into()], effect: EffectState::NoEffect })
    }
}

fn superseded(b: &dyn Backend) -> Result<(), String> {
    let s = b.fresh("stale");
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Usurped(b.reopener("stale")))];
    eq("w1", execute(&*s, &hs, "x", &opts("w1", 100)), Err(ExecError::StaleFence))?;
    eq("log", event_log(&*s, 1)?, vec!["step0:B:x".to_string()])
}

/// Store wrapper: right after the executor's lease renewal (the 2nd `lease` CAS: the 1st is the claim) a
/// skewed-clock second worker reclaims, i.e. the fence moves AFTER the executor's last verification and
/// BEFORE its out/N commit. Only a commit that re-checks the fence atomically can refuse the stale worker.
struct Usurp<'a> {
    inner: Box<dyn JobStore>,
    other: Box<dyn Fn() + 'a>,
    lease_cas: AtomicUsize,
}
impl JobStore for Usurp<'_> {
    fn get(&self, key: &str) -> Result<Option<(u64, String)>, String> { self.inner.get(key) }
    fn cas(&self, key: &str, expected: u64, value: &str) -> Result<u64, String> {
        let r = self.inner.cas(key, expected, value);
        if key == "lease" && self.lease_cas.fetch_add(1, Ordering::SeqCst) == 1 {
            (self.other)();
        }
        r
    }
    fn commit_guarded(&self, key: &str, value: &str, guard: &CommitGuard) -> Result<u64, String> {
        self.inner.commit_guarded(key, value, guard)
    }
}

fn superseded_after_last_check(b: &dyn Backend) -> Result<(), String> {
    let s = Usurp {
        inner: b.fresh("late"),
        other: Box::new(|| {
            let o = b.reopen("late");
            // w2's clock is far ahead, so the live lease looks expired to it; zero handlers: claim and release
            let none: Vec<Box<dyn JobHandler>> = vec![];
            execute(&*o, &none, "x", &opts("w2", 10_000)).unwrap();
        }),
        lease_cas: AtomicUsize::new(0),
    };
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(demo::Append("A"))];
    eq("w1 must be refused", execute(&s, &hs, "x", &opts("w1", 100)), Err(ExecError::StaleFence))?;
    eq("out/0 not written by the stale worker", s.get("out/0")?, None)
}

fn commit_guard(b: &dyn Backend) -> Result<(), String> {
    let s = b.fresh("guard");
    s.cas("lease", 0, "w2|2|2|200")?;
    let g = |w: &'static str, f: u64, now: u64| CommitGuard { worker_id: w, fence_token: f, now };
    for (what, guard) in [("older fence, skewed clock", g("w1", 1, 100)), ("right fence wrong worker", g("w1", 2, 100)), ("expired", g("w2", 2, 200))] {
        if s.commit_guarded("out/0", "P x", &guard).is_ok() {
            return Err(format!("{what}: stale commit accepted"));
        }
    }
    eq("nothing written", s.get("out/0")?, None)?;
    s.commit_guarded("out/0", "P new", &g("w2", 2, 150))?;
    if s.commit_guarded("out/0", "P again", &g("w2", 2, 151)).is_ok() {
        return Err("duplicate out/0 accepted".into());
    }
    eq("first wins", s.get("out/0")?.map(|(_, v)| v), Some("P new".to_string()))?;
    // the same worker name reclaimed its own job (fence 2): only the fence can refuse its old attempt
    let same = b.fresh("guard-same-worker");
    same.cas("lease", 0, "w1|2|2|200")?;
    if same.commit_guarded("out/0", "P stale", &g("w1", 1, 100)).is_ok() {
        return Err("same worker, older fence: stale commit accepted".into());
    }
    eq("nothing written by the old fence", same.get("out/0")?, None)?;
    let none = b.fresh("guard-no-lease");
    if none.commit_guarded("out/0", "P x", &g("w1", 1, 1)).is_ok() {
        return Err("commit without any lease accepted".into());
    }
    Ok(())
}

fn past_own_lease(b: &dyn Backend) -> Result<(), String> {
    let s = b.fresh("expired");
    let clock = Arc::new(AtomicU64::new(100));
    struct Slow(Arc<AtomicU64>);
    impl JobHandler for Slow {
        fn id(&self) -> HandlerId { HandlerId("slow".into()) }
        fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
            self.0.store(160, Ordering::SeqCst); // now >= expires
            Ok(pure(&i.payload))
        }
    }
    let c = clock.clone();
    let mut o = opts("w1", 0);
    o.now = Box::new(move || c.load(Ordering::SeqCst));
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Slow(clock))];
    eq("expired", execute(&*s, &hs, "x", &o), Err(ExecError::StaleFence))?;
    eq("no out/0", s.get("out/0")?, None)
}

struct Eff { runs: Arc<AtomicUsize>, fail: bool }
impl JobHandler for Eff {
    fn id(&self) -> HandlerId { HandlerId("eff".into()) }
    fn effectful(&self) -> bool { true }
    fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if self.fail { return Err(HandlerError::Failed("dispatch lost".into())); }
        Ok(OutputEnvelope { payload: i.payload.clone(), events: vec!["applied".into()], effect: EffectState::AppliedAcknowledged })
    }
}

fn effect_skipped(b: &dyn Backend) -> Result<(), String> {
    let s = b.fresh("eff-ok");
    let runs = Arc::new(AtomicUsize::new(0));
    let hs = || -> Vec<Box<dyn JobHandler>> { vec![Box::new(Eff { runs: runs.clone(), fail: false }), Box::new(demo::Append("z"))] };
    let mut o = opts("w1", 0);
    o.crash_after = Some(0);
    eq("crash", execute(&*s, &hs(), "x", &o), Err(ExecError::Crashed(0)))?;
    eq("resume", execute(&*s, &hs(), "x", &opts("w2", 60)), Ok("xz".to_string()))?;
    eq("effect runs", runs.load(Ordering::SeqCst), 1)?;
    eq("log", event_log(&*s, 2)?, vec!["applied".to_string(), "step1:z:x".to_string()])
}

fn effect_blocks(b: &dyn Backend) -> Result<(), String> {
    let s = b.fresh("eff-unknown");
    let runs = Arc::new(AtomicUsize::new(0));
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Eff { runs: runs.clone(), fail: true })];
    eq("w1", execute(&*s, &hs, "x", &opts("w1", 0)), Err(ExecError::Handler(HandlerError::Failed("dispatch lost".into()))))?;
    eq("w2", execute(&*s, &hs, "x", &opts("w2", 60)), Err(ExecError::NeedsReconciliation(0)))?;
    eq("never re-dispatched", runs.load(Ordering::SeqCst), 1)
}

fn retry_pure(b: &dyn Backend) -> Result<(), String> {
    struct Flaky(Arc<AtomicUsize>);
    impl JobHandler for Flaky {
        fn id(&self) -> HandlerId { HandlerId("flaky".into()) }
        fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 { return Err(HandlerError::Failed("transient".into())); }
            Ok(pure(&i.payload))
        }
    }
    let s = b.fresh("flaky");
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Flaky(Arc::new(AtomicUsize::new(0))))];
    if execute(&*s, &hs, "x", &opts("w1", 0)).is_ok() {
        return Err("first run should fail".into());
    }
    eq("retry", execute(&*s, &hs, "x", &opts("w2", 60)), Ok("x".to_string()))
}

fn max_attempts(b: &dyn Backend) -> Result<(), String> {
    struct Bad;
    impl JobHandler for Bad {
        fn id(&self) -> HandlerId { HandlerId("bad".into()) }
        fn run(&self, _f: &Fence, _i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
            Err(HandlerError::Failed("always".into()))
        }
    }
    let s = b.fresh("maxattempts");
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Bad)];
    for k in 0..3u64 {
        let mut o = opts("w", k * 60);
        o.max_attempts = 3;
        if !matches!(execute(&*s, &hs, "x", &o), Err(ExecError::Handler(_))) {
            return Err(format!("attempt {k} did not fail in the handler"));
        }
    }
    let mut o = opts("w", 3 * 60);
    o.max_attempts = 3;
    eq("exhausted", execute(&*s, &hs, "x", &o), Err(ExecError::AttemptsExhausted(3)))
}
