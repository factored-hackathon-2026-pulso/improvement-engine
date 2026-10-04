//! K3 registry client against a std-TcpListener FakeRegistry. The fake mirrors the request/response shapes of the
//! Python `FakeRegistry` (`e2e-core/tests/unit/test_core_hooks_real.py`) and the pinned agent-core `registry/http.py`:
//! `GET /proposals/{id}` -> `{proposal:{rev,state,candidate_hash}}`; approve -> `409 candidate_changed` for a hash that
//! is not the candidate, `409 illegal_transition` out of state; publish needs `Idempotency-Key`; alias -> AliasState.
//! Everything signed here is the CLAUDE-STANDIN human (`LocalSimAuthorizer`, simulated issuer).
use core_client::authorizer::{Authorizer, Jws, LocalSimAuthorizer, ProposalTarget};
use core_client::registry::{Refusal, RegistryClient, RegistryError, RegistryFlow};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CAND: &str = "abababababababababababababababababababababababababababababababab";
const BASE: &str = "rel-base";
const BOT: &str = "bot.jws.x";
const SEED: [u8; 32] = [0x33; 32];

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    auth: String,
    idem: Option<String>,
    body: Value,
}

struct St {
    state: &'static str,
    staging: String,
    publish_returns_base: bool,
    log: Vec<Req>,
}

struct FakeRegistry {
    addr: String,
    st: Arc<Mutex<St>>,
    stop: Arc<AtomicBool>,
}

impl Drop for FakeRegistry {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.addr);
    }
}

fn reply(s: &mut TcpStream, status: u16, body: Value) {
    let b = body.to_string();
    let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len());
}

fn read_req(s: &mut TcpStream) -> Option<Req> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let (head_end, len) = loop {
        let n = s.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buf[..n]);
        if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..p]).to_lowercase();
            let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0usize);
            break (p + 4, len);
        }
    };
    while raw.len() < head_end + len {
        let n = s.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
    }
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let hv = |n: &str| lines.clone().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string()));
    let auth = hv("authorization").unwrap_or_default().trim_start_matches("Bearer ").to_string();
    let body = serde_json::from_slice(&raw[head_end..]).unwrap_or(Value::Null);
    Some(Req { method, path, auth, idem: hv("idempotency-key"), body })
}

impl FakeRegistry {
    fn start(state: &'static str, publish_returns_base: bool) -> FakeRegistry {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let st = Arc::new(Mutex::new(St { state, staging: BASE.into(), publish_returns_base, log: vec![] }));
        let stop = Arc::new(AtomicBool::new(false));
        let (st2, stop2) = (st.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in l.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut s) = conn else { continue };
                let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                let Some(req) = read_req(&mut s) else { continue };
                let mut g = st2.lock().unwrap();
                g.log.push(req.clone());
                let (status, body) = route(&mut g, &req);
                drop(g);
                reply(&mut s, status, body);
            }
        });
        FakeRegistry { addr, st, stop }
    }
    fn log(&self) -> Vec<Req> {
        self.st.lock().unwrap().log.clone()
    }
}

fn route(g: &mut St, r: &Req) -> (u16, Value) {
    let p = r.path.strip_prefix("/v1/registry").unwrap_or("");
    let write = r.method == "POST";
    if write && r.auth == BOT {
        return (403, json!({"code": "role_required"}));
    }
    match (r.method.as_str(), p) {
        ("GET", "/proposals/prop-1") => (200, json!({"proposal": {"rev": 3, "state": g.state, "candidate_hash": CAND}})),
        ("POST", "/proposals/prop-1/approve") => {
            if r.body["candidate_hash"] != CAND {
                return (409, json!({"code": "candidate_changed"}));
            }
            if g.state != "evaluated" {
                return (409, json!({"code": "illegal_transition"}));
            }
            g.state = "approved";
            (200, json!({"actor": "local-supervisor", "decision": "approved", "candidate_hash": CAND}))
        }
        ("POST", "/proposals/prop-1/publish") => {
            if r.idem.is_none() {
                return (422, json!({"code": "invalid_request"}));
            }
            if g.state != "approved" {
                return (409, json!({"code": "illegal_transition"}));
            }
            if g.publish_returns_base {
                return (200, json!({"release_id": BASE}));
            }
            g.staging = "rel-new".into();
            g.state = "published";
            (200, json!({"release_id": "rel-new"}))
        }
        ("GET", a) if a.starts_with("/aliases/atencion/") => {
            let alias = &a["/aliases/atencion/".len()..];
            let rel = if alias == "staging" { g.staging.clone() } else { BASE.into() };
            (200, json!({"agent_id": "atencion", "alias": alias, "release_id": rel, "status": "active"}))
        }
        ("GET", a) if a.starts_with("/releases/") => (200, json!({"release_id": &a["/releases/".len()..], "status": "active", "agent_id": "atencion"})),
        _ => (404, json!({"code": "not_found"})),
    }
}

