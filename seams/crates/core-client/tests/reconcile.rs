//! K5a: the three fault cases yield exactly one server-side effect.
mod common;
use common::armcore::{ArmCore, Fault};
use common::{KID, SEED};
use core_client::dto::ArmRequest;
use core_client::{ClientConfig, CoreClient};
use core_client::reconcile::{Clock, RetryPolicy};
use serde_json::json;
use std::sync::Mutex;
use std::time::Duration;

/// Records requested waits instead of sleeping.
#[derive(Default)]
struct TestClock(Mutex<Vec<Duration>>);
impl Clock for TestClock {
    fn sleep(&self, d: Duration) {
        self.0.lock().unwrap().push(d);
    }
}

fn client(addr: &str) -> CoreClient {
    let mut cfg = ClientConfig::new(addr, KID, SEED, "bridge-1");
    cfg.timeout = std::time::Duration::from_secs(5);
    CoreClient::new(cfg)
}

fn req() -> ArmRequest {
    let mut r = base();
    r.execution_profile = Some(core_client::dto::ExecutionProfile::EvolutionTask);
    r
}

fn base() -> ArmRequest {
    ArmRequest::new("k5a-arm-1", "bind-arm", "case-1", "baseline", 0, json!(7), json!({"kind":"published_release","release_id":"rel-demo"}), "art-golden", "bud-1")
}

#[test]
fn lost_response_returns_the_same_run_with_one_effect() {
    let core = ArmCore::start(vec![Fault::DropAfterEffect]);
    let rep = client(&core.addr).run_arm_reconciled("t1", Some("job-1"), &req(), &RetryPolicy::default(), &TestClock::default()).unwrap();
    assert_eq!(core.effects(), 1, "lost-response must not create a duplicate");
    assert_eq!(rep.execution_id, core_client::canon::arm_execution_id("t1", "k5a-arm-1").unwrap());
}

fn run(core: &ArmCore, clock: &TestClock, policy: &RetryPolicy) -> Result<core_client::dto::ArmReport, core_client::OpError> {
    client(&core.addr).run_arm_reconciled("t1", Some("job-1"), &req(), policy, clock)
}

#[test]
fn throttle_honours_retry_after_and_reuses_the_key() {
    let core = ArmCore::start(vec![Fault::Throttle(Some(3))]);
    let clock = TestClock::default();
    run(&core, &clock, &RetryPolicy::default()).unwrap();
    assert_eq!(core.effects(), 1);
    assert_eq!(*clock.0.lock().unwrap(), vec![Duration::from_secs(3)], "must wait exactly Retry-After");
    let keys: Vec<_> = core.requests().into_iter().map(|r| r.2).collect();
    assert_eq!(keys, vec![Some("k5a-arm-1".to_string()); 2]);
}

#[test]
fn throttle_is_bounded_and_retry_after_is_capped() {
    let core = ArmCore::start(vec![Fault::Throttle(Some(3600)); 5]);
    let clock = TestClock::default();
    let p = RetryPolicy::default();
    assert!(run(&core, &clock, &p).is_err());
    assert_eq!(core.requests().len(), 3, "default 3 attempts");
    assert_eq!(core.effects(), 0);
    assert!(clock.0.lock().unwrap().iter().all(|d| *d <= p.cap), "waits capped");
}

#[test]
fn unknown_error_classification_fails_closed() {
    let core = ArmCore::start(vec![Fault::ThrottleUnknownCode]);
    let clock = TestClock::default();
    assert!(run(&core, &clock, &RetryPolicy::default()).is_err());
    assert_eq!(core.requests().len(), 1, "unknown code is never retried");
    assert!(clock.0.lock().unwrap().is_empty());
}

#[test]
fn relay_drops_the_response_after_the_effect_one_effect() {
    let core = ArmCore::start(vec![]);
    let relay = common::relay::TcpRelay::start(&core.addr, 1);
    let rep = client(&relay.addr).run_arm_reconciled("t1", Some("job-1"), &req(), &RetryPolicy::default(), &TestClock::default()).unwrap();
    assert_eq!(relay.dropped.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(core.effects(), 1);
    assert_eq!(rep.execution_id, core_client::canon::arm_execution_id("t1", "k5a-arm-1").unwrap());
}

#[test]
fn request_lost_before_the_effect_is_absent_then_sent_once_with_the_same_key() {
    let core = ArmCore::start(vec![Fault::DropBeforeEffect]);
    run(&core, &TestClock::default(), &RetryPolicy::default()).unwrap();
    assert_eq!(core.effects(), 1);
    let keys: Vec<_> = core.requests().into_iter().filter(|r| r.0 == "POST").map(|r| r.2).collect();
    assert_eq!(keys, vec![Some("k5a-arm-1".to_string()); 2]);
}

#[test]
fn crash_after_write_reconcile_finds_the_effect_or_reports_absent() {
    use core_client::reconcile::Reconciled;
    // the first process wrote the effect and died before recording it (no response ever read)
    let core = ArmCore::start(vec![Fault::DropAfterEffect]);
    let first = client(&core.addr).with_attempts(1);
    let _ = first.run_arm("t1", Some("job-1"), &req()); // outcome lost to the "crash"
    drop(first);
    assert_eq!(core.effects(), 1);
    // restarted process: only the key survives
    let resumed = client(&core.addr);
    match resumed.reconcile_arm("t1", Some("job-1"), "k5a-arm-1").unwrap() {
        Reconciled::Found(r) => assert_eq!(r.execution_id, core_client::canon::arm_execution_id("t1", "k5a-arm-1").unwrap()),
        Reconciled::Absent => panic!("effect exists"),
    }
    assert_eq!(resumed.reconcile_arm("t1", Some("job-1"), "never-sent").unwrap(), Reconciled::Absent);
    assert_eq!(core.effects(), 1, "reconcile is read-only");
    // and a re-run after the crash replays instead of double-effecting
    resumed.run_arm_reconciled("t1", Some("job-1"), &req(), &RetryPolicy::default(), &TestClock::default()).unwrap();
    assert_eq!(core.effects(), 1);
}
