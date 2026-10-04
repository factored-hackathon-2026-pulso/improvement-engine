//! Monitor loop (stub tick, registration point) and engine job worker (lease, retry, resume after kill, stop).
use pg::repo::{Claimed, JobRepository, MemRepo};
use pulso::config::DataMode;
use pulso::run::log::Logger;
use pulso::run::supervisor::{StopToken, Task};
use pulso::run::tasks::{JobCtx, JobRunner, JobWorker, MonitorTask, StubTick, Tick, TickCtx, TickReport};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn log() -> Logger {
    let sink: Box<dyn std::io::Write + Send> = Box::new(std::io::sink());
    Logger::new(sink)
}

fn wait_for(mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn run_in_thread(mut t: impl Task, stop: &StopToken) -> std::thread::JoinHandle<Result<(), String>> {
    let s = stop.clone();
    std::thread::spawn(move || t.run(&s))
}

// ---- monitor ----

struct Counting {
    ticks: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<(String, u32)>>>,
    fail_first: usize,
    always_fail: bool,
}
impl Tick for Counting {
    fn tick(&mut self, ctx: &TickCtx) -> Result<TickReport, String> {
        let n = self.ticks.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push((ctx.adapter.clone(), ctx.batch_cap));
        if self.always_fail || n < self.fail_first { Err("source unavailable".into()) } else { Ok(TickReport { processed: 1 }) }
    }
}

fn ctx() -> TickCtx {
    TickCtx { data_mode: DataMode::Dataset, adapter: "stub".into(), batch_cap: 7 }
}

fn counting(fail_first: usize, always_fail: bool) -> (Counting, Arc<AtomicUsize>, Arc<Mutex<Vec<(String, u32)>>>) {
    let (ticks, seen) = (Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(vec![])));
    (Counting { ticks: ticks.clone(), seen: seen.clone(), fail_first, always_fail }, ticks, seen)
}

#[test]
fn monitor_ticks_every_interval_with_its_context_and_stops_promptly() {
    let (c, ticks, seen) = counting(0, false);
    let stop = StopToken::new();
    let h = run_in_thread(MonitorTask::new(Box::new(c), ctx(), Duration::from_millis(10), log()), &stop);
    wait_for(|| ticks.load(Ordering::SeqCst) >= 3);
    let t = Instant::now();
    stop.stop();
    assert_eq!(h.join().unwrap(), Ok(()));
    assert!(t.elapsed() < Duration::from_secs(1));
    assert!(seen.lock().unwrap().iter().all(|(a, cap)| a == "stub" && *cap == 7));
}

#[test]
fn monitor_survives_transient_tick_errors_but_gives_up_after_too_many_in_a_row() {
    let (c, ticks, _) = counting(2, false);
    let stop = StopToken::new();
    let h = run_in_thread(MonitorTask::new(Box::new(c), ctx(), Duration::from_millis(5), log()), &stop);
    wait_for(|| ticks.load(Ordering::SeqCst) >= 6);
    stop.stop();
    assert_eq!(h.join().unwrap(), Ok(()), "two failures then success must not kill the task");

    let (c, ticks, _) = counting(0, true);
    let stop = StopToken::new();
    let h = run_in_thread(MonitorTask::new(Box::new(c), ctx(), Duration::from_millis(1), log()), &stop);
    let err = h.join().unwrap().expect_err("persistent failure ends the task");
    assert!(err.contains("consecutive"), "{err}");
    assert_eq!(ticks.load(Ordering::SeqCst), 5);
}

#[test]
fn the_stub_tick_reports_zero() {
    assert_eq!(StubTick.tick(&ctx()).unwrap(), TickReport { processed: 0 });
}

// ---- worker ----

struct Recorder {
    seen: Mutex<Vec<(String, u64, u64)>>,
    fail_attempts_below: u64,
}
impl JobRunner for Recorder {
    fn run(&self, job: &Claimed, ctx: &JobCtx) -> Result<(), String> {
        self.seen.lock().unwrap().push((job.job.clone(), job.fence_token, job.attempt));
        if job.attempt < self.fail_attempts_below {
            return Err("transient".into());
        }
        ctx.repo.commit_output(ctx.tenant, &job.job, 0, ctx.worker, job.fence_token, ctx.now, "done").map_err(|e| format!("{e:?}"))
    }
}

fn worker(repo: Arc<MemRepo>, runner: Option<Arc<dyn JobRunner>>, clock: Arc<AtomicU64>, batch_cap: u32) -> JobWorker {
    JobWorker {
        repo,
        runner,
        tenant: "t1".into(),
        worker_id: "w-live".into(),
        poll: Duration::from_millis(5),
        batch_cap,
        lease_seconds: 10,
        clock: Arc::new(move || clock.load(Ordering::SeqCst)),
        log: log(),
    }
}

