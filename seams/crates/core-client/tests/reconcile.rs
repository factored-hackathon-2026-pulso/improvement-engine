//! K5a: the three fault cases yield exactly one server-side effect.
mod common;
use common::armcore::{ArmCore, Fault};
use common::{KID, SEED};
use core_client::dto::ArmRequest;
use core_client::{ClientConfig, CoreClient};
use serde_json::json;

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
    let rep = client(&core.addr).run_arm_reconciled("t1", Some("job-1"), &req()).unwrap();
    assert_eq!(core.effects(), 1, "lost-response must not create a duplicate");
    assert_eq!(rep.execution_id, core_client::canon::arm_execution_id("t1", "k5a-arm-1").unwrap());
}