fn auth() -> LocalSimAuthorizer {
    LocalSimAuthorizer::new("local-sim-human-1", SEED, "t1", "local-supervisor")
}
fn target(hash: &str) -> ProposalTarget {
    ProposalTarget { proposal_id: "prop-1".into(), candidate_hash: hash.into(), expected_revision: 3 }
}
fn client(f: &FakeRegistry) -> RegistryClient {
    RegistryClient::new(&f.addr, Duration::from_secs(5))
}
fn bot() -> Jws {
    Jws::new(BOT.into())
}
fn writes(f: &FakeRegistry) -> usize {
    f.log().iter().filter(|r| r.method == "POST").count()
}

#[test]
fn approve_with_a_wrong_candidate_hash_is_refused_by_the_client_and_never_sent() {
    let f = FakeRegistry::start("evaluated", false);
    let bad = "0".repeat(64);
    let c = client(&f);
    // body hash is not the frozen candidate
    let jws = auth().authorize("approve", &target(CAND)).unwrap();
    assert_eq!(c.approve(&target(CAND), &bad, &jws), Err(RegistryError::Refused(Refusal::CandidateHashMismatch)));
    // a JWS authorised for another hash than the candidate (body hash honest)
    let jws_bad = auth().authorize("approve", &target(&bad)).unwrap();
    assert_eq!(c.approve(&target(CAND), CAND, &jws_bad), Err(RegistryError::Refused(Refusal::JwsBindingMismatch("candidate_hash"))));
    assert_eq!(writes(&f), 0, "nothing reached Core");
}

#[test]
fn server_side_semantic_a_wrong_hash_is_refused_by_core_with_candidate_changed() {
    let f = FakeRegistry::start("evaluated", false);
    let bad = "0".repeat(64);
    let jws = auth().authorize("approve", &target(&bad)).unwrap();
    let out = client(&f).probe_approve("prop-1", &bad, &jws).unwrap();
    assert_eq!((out.status, out.code.as_deref()), (409, Some("candidate_changed")));
}

#[test]
fn a_jws_for_another_operation_proposal_or_revision_is_refused_locally() {
    let f = FakeRegistry::start("evaluated", false);
    let c = client(&f);
    let publish_jws = auth().authorize("publish", &target(CAND)).unwrap();
    assert_eq!(c.approve(&target(CAND), CAND, &publish_jws), Err(RegistryError::Refused(Refusal::JwsBindingMismatch("operation"))));
    let other = ProposalTarget { proposal_id: "prop-2".into(), ..target(CAND) };
    let j = auth().authorize("approve", &other).unwrap();
    assert_eq!(c.approve(&target(CAND), CAND, &j), Err(RegistryError::Refused(Refusal::JwsBindingMismatch("proposal_id"))));
    let stale = ProposalTarget { expected_revision: 2, ..target(CAND) };
    let j = auth().authorize("approve", &stale).unwrap();
    assert_eq!(c.approve(&target(CAND), CAND, &j), Err(RegistryError::Refused(Refusal::JwsBindingMismatch("expected_revision"))));
    assert_eq!(writes(&f), 0);
}

