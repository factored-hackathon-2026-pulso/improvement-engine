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
