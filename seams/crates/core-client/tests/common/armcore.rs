//! ArmCore: a stateful, effect-counting fake of the arm routes with scripted faults. It models receiver-side
//! idempotency (key -> body digest + stored report) and counts server-side EFFECTS (`effects()`), which is what the
//! K5a acceptance asserts. Faults pop in order and apply to the next request that reaches the server.
#![allow(dead_code)]
use super::envelope;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Clone, Debug)]
pub enum Fault {
    /// Execute and store the effect, then close the socket without answering (the response is lost).
    DropAfterEffect,
    /// Close the socket before doing anything (the request is lost after being sent).
    DropBeforeEffect,
    /// Answer 429 `pulso:bridge_busy` (no effect) with an optional `Retry-After` header (seconds).
    Throttle(Option<u64>),
    /// Answer 429 with a code that is not in the contract table.
    ThrottleUnknownCode,
    /// The next by-key read answers a report of ANOTHER run (execution_id of a different key).
    ReadOtherRun,
    /// The next by-key read answers this key's id but with a different arm (key reused with another body).
    ReadOtherBody,
}

#[derive(Default)]
pub struct ArmState {
    pub effects: u32,
    pub reports: HashMap<(String, String), (String, Value)>,
    pub faults: VecDeque<Fault>,
    pub requests: Vec<(String, String, Option<String>)>,
}

pub struct ArmCore {
    pub addr: String,
    pub state: Arc<Mutex<ArmState>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

fn report(tenant: &str, key: &str, body: &Value) -> Value {
    let id = core_client::canon::arm_execution_id(tenant, key).unwrap();
    json!({"arm": body["arm"], "case_ref": body["case_ref"], "closed_early": false, "closed_early_runs": [], "cost_known": true,
        "effect_receipts": [], "event_refs": [], "execution_id": id, "final_state_ref": null, "initial_state_digest": null,
        "oracle_ref": null, "reason": null, "repetition": body["repetition"], "seed": body["seed"], "status": "completed",
        "target_commitment": null, "trace_id": "t-1", "usage": null})
}

fn respond(s: &mut TcpStream, status: u16, extra: &str, body: &Value) {
    let p = serde_json::to_vec(body).unwrap();
    let h = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n", p.len());
    let _ = s.write_all(h.as_bytes());
    let _ = s.write_all(&p);
}

fn serve(mut s: TcpStream, st: &Arc<Mutex<ArmState>>) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let he = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..he]).to_string();
    let mut lines = head.split("\r\n");
    let mut rl = lines.next().unwrap_or("").split(' ');
    let (method, path) = (rl.next().unwrap_or("").to_string(), rl.next().unwrap_or("").to_string());
    let headers: HashMap<String, String> =
        lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = buf[he + 4..].to_vec();
    while body.len() < len {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
        }
    }
    let tenant = headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| t.split('.').nth(1))
        .and_then(|p| B64.decode(p).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|c| c["tenant_id"].as_str().map(str::to_string))
        .unwrap_or_default();
    let mut g = st.lock().unwrap();
    g.requests.push((method.clone(), path.clone(), headers.get("idempotency-key").cloned()));
    let fault = g.faults.pop_front();
    match fault {
        Some(Fault::DropBeforeEffect) => return,
        Some(Fault::Throttle(ra)) => {
            let extra = ra.map(|r| format!("Retry-After: {r}\r\n")).unwrap_or_default();
            return respond(&mut s, 429, &extra, &envelope("pulso:bridge_busy", true));
        }
        Some(Fault::ThrottleUnknownCode) => return respond(&mut s, 429, "", &envelope("pulso:made_up_code", true)),
        _ => {}
    }
    let drop_after = matches!(fault, Some(Fault::DropAfterEffect));
    if method == "POST" && path == "/internal/v1/evaluation/arms/run" {
        let key = headers.get("idempotency-key").cloned().unwrap_or_default();
        let b: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let digest = String::from_utf8_lossy(&body).to_string();
        let slot = (tenant.clone(), key.clone());
        let (st_code, out) = match g.reports.get(&slot) {
            Some((d, r)) if *d == digest => (200, r.clone()),
            Some(_) => (409, envelope("pulso:idempotency_conflict", false)),
            None => {
                g.effects += 1;
                let r = report(&tenant, &key, &b);
                g.reports.insert(slot, (digest, r.clone()));
                (200, r)
            }
        };
        if drop_after {
            return;
        }
        respond(&mut s, st_code, "", &out);
    } else if method == "GET" && path.starts_with("/internal/v1/evaluation/arms/by-key/") {
        let key = &path["/internal/v1/evaluation/arms/by-key/".len()..];
        match g.reports.get(&(tenant, key.to_string())) {
            Some((_, r)) => {
                let mut r = r.clone();
                match fault {
                    Some(Fault::ReadOtherRun) => r["execution_id"] = json!(core_client::canon::arm_execution_id("t1", "someone-else").unwrap()),
                    Some(Fault::ReadOtherBody) => r["arm"] = json!("other-arm"),
                    _ => {}
                }
                respond(&mut s, 200, "", &r)
            }
            None => respond(&mut s, 404, "", &envelope("pulso:not_found", false)),
        }
    } else {
        respond(&mut s, 404, "", &envelope("pulso:not_found", false));
    }
}

impl ArmCore {
    pub fn start(faults: Vec<Fault>) -> ArmCore {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let state = Arc::new(Mutex::new(ArmState { faults: faults.into(), ..Default::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, p2) = (state.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            for c in l.incoming() {
                if p2.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(c) = c {
                    serve(c, &s2);
                }
            }
        });
        ArmCore { addr, state, stop, handle: Some(handle) }
    }
    pub fn effects(&self) -> u32 {
        self.state.lock().unwrap().effects
    }
    pub fn requests(&self) -> Vec<(String, String, Option<String>)> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl Drop for ArmCore {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