#[test]
fn replay_of_a_used_jws_is_refused_by_the_client_and_by_core() {
    let f = FakeRegistry::start("evaluated", false);
    let c = client(&f);
    let jws = auth().authorize("approve", &target(CAND)).unwrap();
    assert_eq!(c.approve(&target(CAND), CAND, &jws).unwrap().decision, "approved");
    assert_eq!(c.approve(&target(CAND), CAND, &jws), Err(RegistryError::Refused(Refusal::ReplayedJws)));
    assert_eq!(writes(&f), 1, "the replay was not sent");
    let out = c.probe_approve("prop-1", CAND, &jws).unwrap(); // server-side semantic of the same replay
    assert_eq!((out.status, out.code.as_deref()), (409, Some("illegal_transition")));
}

#[test]
fn publish_that_returns_the_base_release_is_an_error_not_a_publication() {
    let f = FakeRegistry::start("evaluated", true);
    let (c, a) = (client(&f), auth());
    let mut flow = RegistryFlow::new(&c, &a, bot(), "atencion", "prop-1", CAND, Some(BASE.into()));
    flow.approve().unwrap();
    let e = flow.publish("pub-1").unwrap_err();
    assert!(matches!(&e, RegistryError::Invariant(m) if m.contains("base release")), "{e:?}");
    assert!(matches!(flow.alias_read("staging"), Err(RegistryError::Flow(_))), "a refused publish does not unlock staging");
}

#[test]
fn alias_read_of_staging_before_publish_is_refused_without_calling_core() {
    let f = FakeRegistry::start("evaluated", false);
    let (c, a) = (client(&f), auth());
    let mut flow = RegistryFlow::new(&c, &a, bot(), "atencion", "prop-1", CAND, Some(BASE.into()));
    flow.approve().unwrap();
    let before = f.log().len();
    assert!(matches!(flow.alias_read("staging"), Err(RegistryError::Flow(m)) if m.contains("before publish")));
    assert_eq!(f.log().len(), before);
    assert_eq!(flow.alias_read("prod").unwrap().release_id, BASE, "other aliases stay readable");
    let before = f.log().len();
    for v in ["Staging", "STAGING", " staging", "staging "] {
        assert!(matches!(flow.alias_read(v), Err(RegistryError::Flow(_))), "{v:?}");
    }
    assert_eq!(f.log().len(), before);
}

#[test]
fn full_flow_approves_proves_tamper_and_replay_publishes_and_reads_staging_back() {
    let f = FakeRegistry::start("evaluated", false);
    let (c, a) = (client(&f), auth());
    let mut flow = RegistryFlow::new(&c, &a, bot(), "atencion", "prop-1", CAND, Some(BASE.into()));
    assert!(matches!(flow.publish("pub-0"), Err(RegistryError::Flow(m)) if m.contains("not approved")));
    let r = flow.approve().unwrap();
    assert_eq!((r.decision.as_str(), r.candidate_hash.as_str(), r.approver.as_str()), ("approved", CAND, "local-supervisor"));
    assert!(r.tamper_refused && r.replay_refused);
    let p = flow.publish("pub-1").unwrap();
    assert_eq!((p.release_id.as_str(), p.alias), ("rel-new", "staging"));
    let s = flow.alias_read("staging").unwrap();
    assert_eq!((s.release_id.as_str(), s.alias.as_str(), s.status.as_str()), ("rel-new", "staging", "active"));
    let rel = c.release("rel-new", &bot()).unwrap();
    assert_eq!(rel["release_id"], "rel-new");
    // wire: reads use the bot credential, writes a human JWS bound to the proposal; publish carried its idempotency key
    for r in f.log() {
        match r.method.as_str() {
            "GET" => assert_eq!(r.auth, BOT),
            _ => {
                assert_ne!(r.auth, BOT);
                let claims = Jws::new(r.auth.clone()).claims().unwrap();
                assert_eq!(claims["auth"]["simulated"], true);
                assert_eq!(claims["attrs"]["proposal_id"], "prop-1");
            }
        }
    }
    let publishes: Vec<_> = f.log().into_iter().filter(|r| r.path.ends_with("/publish")).collect();
    assert_eq!(publishes.len(), 1);
    assert_eq!(publishes[0].idem.as_deref(), Some("pub-1"));
    assert_eq!(Jws::new(publishes[0].auth.clone()).claims().unwrap()["attrs"]["operation"], "publish");
}

