//! LIVE tests against the LOCAL agent-core stack (`python scripts/dev-stack/stack.py up`, our own instance, NOT the shared Core).
//! Opt-in: every test is `#[ignore]`d. Run with
//!
//! ```text
//! PULSO_DEV_STACK_DIR=<worktree>/.dev-stack [PULSO_STACK_ADDR=127.0.0.1:8001] \
//!   cargo test -j 1 -p registry-writer --test live -- --ignored --nocapture --test-threads 1
//! ```
//!
//! The tokens are read from `<PULSO_DEV_STACK_DIR>/tokens.json` and never printed. The tests create real DRAFT proposals in the
//! local registry (quota: 10 per 24 h) and never approve, publish or promote. Only the label of the credential used is printed.
use core_client::authorizer::Jws;
use registry_writer::{Config, Environment, FileStore, HttpTransport, Reason, Submission, Via, Writer};
use serde_json::{Value, json};
use std::time::Duration;

fn tokens() -> (Jws, Jws, String) {
    let dir = std::env::var("PULSO_DEV_STACK_DIR").expect("set PULSO_DEV_STACK_DIR to the .dev-stack folder of the running local stack");
    let raw = std::fs::read_to_string(std::path::Path::new(&dir).join("tokens.json")).expect("tokens.json (run stack.py up)");
    let v: Value = serde_json::from_str(&raw).unwrap();
    let get = |k: &str| Jws::new(v[k].as_str().unwrap_or_else(|| panic!("tokens.json has no {k}")).to_string());
    let addr = std::env::var("PULSO_STACK_ADDR").unwrap_or_else(|_| "127.0.0.1:8001".into());
    (get("builder"), get("admin"), addr)
}

/// The registry API of the local stack verifies only staff keys: the engine-signed builder token is accepted there only when the
/// engine kid is in staff-keys (`identity.py keys` does that now). Otherwise the staff admin credential is the fallback.
fn registry_token(t: &HttpTransport, builder: &Jws, admin: &Jws) -> (Jws, &'static str) {
    let probe = registry_writer::MemoryStore::new();
    let w = Writer::new(Config::new(Via::RegistryApi, Environment::LocalStack, builder.clone()), t, &probe);
    match w.fetch_artifact("prompt:p/pulso-builder") {
        Ok(_) => (builder.clone(), "engine builder principal"),
        Err((Reason::Unauthorized, _)) => (admin.clone(), "local staff admin credential (stand-in: engine kid not in this stack's staff keys)"),
        Err(f) => panic!("the local stack is not reachable or has no pulso-builder prompt: {f:?}"),
    }
}

fn store_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pulso-b2-live-{name}-{}.json", std::process::id()))
}

fn unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// A patch of the live `p/pulso-builder` prompt: one sentence appended to the live text, version bumped.
fn live_patch(w: &Writer, tag: &str) -> Submission {
    let live = w.fetch_artifact("prompt:p/pulso-builder").expect("live prompt");
    let mut locales = live.locales.clone();
    for t in locales.values_mut() {
        t.push_str(" Every rationale names the signal id of the evidence.");
    }
    let parts: Vec<u64> = live.version.split('.').filter_map(|x| x.parse().ok()).collect();
    let next = format!("{}.{}.{}", parts[0], parts[1], parts[2] + 1);
    let mut content = json!({"id": live.id, "version": next, "locales": locales});
    content.as_object_mut().unwrap().extend(live.extra.as_object().unwrap().clone());
    Submission {
        finding_id: "finding_live".into(),
        evidence_ref: format!("ev_live_{tag}"),
        agent_id: "pulso-builder".into(),
        target_ref: "prompt:p/pulso-builder".into(),
        kind: "patch".into(),
        base_digest: live.digest(),
        changes: vec![json!({"kind": "prompt", "content": content, "docs": {"description": "[improvement-engine] live B2 test: name the signal id in every rationale", "rationale": "synthetic evidence: reviewers rejected drafts without a signal id"}})],
        rationale: "synthetic: reviewers rejected 4 of 5 drafts because the rationale did not name the signal id".into(),
    }
}

#[test]
#[ignore = "live: needs the local agent-core stack and PULSO_DEV_STACK_DIR"]
fn live_registry_api_delivers_a_draft_and_a_retry_is_idempotent() {
    let (builder, admin, addr) = tokens();
    let t = HttpTransport::new(&addr, Duration::from_secs(60));
    let (reg, who) = registry_token(&t, &builder, &admin);
    println!("registry credential: {who}");
    let store = FileStore::new(store_path("api"));
    let mut cfg = Config::new(Via::RegistryApi, Environment::LocalStack, reg);
    cfg.credential = who;
    cfg.run_token = Some(builder);
    let w = Writer::new(cfg, &t, &store);
    let s = live_patch(&w, &unix().to_string());

    let first = w.deliver(&s);
    println!("first: {}", first.to_json());
    assert!(first.delivered(), "{}", first.to_json());
    assert!(first.proposal_id.is_some() && first.valid == Some(true) && first.changes >= 1);
    assert_eq!(first.state.as_deref(), Some("draft"), "the engine leaves the proposal as a draft: agent-core manages the rest");

    let again = w.deliver(&s);
    println!("retry: {}", again.to_json());
    assert!(again.delivered() && again.replayed, "{}", again.to_json());
    assert_eq!(again.proposal_id, first.proposal_id, "idempotent per finding key");

    // the live base moved on (as if someone else published): refused before any write, no quota spent
    let mut stale = live_patch(&w, &format!("{}-stale", unix()));
    stale.base_digest = "0000000000000000".into();
    let o = w.deliver(&stale);
    assert_eq!(o.reason, Some(Reason::BaseChanged), "{}", o.to_json());
    assert!(o.proposal_id.is_none());
}

#[test]
#[ignore = "live: needs the local agent-core stack, a model key behind its gateway, and PULSO_DEV_STACK_DIR"]
fn live_builder_run_delivers_a_draft_authored_by_the_agent() {
    let (builder, admin, addr) = tokens();
    let t = HttpTransport::new(&addr, Duration::from_secs(240));
    let (reg, who) = registry_token(&t, &builder, &admin);
    println!("registry credential: {who}");
    let store = FileStore::new(store_path("run"));
    let mut cfg = Config::new(Via::BuilderRun, Environment::LocalStack, reg);
    cfg.credential = who;
    cfg.run_token = Some(builder);
    let w = Writer::new(cfg, &t, &store);
    let mut outcomes = vec![];
    for attempt in 0..2 {
        let s = live_patch(&w, &format!("{}-run{attempt}", unix()));
        let o = w.deliver(&s);
        println!("attempt {attempt}: {}", o.to_json());
        outcomes.push(o.reason.map(Reason::code).unwrap_or("delivered"));
        if o.delivered() {
            assert_eq!(o.valid, Some(true));
            assert!(o.changes >= 1);
            return;
        }
    }
    panic!("no attempt produced a valid non-empty draft: {outcomes:?}");
}
