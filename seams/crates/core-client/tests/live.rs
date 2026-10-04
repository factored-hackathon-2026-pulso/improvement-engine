//! Live check against a real bridge image: 2 identical Idempotency-Keys must make exactly 1 run.
//! SKIPPED by default (needs a stack from `e2e-core/run.ps1` on Podman machine pulso-dev).
//! Run: PULSO_BRIDGE_ADDR=127.0.0.1:<port> PULSO_SERVICE_KID=<kid> PULSO_SERVICE_SEED_HEX=<64 hex>
//!      cargo test --manifest-path seams/Cargo.toml -p core-client --test live -- --ignored
//! Env var values are never printed. K2 adds the typed ops: `live_typed_replay_is_one_run` also needs a seeded scout
//! release (`PULSO_LIVE_SCOUT_RELEASE`, `PULSO_LIVE_TENANT`; the stack's world file). Not run in CI: no live stack there.
use core_client::{ClientConfig, CoreClient, routes};

#[test]
#[ignore = "needs a live bridge stack; see module docs"]
fn live_version_matches_pin() {
    let addr = std::env::var("PULSO_BRIDGE_ADDR").expect("PULSO_BRIDGE_ADDR");
    let kid = std::env::var("PULSO_SERVICE_KID").expect("PULSO_SERVICE_KID");
    let hex = std::env::var("PULSO_SERVICE_SEED_HEX").expect("PULSO_SERVICE_SEED_HEX");
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex seed");
    }
    let c = CoreClient::new(ClientConfig::new(&addr, &kid, seed, "k1-live"));
    let v = c.call(&routes::VERSION, "", None, &[], None, None).expect("version");
    assert_eq!(v.body["agent_core_sha"], core_client::pins::AGENT_CORE_SHA);
    assert_eq!(v.body["contracts_version"], core_client::pins::CONTRACTS_VERSION);
}

#[test]
#[ignore = "needs a live bridge stack and a seeded scout release; see module docs"]
fn live_typed_replay_is_one_run() {
    use core_client::dto::{Stage, TaskInvocation};
    let addr = std::env::var("PULSO_BRIDGE_ADDR").expect("PULSO_BRIDGE_ADDR");
    let kid = std::env::var("PULSO_SERVICE_KID").expect("PULSO_SERVICE_KID");
    let hex = std::env::var("PULSO_SERVICE_SEED_HEX").expect("PULSO_SERVICE_SEED_HEX");
    let tenant = std::env::var("PULSO_LIVE_TENANT").unwrap_or_else(|_| "t1".into());
    let release = std::env::var("PULSO_LIVE_SCOUT_RELEASE").expect("PULSO_LIVE_SCOUT_RELEASE");
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex seed");
    }
    let c = CoreClient::new(ClientConfig::new(&addr, &kid, seed, "k2-live"));
    c.version().expect("version").check_pin().expect("pin");
    let job = format!("job-k2-live-{}", std::process::id());
    let mut inv = TaskInvocation::new(&tenant, &job, Stage::Scout, "k2-live-1");
    inv.agent_id = "pulso-scout".into();
    inv.agent_version = "1.0.0".into();
    inv.release_id = release;
    inv.pulso_run_ref = format!("pr-{job}");
    inv.lab_grant_ref = "grant-live".into();
    inv.input = serde_json::json!({"briefing_ref": "wiki/briefing.md"});
    let (a, b) = (c.invoke(&inv).expect("first"), c.invoke(&inv).expect("replay"));
    assert_eq!(a.core_run_id, b.core_run_id, "an identical replay must be the same run");
}
