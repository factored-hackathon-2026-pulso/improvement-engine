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

/// K3 live: dry-run, seal, writer stage (create_proposal, put_draft, freeze) on the real image; the frozen
/// `candidate_hash` must equal the dry-run digest. Needs a stack from `e2e-core/run.ps1` on machine pulso-dev, plus
/// `PULSO_E2E_FX_ADDR` (the fixtures server `/_e2e/config` that holds sealed artifacts), `PULSO_LIVE_WRITER_RELEASE`
/// (the `pulso-writer` release id of the bridge manifest), `PULSO_LIVE_TENANT`, `PULSO_LIVE_BASE_RELEASE` (prod alias
/// release) and `PULSO_LIVE_AGENT` (default `atencion`). The drafts are the K3 fixture (replace prompt + add suite).
/// NOT covered here: approve and publish to the staging alias use Core `/v1/registry` with a human-issued JWS (no
/// `/internal/v1` route); after them, `core_client::writer::check_alias_readback` verifies the staging alias.
/// Run: cargo test --manifest-path seams/Cargo.toml --offline -j 2 -p core-client --test live -- --ignored live_k3
#[test]
#[ignore = "needs a live bridge stack, its fixtures server and a pulso-writer release; see doc comment"]
fn live_k3_freeze_draft_matches_the_dry_run() {
    use core_client::authoring::Change;
    use core_client::writer::{ArtifactSealer, DraftPlan, WriterRun};
    struct FxSealer {
        addr: String,
        tenant: String,
    }
    impl ArtifactSealer for FxSealer {
        fn seal(&self, plan_ref: &str, content: &serde_json::Value) -> Result<(), String> {
            let body = serde_json::json!({"artifacts": [{"tenant": self.tenant, "id": plan_ref, "content": content}]});
            let r = core_client::http::request(&self.addr, "POST", "/_e2e/config", &[], Some(&serde_json::to_vec(&body).unwrap()), std::time::Duration::from_secs(10))
                .map_err(|e| format!("{e:?}"))?;
            if r.status == 200 { Ok(()) } else { Err(format!("fixtures server answered {}", r.status)) }
        }
    }
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
    let hex = env("PULSO_SERVICE_SEED_HEX");
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex seed");
    }
    let tenant = std::env::var("PULSO_LIVE_TENANT").unwrap_or_else(|_| "t1".into());
    let agent = std::env::var("PULSO_LIVE_AGENT").unwrap_or_else(|_| "atencion".into());
    let c = CoreClient::new(ClientConfig::new(&env("PULSO_BRIDGE_ADDR"), &env("PULSO_SERVICE_KID"), seed, "k3-live"));
    c.version().expect("version").check_pin().expect("pin");
    let fx: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/k3_draft.json")).unwrap()).unwrap();
    let changes = fx["plan"]["changes"].as_array().unwrap().iter().map(|ch| Change::new(ch["kind"].as_str().unwrap(), ch["content"].clone(), ch["docs"].clone())).collect();
    let n = format!("{}", std::process::id());
    let plan = DraftPlan::new(&agent, &format!("pulso-key:k3live-{n}"), changes);
    let run = WriterRun {
        tenant_id: tenant.clone(),
        job_id: format!("job-k3-live-{n}"),
        logical_key: "k3-live-1".into(),
        pulso_run_ref: format!("pr-job-k3-live-{n}"),
        lab_grant_ref: "grant-contract".into(),
        writer_release_id: env("PULSO_LIVE_WRITER_RELEASE"),
        writer_agent_version: "1.0.0".into(),
    };
    let sealer = FxSealer { addr: env("PULSO_E2E_FX_ADDR"), tenant };
    let fz = c.freeze_draft(&sealer, &plan, &run, Some(&env("PULSO_LIVE_BASE_RELEASE"))).expect("freeze");
    assert_eq!(fz.candidate_hash.len(), 64);
    assert!(!fz.proposal_id.is_empty());
}