#[test]
fn approving_a_proposal_that_is_not_evaluated_is_refused_by_core_and_surfaces_the_code() {
    let f = FakeRegistry::start("candidate", false);
    let (c, a) = (client(&f), auth());
    let mut flow = RegistryFlow::new(&c, &a, bot(), "atencion", "prop-1", CAND, Some(BASE.into()));
    assert_eq!(flow.approve(), Err(RegistryError::Http { status: 409, code: Some("illegal_transition".into()) }));
}

#[test]
fn path_params_that_could_escape_the_route_are_refused_locally() {
    let f = FakeRegistry::start("evaluated", false);
    let c = client(&f);
    for bad in ["../x", "a/b", "%2e%2e", ""] {
        assert!(matches!(c.alias(bad, "staging", &bot()), Err(RegistryError::Refused(Refusal::InvalidPathParam(_)))), "{bad}");
    }
    assert_eq!(f.log().len(), 0);
}

#[test]
fn nothing_debug_printed_carries_a_token_or_key_material() {
    let f = FakeRegistry::start("evaluated", false);
    let (c, a) = (client(&f), auth());
    let jws = a.authorize("approve", &target(CAND)).unwrap();
    let human = jws.reveal().to_string();
    let mut shown = format!("{jws:?} {a:?} {:?}", bot());
    shown += &format!("{:?}", c.approve(&target(CAND), &"0".repeat(64), &jws));
    shown += &format!("{:?}", c.approve(&target(CAND), CAND, &jws));
    shown += &format!("{:?}", c.approve(&target(CAND), CAND, &jws)); // replay error
    let mut flow = RegistryFlow::new(&c, &a, bot(), "atencion", "prop-1", CAND, None);
    shown += &format!("{:?}{:?}", flow.approve(), flow.publish("pub-1"));
    let seed_hex: String = SEED.iter().map(|b| format!("{b:02x}")).collect();
    for needle in [human.as_str(), human.split('.').nth(2).unwrap(), BOT, seed_hex.as_str()] {
        assert!(!shown.contains(needle), "leaked {needle}");
    }
}

/// LIVE (not run by default, no containers are started by this suite): the real image's Core `/v1/registry`.
/// Needs a running stack from `e2e-core/run.ps1` (Podman, owned by the operator), a frozen AND natively evaluated
/// proposal (the K3 writer flow + evaluate stage), and the local human issuer's SIM key (the stack's
/// `local-identity` seed; the registry trusts the matching public key).
/// Run: PULSO_CORE_ADDR=127.0.0.1:<port> PULSO_BOT_JWS=<bridge-issued registry_write credential>
///      PULSO_HUMAN_SEED_HEX=<64 hex> PULSO_HUMAN_KID=local-sim-human-1 PULSO_LIVE_TENANT=t1 PULSO_LIVE_AGENT=atencion
///      PULSO_PROPOSAL_ID=<id> PULSO_CANDIDATE_HASH=<hex64> PULSO_BASE_RELEASE=<prod release>
///      cargo test --manifest-path seams/Cargo.toml --offline -j 1 -p core-client --test registry -- --ignored live_k3_registry
/// Env values are never printed.
#[test]
#[ignore = "needs a live Core stack with an evaluated proposal; see doc comment (no Podman in this suite)"]
fn live_k3_registry_approve_publish_readback() {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
    let hex = env("PULSO_HUMAN_SEED_HEX");
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex seed");
    }
    let a = LocalSimAuthorizer::new(&env("PULSO_HUMAN_KID"), seed, &env("PULSO_LIVE_TENANT"), "local-supervisor");
    let c = RegistryClient::new(&env("PULSO_CORE_ADDR"), Duration::from_secs(60));
    let base = env("PULSO_BASE_RELEASE");
    let mut flow = RegistryFlow::new(&c, &a, Jws::new(env("PULSO_BOT_JWS")), &env("PULSO_LIVE_AGENT"), &env("PULSO_PROPOSAL_ID"), &env("PULSO_CANDIDATE_HASH"), Some(base));
    let r = flow.approve().expect("approve");
    assert!(r.tamper_refused && r.replay_refused);
    let p = flow.publish(&format!("pub-k3-live-{}", std::process::id())).expect("publish");
    assert_eq!(flow.alias_read("staging").expect("staging").release_id, p.release_id);
}
