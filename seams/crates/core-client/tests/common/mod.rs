//! FakeCore: an HTTP/1.1 stub of the bridge `/internal/v1` over a std `TcpListener`.
//! It verifies the service JWT like the Python verifier and models receiver-side idempotency.
#![allow(dead_code)]
pub mod armcore;
pub mod golden;
pub mod relay;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub const KID: &str = "k1";
pub const SEED: [u8; 32] = [7u8; 32];

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub claims: Option<Value>,
}

#[derive(Default)]
pub struct State {
    pub runs: u32,
    pub requests: Vec<Seen>,
    pub by_key: HashMap<String, (String, Value)>,
    pub jtis: HashSet<String>,
    /// Scripted responses (status, body) served before normal handling.
    pub script: VecDeque<(u16, Value)>,
}

pub struct FakeCore {
    pub addr: String,
    pub state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

pub fn envelope(code: &str, retryable: bool) -> Value {
    json!({"schema_version":"1","code":code,"retryable":retryable,"trace_id":"t-1","details":{}})
}

pub fn verify(token: &str, purpose_expected: &str) -> Result<Value, &'static str> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("malformed");
    }
    let dec = |s: &str| B64.decode(s).map_err(|_| "malformed");
    let header: Value = serde_json::from_slice(&dec(parts[0])?).map_err(|_| "malformed")?;
    let claims: Value = serde_json::from_slice(&dec(parts[1])?).map_err(|_| "malformed")?;
    let sig = dec(parts[2])?;
    let h = header.as_object().ok_or("malformed")?;
    let mut keys: Vec<_> = h.keys().cloned().collect();
    keys.sort();
    if h.get("alg") != Some(&json!("EdDSA")) || h.get("typ") != Some(&json!("JWT")) || keys != ["alg", "kid", "typ"] {
        return Err("bad_header");
    }
    if h.get("kid") != Some(&json!(KID)) {
        return Err("unknown_kid");
    }
    let sk = ed25519_dalek::SigningKey::from_bytes(&SEED);
    let vk: VerifyingKey = sk.verifying_key();
    let sig = Signature::from_slice(&sig).map_err(|_| "bad_signature")?;
    vk.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig).map_err(|_| "bad_signature")?;
    if claims["iss"] != "control-api" {
        return Err("key_binding");
    }
    if claims["aud"] != "core-bridge" {
        return Err("wrong_audience");
    }
    let (iat, exp) = (claims["iat"].as_i64().ok_or("missing_claims")?, claims["exp"].as_i64().ok_or("missing_claims")?);
    if exp - iat > 300 {
        return Err("ttl_too_long");
    }
    if claims["purpose"] != purpose_expected {
        return Err("purpose_denied");
    }
    if !claims["sub"].as_str().is_some_and(|s| s.starts_with("worker:") && s.len() > 7) {
        return Err("sub_not_worker");
    }
    if purpose_expected != "version_probe" && !claims["tenant_id"].as_str().is_some_and(|s| !s.is_empty()) {
        return Err("tenant_required");
    }
    Ok(claims)
}

fn purpose_for(method: &str, path: &str) -> Option<&'static str> {
    Some(match (method, path) {
        ("POST", "/internal/v1/core-tasks/invoke") => "core_task_invoke",
        ("POST", "/internal/v1/evaluation/arms/run") => "evaluation_arm_run",
        ("POST", "/internal/v1/evaluation/admissions") => "evaluation_admit",
        ("GET", "/internal/v1/version") => "version_probe",
        _ => return None,
    })
}

fn handle(st: &mut State, seen: &Seen) -> (u16, Value) {
    if let Some(s) = st.script.pop_front() {
        return s;
    }
    let Some(purpose) = purpose_for(&seen.method, &seen.path) else {
        return (404, envelope("pulso:not_found", false));
    };
    let token = seen.headers.get("authorization").and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    if token.is_empty() {
        return (401, envelope("pulso:auth_invalid", false));
    }
    let claims = match verify(token, purpose) {
        Ok(c) => c,
        Err(r) if r == "purpose_denied" || r == "sub_not_worker" || r == "tenant_required" => {
            let mut e = envelope("pulso:auth_denied", false);
            e["details"] = json!({"reason": r});
            return (403, e);
        }
        Err(r) => {
            let mut e = envelope("pulso:auth_invalid", false);
            e["details"] = json!({"reason": r});
            return (401, e);
        }
    };
    let jti = claims["jti"].as_str().unwrap_or("").to_string();
    if !st.jtis.insert(jti) {
        let mut e = envelope("pulso:auth_invalid", false);
        e["details"] = json!({"reason": "jti_replayed"});
        return (401, e);
    }
    match purpose {
        "version_probe" => (200, json!({"agent_core_sha":"c814c2bad9f154d10c092326558815dca9562be7","contracts_version":"1.3.0"})),
        "core_task_invoke" | "evaluation_arm_run" => {
            let Some(key) = seen.headers.get("idempotency-key").cloned() else {
                let mut e = envelope("pulso:invalid_request", false);
                e["details"] = json!({"fields":["Idempotency-Key"]});
                return (422, e);
            };
            let digest = String::from_utf8_lossy(&seen.body).to_string();
            if let Some((d, receipt)) = st.by_key.get(&key) {
                return if *d == digest { (200, receipt.clone()) } else { (409, envelope("pulso:digest_conflict", false)) };
            }
            st.runs += 1;
            let receipt = json!({"run_id": format!("run-{}", st.runs), "state": "terminal_ok"});
            st.by_key.insert(key, (digest, receipt.clone()));
            (200, receipt)
        }
        _ => (200, json!({})),
    }
}

fn serve_conn(mut s: TcpStream, state: &Arc<Mutex<State>>) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut rl = lines.next().unwrap_or("").split(' ');
    let (method, path) = (rl.next().unwrap_or("").to_string(), rl.next().unwrap_or("").to_string());
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())))
        .collect();
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
        }
    }
    let claims = headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| t.split('.').nth(1))
        .and_then(|p| B64.decode(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok());
    let seen = Seen { method, path, headers, body, claims };
    let (status, out) = {
        let mut st = state.lock().unwrap();
        let r = handle(&mut st, &seen);
        st.requests.push(seen);
        r
    };
    let payload = serde_json::to_vec(&out).unwrap();
    let resp = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let _ = s.write_all(resp.as_bytes());
    let _ = s.write_all(&payload);
}

impl FakeCore {
    pub fn start() -> FakeCore {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (st2, stop2) = (state.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            for conn in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(c) = conn {
                    serve_conn(c, &st2);
                }
            }
        });
        FakeCore { addr, state, stop, handle: Some(handle) }
    }
    pub fn runs(&self) -> u32 {
        self.state.lock().unwrap().runs
    }
    pub fn script(&self, status: u16, body: Value) {
        self.state.lock().unwrap().script.push_back((status, body));
    }
    pub fn requests(&self) -> Vec<Seen> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl Drop for FakeCore {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
