//! E2 LIVE: the 5-step `thread` job through the executor against the REAL Core image, with the live handlers
//! (dry-run hook, arms via `run_arm`, native evaluation, authority decision, publish to staging with alias readback).
//! IGNORED by default. ONE run per fresh stack (a publish consumes the immutable prompt/suite 2.0.0 of the seeded world):
//!
//!   pwsh e2e-core\run.ps1 -BaseImage localhost/pulso-core-runtime:c814c2b-920f5e3 -Namespace <ns> -Keep -PytestArgs "-k","nothing_selected"
//!   . seams\scripts\live-env.ps1 -Namespace <ns>
//!   $env:CARGO_TARGET_DIR='D:/cargo-targets/claude-seams-live'; $env:PULSO_LIVE_EVIDENCE_DIR="$PWD\docs\reports\w4a-live"
//!   cargo test --offline -j 2 --manifest-path seams/Cargo.toml -p engine --test live_thread -- --ignored --nocapture --test-threads 1
//!   local\core\stop.ps1 -Namespace <ns>; local\core\reset.ps1 -Namespace <ns> -Confirm        (teardown is mandatory)
//!
//! Two jobs, in this order: (1) WITHOUT a human override: the stand-in gate does not pass on the live world (base and
//! candidate complete the same cases), so the job stops at `authority` with `blocked(gate)` and nothing is approved or
//! published; (2) WITH an explicit, labelled SIMULATED human override (DEMO-0 / authority crate rule): the job completes,
//! publishes to staging and the alias readback equals the published release. Prod never moves.
//! Labels: arms and native evaluation are real Core; judge = claude-standin; human = claude-standin (`simulated=true`);
//! the platform double seals artifacts and issues bindings (BRG1 gap candidate); oracle = the suite's own `expect` blocks.
#[path = "../../core-client/tests/live_common/mod.rs"]
mod live_common;
use authority::Override;
use core_client::authorizer::LocalSimAuthorizer;
use core_client::authoring::Alias;
use core_client::registry::RegistryClient;
use engine::adapters::thread_handlers;
use engine::executor::{ExecError, ExecOptions, execute};
use engine::live::{self, CorePort, LiveConfig};
use engine::live_core::{LiveCore, LiveCoreConfig, LiveWorld};
use engine::{FileStore, event_log, synth};
use live_common::*;
use serde_json::{Value, json};
use std::rc::Rc;
use std::time::{Duration, Instant};