#[test]
fn worker_claims_runs_and_commits_jobs_in_admission_order() {
    let repo = Arc::new(MemRepo::new());
    let ids: Vec<String> = (0..3).map(|_| repo.admit("t1").unwrap()).collect();
    let rec = Arc::new(Recorder { seen: Mutex::new(vec![]), fail_attempts_below: 0 });
    let stop = StopToken::new();
    let h = run_in_thread(worker(repo.clone(), Some(rec.clone()), Arc::new(AtomicU64::new(1000)), 10), &stop);
    wait_for(|| rec.seen.lock().unwrap().len() == 3);
    stop.stop();
    assert_eq!(h.join().unwrap(), Ok(()));
    let order: Vec<String> = rec.seen.lock().unwrap().iter().map(|s| s.0.clone()).collect();
    assert_eq!(order, ids);
    for id in &ids {
        assert_eq!(repo.output("t1", id, 0).unwrap().as_deref(), Some("done"));
    }
}

#[test]
fn worker_resumes_a_job_whose_holder_was_killed_once_its_lease_expires() {
    let repo = Arc::new(MemRepo::new());
    let id = repo.admit("t1").unwrap();
    // a worker that died mid-job: it claimed at t=100 for 10 s and never committed or touched again
    let dead = repo.claim_next("t1", "w-dead", 100, 10).unwrap().unwrap();
    assert_eq!((dead.fence_token, dead.attempt), (1, 1));
    let clock = Arc::new(AtomicU64::new(105));
    let rec = Arc::new(Recorder { seen: Mutex::new(vec![]), fail_attempts_below: 0 });
    let stop = StopToken::new();
    let h = run_in_thread(worker(repo.clone(), Some(rec.clone()), clock.clone(), 10), &stop);
    std::thread::sleep(Duration::from_millis(80));
    assert!(rec.seen.lock().unwrap().is_empty(), "a live lease must not be stolen");
    clock.store(111, Ordering::SeqCst);
    wait_for(|| !rec.seen.lock().unwrap().is_empty());
    stop.stop();
    h.join().unwrap().unwrap();
    assert_eq!(rec.seen.lock().unwrap()[0], (id.clone(), 2, 2), "fence and attempt were bumped by the reclaim");
    // the dead worker's late commit is refused: stale fence
    assert!(repo.commit_output("t1", &id, 0, "w-dead", dead.fence_token, 111, "late").is_err());
}

