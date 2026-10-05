//! LIVE (opt-in): the engine's minted credentials against a real `agentcore serve --registry-api`.
//! Run: `PULSO_LIVE_SERVE_ADDR=127.0.0.1:<port> PULSO_SERVICE_KID=<kid listed in the Core's staff-keys> PULSO_SERVICE_SEED_HEX=<64 hex>
//!       [PULSO_LIVE_AGENT=consultas] [PULSO_LIVE_SPAN_S=150] cargo test -j 1 -p pulso --test live_serve_credentials -- --ignored --nocapture`
//! The kid must already be in the Core's staff-keys file (the Core re-reads it within seconds, no restart). Names only are printed.
use core_client::authorizer::Jws;
use pulso::run::registry_auth::RegistryAuth;
use registry_writer::{HttpTransport, Reply, Request, Transport, TransportError};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

/// Remembers a short fingerprint of each distinct credential that went out.
struct Spy {
    inner: Arc<dyn Transport + Send + Sync>,
    seen: Mutex<Vec<String>>,
}
impl Transport for Spy {
    fn send(&self, req: &Request) -> Result<Reply, TransportError> {
        let f = req.bearer.fingerprint();
        let mut s = self.seen.lock().unwrap();
        if !s.contains(&f) {
            s.push(f);
        }
        drop(s);
        self.inner.send(req)
    }
}

fn rig() -> (Arc<Spy>, Arc<dyn Transport + Send + Sync>, String) {
    let addr = env("PULSO_LIVE_SERVE_ADDR").expect("PULSO_LIVE_SERVE_ADDR");
    let auth = RegistryAuth::from_lookup(&|k| env(k).filter(|_| k != "PULSO_REGISTRY_TOKEN")).expect("PULSO_SERVICE_SEED_HEX + PULSO_SERVICE_KID");
    assert!(auth.is_minted());
    let raw: Arc<dyn Transport + Send + Sync> = Arc::new(HttpTransport::new(&addr, Duration::from_secs(20)));
    // the Spy sits BELOW the minting transport: it sees exactly what goes on the wire
    let spy = Arc::new(Spy { inner: raw.clone(), seen: Mutex::new(vec![]) });
    let minted = auth.wrap(spy.clone());
    (spy, minted, addr)
}

fn get(t: &dyn Transport, path: &str) -> Reply {
    t.send(&Request { method: "GET", path: path.into(), bearer: &Jws::new(String::new()), idempotency_key: None, body: None }).expect("the Core answers")
}

#[test]
#[ignore = "live: needs PULSO_LIVE_SERVE_ADDR, PULSO_SERVICE_KID (in the Core's staff-keys) and PULSO_SERVICE_SEED_HEX"]
fn serve_accepts_the_minted_credential_and_refuses_no_credential_and_a_forged_one() {
    let (_, minted, addr) = rig();
    let r = get(&*minted, "/v1/registry/proposals?limit=1");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body["items"].is_array() || r.body["total"].is_number(), "a proposal page: {}", r.body);
    // no credential, and one signed by another key (same kid): refused
    let raw = HttpTransport::new(&addr, Duration::from_secs(20));
    let none = raw.send(&Request { method: "GET", path: "/v1/registry/proposals?limit=1".into(), bearer: &Jws::new(String::new()), idempotency_key: None, body: None }).unwrap();
    assert_eq!(none.status, 401);
    let forged = core_client::service_identity::ServiceIdentity::new(&env("PULSO_SERVICE_KID").unwrap(), [1u8; 32]).credential();
    let bad = raw.send(&Request { method: "GET", path: "/v1/registry/proposals?limit=1".into(), bearer: &forged, idempotency_key: None, body: None }).unwrap();
    assert_eq!(bad.status, 401, "a credential signed by another key must not verify");
    println!("live: minted 200, none 401, forged 401");
}

#[test]
#[ignore = "live: opens one auto_detect proposal on the local Core"]
fn a_minted_builder_opens_a_proposal_as_pulso_engine_and_cannot_approve() {
    let (_, minted, _) = rig();
    let agent = env("PULSO_LIVE_AGENT").unwrap_or_else(|| "consultas".into());
    let body = json!({"agent_id": agent, "origin": "auto_detect", "title": "[improvement-engine] live minted credential check"});
    let r = minted.send(&Request { method: "POST", path: "/v1/registry/proposals".into(), bearer: &Jws::new(String::new()), idempotency_key: Some("engprod-live-1"), body: Some(body) }).unwrap();
    assert_eq!(r.status, 201, "{}", r.body);
    assert_eq!(r.body["created_by"], "pulso-engine");
    let pid = r.body["proposal_id"].as_str().expect("proposal_id").to_string();
    // the Core itself refuses an approval by this principal (constructor only), whatever the engine's allow-list says
    let a = minted.send(&Request { method: "POST", path: format!("/v1/registry/proposals/{pid}/approve"), bearer: &Jws::new(String::new()), idempotency_key: None, body: Some(json!({"candidate_hash": "0".repeat(64)})) }).unwrap();
    assert!(matches!(a.status, 403 | 409 | 422), "approve was {}: {}", a.status, a.body);
    assert_ne!(a.status, 200);
    println!("live: proposal created_by pulso-engine; approve answered {}", a.status);
}

#[test]
#[ignore = "live: runs for PULSO_LIVE_SPAN_S seconds (default 150) with a 60 s credential lifetime"]
fn a_long_job_keeps_working_across_credential_expiries() {
    // TTL 60 s (the minimum): over 150 s the Core sees at least three different credentials and refuses none.
    let mut a = std::env::vars().collect::<std::collections::HashMap<_, _>>();
    a.retain(|k, _| k != "PULSO_REGISTRY_TOKEN");
    a.insert("PULSO_SERVICE_CRED_TTL_S".into(), "60".into());
    let addr = env("PULSO_LIVE_SERVE_ADDR").expect("PULSO_LIVE_SERVE_ADDR");
    let auth = RegistryAuth::from_lookup(&|k| a.get(k).cloned()).unwrap();
    let spy = Arc::new(Spy { inner: Arc::new(HttpTransport::new(&addr, Duration::from_secs(20))), seen: Mutex::new(vec![]) });
    let minted = auth.wrap(spy.clone());
    let span = Duration::from_secs(env("PULSO_LIVE_SPAN_S").and_then(|v| v.parse().ok()).unwrap_or(150));
    let (t0, mut statuses) = (Instant::now(), vec![]);
    while t0.elapsed() < span {
        statuses.push(get(&*minted, "/v1/registry/proposals?limit=1").status);
        std::thread::sleep(Duration::from_secs(10));
    }
    let n = spy.seen.lock().unwrap().len();
    println!("live: {} requests over {} s, {} distinct credentials, statuses {:?}", statuses.len(), t0.elapsed().as_secs(), n, statuses.iter().collect::<std::collections::BTreeSet<_>>());
    assert!(statuses.iter().all(|s| *s == 200), "every request must be accepted: {statuses:?}");
    assert!(n >= 3, "a 60 s credential over {} s must have been replaced at least twice, saw {n}", span.as_secs());
}
