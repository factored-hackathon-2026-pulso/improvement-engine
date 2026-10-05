//! RealCore readiness: the `CorePort` over core-client (`LiveCore`) is selectable by config and fails closed. Nothing here starts
//! a container or reaches a real Core: the bridge is the existing core-client FakeCore / GoldenCore (a transcript generated from
//! `bridge-contract/examples/flows`), so the requests LiveCore builds are compared with the golden ones.
#[path = "../../core-client/tests/common/mod.rs"]
mod common;
use common::golden::GoldenCore;
use common::{FakeCore, KID, SEED};
use core_client::authorizer::LocalSimAuthorizer;
use core_client::registry::RegistryClient;
use core_client::{ClientConfig, CoreClient};
use engine::live::CorePort;
use engine::live_core::{E2eFixtures, LiveCore, LiveCoreConfig, LiveWorld};
use engine::real_core::core_from_env;
use std::collections::HashMap;
use std::time::Duration;

fn live(addr: &str) -> LiveCore {
    let mut cfg = ClientConfig::new(addr, KID, SEED, "bridge-1");
    cfg.timeout = Duration::from_secs(5);
    cfg.accept_golden_placeholders = true;
    LiveCore::new(
        CoreClient::new(cfg),
        RegistryClient::new("127.0.0.1:1", Duration::from_secs(1)),
        LocalSimAuthorizer::new("human-kid", [9u8; 32], "t1", "local-supervisor"),
        Box::new(E2eFixtures { addr: "127.0.0.1:1".into(), tenant: "t1".into() }),
        LiveCoreConfig { tenant: "t1".into(), agent_id: "atencion".into(), writer_release_id: "rel-w".into(), budget_ref: "bud".into(), world: LiveWorld::default() },
    )
}

#[test]
fn only_the_live_port_says_it_is_real() {
    assert!(live("127.0.0.1:1").is_real());
}

#[test]
fn readiness_replays_the_golden_version_probe_and_prod_alias_read() {
    let g = GoldenCore::play(&[("auth_and_envelope", "version_ok"), ("authoring", "alias_read")]);
    let r = live(&g.addr).readiness("job-golden").expect("readiness");
    g.finish();
    assert_eq!(r.agent_core_sha, core_client::pins::AGENT_CORE_SHA);
    assert_eq!(r.prod_release_id.as_deref(), Some("rel-demo"));
}

#[test]
fn readiness_fails_closed_when_the_bridge_does_not_report_our_pin() {
    let f = FakeCore::start(); // its version body lacks the fields of a real bridge: a decode error, never a pass
    let e = live(&f.addr).readiness("job-r").unwrap_err();
    assert!(e.contains("version"), "{e}");
    let dead = live("127.0.0.1:1").readiness("job-r").unwrap_err();
    assert!(dead.contains("version"), "{dead}");
}

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |k| m.get(k).cloned()
}

const SEED_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";

fn full() -> Vec<(&'static str, String)> {
    vec![
        ("PULSO_CORE_PORT", "live".into()),
        ("PULSO_BRIDGE_ADDR", "127.0.0.1:1".into()),
        ("PULSO_SERVICE_KID", "k1".into()),
        ("PULSO_SERVICE_SEED_HEX", SEED_HEX.into()),
        ("PULSO_CORE_ADDR", "127.0.0.1:2".into()),
        ("PULSO_HUMAN_KID", "h1".into()),
        ("PULSO_HUMAN_SEED_HEX", SEED_HEX.into()),
        ("PULSO_LIVE_TENANT", "t1".into()),
        ("PULSO_LIVE_WRITER_RELEASE", "rel-w".into()),
        ("PULSO_E2E_FX_ADDR", "127.0.0.1:3".into()),
        ("PULSO_LIVE_WORLD", concat!(env!("CARGO_MANIFEST_DIR"), "/../core-client/tests/fixtures/live_attention_task.json").into()),
    ]
}

fn pairs<'a>(v: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    v.iter().map(|(k, x)| (*k, x.as_str())).collect()
}

#[test]
fn the_offline_double_is_the_default_and_live_is_only_selected_by_config() {
    assert!(core_from_env(&env(&[])).unwrap().is_none(), "no config: the offline double");
    assert!(core_from_env(&env(&[("PULSO_CORE_PORT", "double")])).unwrap().is_none());
    assert!(core_from_env(&env(&[("PULSO_CORE_PORT", "maybe")])).is_err());
    let p = core_from_env(&env(&pairs(&full()))).unwrap().expect("live port");
    assert!(p.is_real(), "constructing it reached nothing: every address above is closed");
}

#[test]
fn a_live_selection_with_missing_or_malformed_values_names_the_variable_and_never_echoes_a_secret() {
    let mut f = full();
    f.retain(|(k, _)| *k != "PULSO_LIVE_TENANT" && *k != "PULSO_E2E_FX_ADDR");
    let e = core_from_env(&env(&pairs(&f))).err().unwrap();
    assert!(e.contains("PULSO_LIVE_TENANT") && e.contains("PULSO_E2E_FX_ADDR"), "{e}");
    let mut f = full();
    f.iter_mut().find(|(k, _)| *k == "PULSO_SERVICE_SEED_HEX").unwrap().1 = "zz-not-hex-secret-material".into();
    let e = core_from_env(&env(&pairs(&f))).err().unwrap();
    assert!(e.contains("PULSO_SERVICE_SEED_HEX") && !e.contains("secret-material"), "{e}");
}
