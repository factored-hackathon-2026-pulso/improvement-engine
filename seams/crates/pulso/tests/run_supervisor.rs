//! Supervisor: start, observe, graceful stop, bounded cut of a task that ignores the stop, failure reporting.
use pulso::health::{DbProbe, Health, Migrations};
use pulso::run::log::Logger;
use pulso::run::supervisor::{StopToken, Supervisor, Task};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Up;
impl DbProbe for Up {
    fn ping(&self) -> Result<(), String> {
        Ok(())
    }
}

fn setup(grace_ms: u64) -> (Arc<Health>, Supervisor) {
    let h = Health::new(Arc::new(Up));
    h.set_migrations(Migrations::NotApplicable);
    let sink: Box<dyn std::io::Write + Send> = Box::new(std::io::sink());
    let s = Supervisor::new(h.clone(), Logger::new(sink), Duration::from_millis(grace_ms));
    (h, s)
}

struct Cooperative {
    name: &'static str,
    ticks: Arc<AtomicUsize>,
    stopped_cleanly: Arc<AtomicBool>,
}
impl Task for Cooperative {
    fn name(&self) -> String {
        self.name.into()
    }
    fn run(&mut self, stop: &StopToken) -> Result<(), String> {
        while !stop.wait(Duration::from_millis(5)) {
            self.ticks.fetch_add(1, Ordering::SeqCst);
        }
        self.stopped_cleanly.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct Stubborn;
impl Task for Stubborn {
    fn name(&self) -> String {
        "stubborn".into()
    }
    fn run(&mut self, _stop: &StopToken) -> Result<(), String> {
        std::thread::sleep(Duration::from_secs(20));
        Ok(())
    }
}

struct Failing(bool);
impl Task for Failing {
    fn name(&self) -> String {
        "failing".into()
    }
    fn run(&mut self, _stop: &StopToken) -> Result<(), String> {
        if self.0 {
            panic!("task blew up");
        }
        Err("lost the database".into())
    }
}

fn wait_for(mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn stop_token_wakes_waiters_early_and_is_sticky() {
    let t = StopToken::new();
    assert!(!t.is_stopped());
    assert!(!t.wait(Duration::from_millis(10)), "times out while running");
    let t2 = t.clone();
    let h = std::thread::spawn(move || {
        let start = Instant::now();
        let stopped = t2.wait(Duration::from_secs(10));
        (stopped, start.elapsed())
    });
    std::thread::sleep(Duration::from_millis(30));
    t.stop();
    let (stopped, took) = h.join().unwrap();
    assert!(stopped && took < Duration::from_secs(2), "{took:?}");
    assert!(t.is_stopped() && t.wait(Duration::from_secs(10)), "sticky and immediate once stopped");
}

#[test]
fn tasks_become_ready_then_stop_gracefully_with_exit_zero() {
    let (h, mut s) = setup(2000);
    let (ticks, ok1, ok2) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    s.add(Box::new(Cooperative { name: "monitor", ticks: ticks.clone(), stopped_cleanly: ok1.clone() }));
    s.add(Box::new(Cooperative { name: "worker", ticks: ticks.clone(), stopped_cleanly: ok2.clone() }));
    let stop = s.stop_token();
    let run = std::thread::spawn(move || s.run());
    wait_for(|| h.readiness().is_ok() && ticks.load(Ordering::SeqCst) > 2);
    stop.stop();
    let out = run.join().unwrap();
    assert_eq!(out.exit_code(), 0, "{out:?}");
    assert!(out.cut.is_empty() && out.failed.is_empty());
    assert!(ok1.load(Ordering::SeqCst) && ok2.load(Ordering::SeqCst), "both tasks saw the stop and returned");
    assert_eq!(h.readiness().unwrap_err(), "shutting_down");
}

#[test]
fn a_task_that_ignores_stop_is_cut_at_the_deadline() {
    let (_h, mut s) = setup(150);
    let ok = Arc::new(AtomicBool::new(false));
    s.add(Box::new(Stubborn));
    s.add(Box::new(Cooperative { name: "polite", ticks: Arc::new(AtomicUsize::new(0)), stopped_cleanly: ok.clone() }));
    let stop = s.stop_token();
    stop.stop();
    let start = Instant::now();
    let out = s.run();
    assert!(start.elapsed() < Duration::from_secs(3), "must not wait for the stubborn task: {:?}", start.elapsed());
    assert_eq!(out.cut, vec!["stubborn".to_string()]);
    assert_eq!(out.exit_code(), 3);
    assert!(ok.load(Ordering::SeqCst), "the polite task still shut down cleanly");
}

#[test]
fn failed_and_panicking_tasks_turn_readiness_red_and_exit_nonzero() {
    for panics in [false, true] {
        let (h, mut s) = setup(1000);
        s.add(Box::new(Failing(panics)));
        let stop = s.stop_token();
        let h2 = h.clone();
        let run = std::thread::spawn(move || s.run());
        wait_for(|| h2.readiness() == Err("task_dead:failing".to_string()));
        stop.stop();
        let out = run.join().unwrap();
        assert_eq!(out.failed, vec!["failing".to_string()], "panics={panics}");
        assert_eq!(out.exit_code(), 1);
    }
}

#[test]
fn stop_before_run_still_starts_and_stops_every_task_once() {
    let (_h, mut s) = setup(1000);
    let ok = Arc::new(AtomicBool::new(false));
    s.add(Box::new(Cooperative { name: "late", ticks: Arc::new(AtomicUsize::new(0)), stopped_cleanly: ok.clone() }));
    s.stop_token().stop();
    let out = s.run();
    assert_eq!(out.exit_code(), 0);
    assert!(ok.load(Ordering::SeqCst));
}