fn core() -> Rc<dyn CorePort> {
    let world = LiveWorld::from_fixture(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../core-client/tests/fixtures/live_attention_task.json")).unwrap()).unwrap();
    let tenant = tenant();
    Rc::new(LiveCore::new(
        client("e2-live"),
        RegistryClient::new(&env("PULSO_CORE_ADDR"), Duration::from_secs(60)),
        LocalSimAuthorizer::new(&env("PULSO_HUMAN_KID"), seed("PULSO_HUMAN_SEED_HEX"), &tenant, "local-supervisor"),
        Box::new(Fx::from_env()),
        LiveCoreConfig { tenant, agent_id: agent(), writer_release_id: env("PULSO_LIVE_WRITER_RELEASE"), budget_ref: "bud-e2e".into(), world },
    ))
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("e2live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn run_job(job: &str, over: Option<Override>) -> (Result<String, ExecError>, Vec<String>, f64) {
    let work = tmp(job);
    let (env, spec) = synth::build(&work, std::path::Path::new(env!("CARGO_BIN_EXE_synth_runner")), None).unwrap();
    let store = FileStore::open(work.join("store")).unwrap();
    let port = core();
    let cfg = LiveConfig { human_actor: "local-supervisor".into(), human_override: over, decision_ttl_seconds: 600 };
    let hs = live::live_handlers(thread_handlers(env, Some(live::dry_run_hook(port.clone()))), port, cfg);
    let mut o = ExecOptions::new(job, "w1", 0);
    o.now = Box::new(now);
    o.lease_seconds = 900;
    let t = Instant::now();
    let r = execute(&store, &hs, &spec, &o);
    let secs = (t.elapsed().as_secs_f64() * 100.0).round() / 100.0;
    let n = hs.len();
    (r, event_log(&store, n).unwrap(), secs)
}

#[test]
#[ignore = "needs a fresh kept e2e stack; see module docs"]
fn live_thread_runs_the_five_step_job_end_to_end_against_the_real_image() {
    let c = client("e2-live-check");
    let (fx, ten) = (Fx::from_env(), tenant());
    fx.script_closing_reply().expect("scripted gateway double");
    c.version().expect("version").check_pin().expect("pin");
    let prod0 = c.read_alias(&ten, None, &agent(), Alias::Prod).unwrap().release_id.unwrap();
    let staging0 = c.read_alias(&ten, None, &agent(), Alias::Staging).unwrap().release_id.unwrap();
    assert_eq!(prod0, staging0, "run on a FRESH stack");

    // (1) no override: blocked at the authority decision, nothing approved or published
    let (r1, ev1, s1) = run_job("thread-live-blocked", None);
    match &r1 {
        Err(ExecError::Handler(abi::HandlerError::Failed(m))) => assert!(m.contains("blocked(gate)"), "{m}"),
        other => panic!("expected blocked(gate), got {other:?}"),
    }
    assert!(ev1.len() == 7 && ev1[4].starts_with("thread:arms:runs=6,completed=6:") && ev1[6].starts_with("thread:native_eval:verdict=pass"), "{ev1:#?}");
    assert_eq!(c.read_alias(&ten, None, &agent(), Alias::Staging).unwrap().release_id.unwrap(), staging0, "nothing was published");

    // (2) explicit labelled simulated human override: completes and publishes
    let over = Override { by: "human".into(), actor: "local-supervisor".into(), reason: "exercise the Core approve/publish mechanics although the structural gate did not pass; no quality claim".into() };
    let (r2, ev2, s2) = run_job("thread-live-override", Some(over));
    let fin = r2.expect("the job completes with the override");
    assert_eq!(ev2.len(), 9, "{ev2:#?}");
    assert!(ev2[7].contains("override=human_override") && ev2[7].contains("simulated=true"), "{}", ev2[7]);
    assert!(ev2[8].contains("alias=staging,readback=equal") && ev2[8].contains("simulated_human=true"), "{}", ev2[8]);
    let out: Value = serde_json::from_str(&fin).unwrap();
    let release = out["out"]["publish"]["release_id"].as_str().unwrap().to_string();
    assert_ne!(release, prod0);
    assert_eq!(c.read_alias(&ten, None, &agent(), Alias::Staging).unwrap().release_id.unwrap(), release, "staging readback equals the publish");
    assert_eq!(c.read_alias(&ten, None, &agent(), Alias::Prod).unwrap().release_id.unwrap(), prod0, "prod is untouched");
    let gate = out["out"]["gate"]["verdict"].as_str().unwrap().to_string();
    assert_ne!(gate, "pass", "the stand-in gate does not pass on the live world (honest): the override is what publishes");
    assert_eq!(out["out"]["compile"]["draft_plan"]["digest"].as_str().unwrap(), format!("sha256:{}", out["out"]["arms"]["frozen"]["candidate_hash"].as_str().unwrap()), "dry-run digest = frozen candidate");

    evidence(
        "live_thread",
        &json!({"image": env("PULSO_LIVE_STACK_IMAGE"), "namespace": env("PULSO_LIVE_NAMESPACE"),
            "job_blocked_without_override": {"events": ev1, "seconds": s1, "result": "blocked(gate)"},
            "job_with_simulated_human_override": {"events": ev2, "seconds": s2, "release_id": release, "gate_verdict": gate,
                "arm_runs": out["out"]["arms"]["runs"], "override": out["out"]["authority"]["override"]},
            "labels": {"human": "claude-standin (simulated=true)", "judge": "claude-standin", "oracle": "suite expect blocks", "platform": "e2e fixtures double (BRG1 gap candidate)", "quality_claims": "forbidden"}}),
    );
}