#[test]
fn a_failing_job_is_retried_after_its_lease_expires() {
    let repo = Arc::new(MemRepo::new());
    let id = repo.admit("t1").unwrap();
    let clock = Arc::new(AtomicU64::new(1000));
    let rec = Arc::new(Recorder { seen: Mutex::new(vec![]), fail_attempts_below: 2 });
    let stop = StopToken::new();
    let h = run_in_thread(worker(repo.clone(), Some(rec.clone()), clock.clone(), 10), &stop);
    wait_for(|| rec.seen.lock().unwrap().len() == 1);
    clock.store(1011, Ordering::SeqCst);
    wait_for(|| repo.output("t1", &id, 0).unwrap().is_some());
    stop.stop();
    h.join().unwrap().unwrap();
    assert_eq!(rec.seen.lock().unwrap().iter().map(|s| s.2).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn without_a_runner_the_worker_is_idle_and_claims_nothing() {
    let repo = Arc::new(MemRepo::new());
    repo.admit("t1").unwrap();
    let stop = StopToken::new();
    let h = run_in_thread(worker(repo.clone(), None, Arc::new(AtomicU64::new(1000)), 10), &stop);
    std::thread::sleep(Duration::from_millis(60));
    stop.stop();
    h.join().unwrap().unwrap();
    let claimed = repo.claim_next("t1", "probe", 1000, 10).unwrap();
    assert_eq!(claimed.unwrap().attempt, 1, "nobody claimed it before");
}

struct Slow {
    started: Arc<AtomicUsize>,
}
impl JobRunner for Slow {
    fn run(&self, _job: &Claimed, ctx: &JobCtx) -> Result<(), String> {
        self.started.fetch_add(1, Ordering::SeqCst);
        if ctx.stop.wait(Duration::from_secs(30)) { Err("interrupted".into()) } else { Ok(()) }
    }
}

#[test]
fn stop_interrupts_the_running_job_and_claims_no_more_work() {
    let repo = Arc::new(MemRepo::new());
    repo.admit("t1").unwrap();
    repo.admit("t1").unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let stop = StopToken::new();
    let h = run_in_thread(worker(repo.clone(), Some(Arc::new(Slow { started: started.clone() })), Arc::new(AtomicU64::new(1000)), 10), &stop);
    wait_for(|| started.load(Ordering::SeqCst) == 1);
    let t = Instant::now();
    stop.stop();
    assert_eq!(h.join().unwrap(), Ok(()));
    assert!(t.elapsed() < Duration::from_secs(1));
    assert_eq!(started.load(Ordering::SeqCst), 1, "the second job was never claimed after stop");
}

#[test]
fn batch_cap_bounds_jobs_per_cycle() {
    let repo = Arc::new(MemRepo::new());
    for _ in 0..4 {
        repo.admit("t1").unwrap();
    }
    let rec = Arc::new(Recorder { seen: Mutex::new(vec![]), fail_attempts_below: 0 });
    let stop = StopToken::new();
    // a long poll: after the first cycle the worker sleeps, so only batch_cap jobs run promptly
    let mut w = worker(repo, Some(rec.clone()), Arc::new(AtomicU64::new(1000)), 2);
    w.poll = Duration::from_secs(30);
    let h = run_in_thread(w, &stop);
    wait_for(|| rec.seen.lock().unwrap().len() >= 2);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rec.seen.lock().unwrap().len(), 2);
    stop.stop();
    h.join().unwrap().unwrap();
}

// ---- adversarial review (CL): the missing 'completed' transition in the JobRepository port ----

/// Characterisation of what the port does TODAY: there is no terminal transition, so a job whose every output is
/// committed stays `leased` and is claimable again once the lease lapses (fence bumped). The only thing that stops a
/// duplicate is `commit_output` ("one winner per step"); the side effect a runner performs BEFORE committing is not
/// protected unless the runner consults `output()` first (resume) or uses `begin_effect`.
#[test]
fn a_fully_committed_job_is_reclaimed_after_lease_expiry_but_cannot_overwrite_its_output() {
    let repo = MemRepo::new();
    let id = repo.admit("t1").unwrap();
    let first = repo.claim_next("t1", "w-a", 100, 10).unwrap().unwrap();
    repo.commit_output("t1", &id, 0, "w-a", first.fence_token, 101, "done").unwrap();
    assert!(repo.claim_next("t1", "w-b", 105, 10).unwrap().is_none(), "lease still held");
    let again = repo.claim_next("t1", "w-b", 111, 10).unwrap().expect("KNOWN GAP: finished job is reclaimable");
    assert!(again.fence_token > first.fence_token && again.attempt == 2);
    assert!(repo.commit_output("t1", &id, 0, "w-b", again.fence_token, 112, "done again").is_err(), "output stays first-writer-wins");
    assert_eq!(repo.output("t1", &id, 0).unwrap().as_deref(), Some("done"));
}

struct SideEffect(Arc<AtomicUsize>);
impl JobRunner for SideEffect {
    fn run(&self, job: &Claimed, ctx: &JobCtx) -> Result<(), String> {
        self.0.fetch_add(1, Ordering::SeqCst); // the external effect (e.g. a gateway call), before the commit
        ctx.repo.commit_output(ctx.tenant, &job.job, 0, ctx.worker, job.fence_token, ctx.now, "done").map_err(|e| format!("{e:?}"))
    }
}

/// DESIRED behaviour: one job, one execution, even when the clock moves past the lease after completion.
/// Fails today (runs twice) because the port has no `complete`; fix = `complete(tenant, job, worker, fence)` setting
/// status 'complete' (already admitted by migration 0050's CHECK) and excluding it from `claim_next`, called by the
/// worker on `Ok`. Ignored so the suite stays green; run with `--ignored` to see it fail.
#[test]
#[ignore = "KNOWN GAP: JobRepository has no completed transition; see doc comment"]
fn a_job_whose_runner_returned_ok_is_never_run_again() {
    let repo = Arc::new(MemRepo::new());
    repo.admit("t1").unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let clock = Arc::new(AtomicU64::new(100));
    let w = worker(repo, Some(Arc::new(SideEffect(runs.clone()))), clock.clone(), 1);
    let stop = StopToken::new();
    let h = run_in_thread(w, &stop);
    wait_for(|| runs.load(Ordering::SeqCst) >= 1);
    clock.store(1_000, Ordering::SeqCst); // lease (10 s) long expired
    std::thread::sleep(Duration::from_millis(100));
    stop.stop();
    h.join().unwrap().unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the finished job was executed again after its lease expired");
}
