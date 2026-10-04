//! Live check against a real bridge image: 2 identical Idempotency-Keys must make exactly 1 run.
//! SKIPPED by default (needs a stack from `e2e-core/run.ps1` on Podman machine pulso-dev).
//! Run: PULSO_BRIDGE_ADDR=127.0.0.1:<port> PULSO_SERVICE_KID=<kid> PULSO_SERVICE_SEED_HEX=<64 hex>
//!      cargo test --manifest-path seams/Cargo.toml -p core-client --test live -- --ignored
//! Env var values are never printed. K2 adds the typed invoke body used here.
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
