//! GoldenCore: a FakeCore GENERATED from `bridge-contract/examples/flows` (via `gen/gen_fakecore.py`
//! -> `golden_flows.json`). It replays golden steps as a transcript: the test names the (flow, case) steps it
//! expects, in order; every incoming request must equal that step's golden request (method, path, JWT claim
//! profile, Idempotency-Key, body) and is answered with the golden status and body. Anything else is recorded as a
//! failure and answered 418, so a client that encodes anything differently from the Python bridge golden fails.
//!
//! Matching rules: `null`-valued TOP-LEVEL body keys equal absent keys (nested nulls are content) and a top-level `schema_version: "1"` equals absent (the
//! DTOs treat them alike, `default: "1"`); a golden string of the form
//! `<name>` is a placeholder for a volatile value: it matches any string, the same name must always bind the same
//! value and different names different values (so `request_digest#2` vs `#3` really differ); `deadline*` must be
//! Z-RFC3339 and `request_digest*` lowercase hex-64.
#![allow(dead_code)]
use super::{Seen, verify};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

const FLOWS: &str = include_str!("golden_flows.json");

#[derive(Clone, Debug)]
pub struct Step {
    pub flow: String,
    pub case: String,
    pub method: String,
    pub path: String,
    pub purpose: Option<String>,
    pub tenant_id: Option<String>,
    pub job_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub request_body: Value,
    pub status: u16,
    pub response_body: Value,
}

pub fn source_sha256() -> String {
    let v: Value = serde_json::from_str(FLOWS).unwrap();
    v["source_sha256"].as_str().unwrap().to_string()
}

pub fn step(flow: &str, case: &str) -> Step {
    let v: Value = serde_json::from_str(FLOWS).unwrap();
    let s = &v["flows"][flow][case];
    assert!(s.is_object(), "no golden step {flow}/{case}");
    let o = |k: &str| s[k].as_str().map(str::to_string);
    Step {
        flow: flow.into(),
        case: case.into(),
        method: o("method").unwrap(),
        path: o("path").unwrap(),
        purpose: o("purpose"),
        tenant_id: o("tenant_id"),
        job_id: o("job_id"),
        idempotency_key: o("idempotency_key"),
        request_body: s["request_body"].clone(),
        status: s["status"].as_u64().unwrap() as u16,
        response_body: s["response_body"].clone(),
    }
}

/// The golden response body of a step (placeholders left as the literal strings).
pub fn golden_response(flow: &str, case: &str) -> Value {
    step(flow, case).response_body
}

fn is_placeholder(s: &str) -> bool {
    s.len() > 2 && s.starts_with('<') && s.ends_with('>')
}

fn is_z(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[b.len() - 1] == b'Z'
        && b[..4].iter().all(u8::is_ascii_digit)
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Default, Clone)]
struct Bindings {
    by_name: HashMap<String, String>,
}

impl Bindings {
    fn bind(&mut self, name: &str, value: &str) -> Result<(), String> {
        let bare = name.trim_matches(|c| c == '<' || c == '>');
        if bare.starts_with("deadline") && !is_z(value) {
            return Err(format!("{name}: {value:?} is not Z-RFC3339"));
        }
        if bare.starts_with("request_digest") && !is_hex64(value) {
            return Err(format!("{name}: {value:?} is not lowercase hex-64"));
        }
        match self.by_name.get(name) {
            Some(v) if v == value => Ok(()),
            Some(v) => Err(format!("{name} was bound to {v:?}, now {value:?}")),
            None => {
                if let Some((other, _)) = self.by_name.iter().find(|(n, v)| v.as_str() == value && n.as_str() != name) {
                    return Err(format!("{name} and {other} must differ but both are {value:?}"));
                }
                self.by_name.insert(name.to_string(), value.to_string());
                Ok(())
            }
        }
    }
}

fn strip_top_nulls(m: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    m.iter().filter(|(_, x)| !x.is_null()).map(|(k, x)| (k.clone(), x.clone())).collect()
}

fn matches(golden: &Value, actual: &Value, b: &mut Bindings, at: &str) -> Result<(), String> {
    match (golden, actual) {
        (Value::String(g), Value::String(a)) if is_placeholder(g) => b.bind(g, a),
        (Value::Object(g), Value::Object(a)) => {
            // `null` == absent ONLY for the top-level optional fields of the body (`seed_manifest_ref`,
            // `base_release_id`, ...). Inside `input`/`target`/... a null is content: dropping or adding one changes
            // the request and must not be masked.
            let (g, a) = if at == "body" { (strip_top_nulls(g), strip_top_nulls(a)) } else { (g.clone(), a.clone()) };
            for k in g.keys().chain(a.keys()).collect::<HashSet<_>>() {
                // `schema_version` defaults to "1" in the DTOs: a golden that omits it equals a request that sends "1".
                if k == "schema_version" && at == "body" && g.get(k).is_none() && a.get(k) == Some(&Value::String("1".into())) {
                    continue;
                }
                match (g.get(k), a.get(k)) {
                    (Some(gv), Some(av)) => matches(gv, av, b, &format!("{at}.{k}"))?,
                    (Some(_), None) => return Err(format!("{at}.{k}: missing in the request")),
                    (None, Some(_)) => return Err(format!("{at}.{k}: not in the golden request")),
                    _ => unreachable!(),
                }
            }
            Ok(())
        }
        (Value::Array(g), Value::Array(a)) => {
            if g.len() != a.len() {
                return Err(format!("{at}: array length {} != golden {}", a.len(), g.len()));
            }
            for (i, (gv, av)) in g.iter().zip(a).enumerate() {
                matches(gv, av, b, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        (g, a) if g == a => Ok(()),
        (g, a) => Err(format!("{at}: request {a} != golden {g}")),
    }
}

/// Compare a golden request body with an actual one (placeholders bind per call; fresh bindings).
pub fn body_matches(golden: &Value, actual: &Value) -> Result<(), String> {
    matches(golden, actual, &mut Bindings::default(), "body")
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[derive(Default)]
struct GState {
    queue: VecDeque<Step>,
    bindings: Bindings,
    failures: Vec<String>,
    served: Vec<String>,
    requests: Vec<Seen>,
    jtis: HashSet<String>,
}

fn fail(st: &mut GState, why: String) -> (u16, Value) {
    eprintln!("GoldenCore deviation: {why}");
    st.failures.push(why);
    (418, serde_json::json!({"code":"pulso:internal_error","retryable":false,"schema_version":"1","details":{}}))
}

fn handle(st: &mut GState, seen: &Seen) -> (u16, Value) {
    let Some(step) = st.queue.pop_front() else {
        return fail(st, format!("unexpected request {} {} (no golden step queued)", seen.method, seen.path));
    };
    let tag = format!("{}/{}", step.flow, step.case);
    let mut b = st.bindings.clone();
    if seen.method != step.method {
        return fail(st, format!("{tag}: method {} != {}", seen.method, step.method));
    }
    let got_path = pct_decode(seen.path.strip_prefix("/internal/v1").unwrap_or(&seen.path));
    let gs: Vec<&str> = step.path.split('/').collect();
    let as_: Vec<&str> = got_path.split('/').collect();
    if gs.len() != as_.len() {
        return fail(st, format!("{tag}: path {got_path} != {}", step.path));
    }
    for (g, a) in gs.iter().zip(&as_) {
        let r = if is_placeholder(g) {
            b.bind(g, a)
        } else if g == a {
            Ok(())
        } else {
            Err(format!("segment {a:?} != {g:?}"))
        };
        if let Err(e) = r {
            return fail(st, format!("{tag}: path {got_path} vs {}: {e}", step.path));
        }
    }
    let token = seen.headers.get("authorization").and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    let claims = match verify(token, step.purpose.as_deref().unwrap_or("")) {
        Ok(c) => c,
        Err(r) => return fail(st, format!("{tag}: service JWT rejected: {r}")),
    };
    if !st.jtis.insert(claims["jti"].as_str().unwrap_or("").to_string()) {
        return fail(st, format!("{tag}: jti replayed"));
    }
    if claims.get("tenant_id").and_then(Value::as_str) != step.tenant_id.as_deref() {
        return fail(st, format!("{tag}: tenant_id claim {:?} != golden {:?}", claims.get("tenant_id"), step.tenant_id));
    }
    if claims.get("job_id").and_then(Value::as_str) != step.job_id.as_deref() {
        return fail(st, format!("{tag}: job_id claim {:?} != golden {:?}", claims.get("job_id"), step.job_id));
    }
    if let Some(k) = &step.idempotency_key {
        if seen.headers.get("idempotency-key") != Some(k) {
            return fail(st, format!("{tag}: Idempotency-Key {:?} != golden {k}", seen.headers.get("idempotency-key")));
        }
    }
    let actual: Value = if seen.body.is_empty() { Value::Null } else { serde_json::from_slice(&seen.body).unwrap_or(Value::Null) };
    if step.request_body.is_null() != actual.is_null() {
        return fail(st, format!("{tag}: body presence differs from the golden"));
    }
    if !actual.is_null() {
        if let Err(e) = matches(&step.request_body, &actual, &mut b, "body") {
            return fail(st, format!("{tag}: {e}"));
        }
    }
    st.bindings = b;
    st.served.push(tag);
    (step.status, step.response_body.clone())
}

pub struct GoldenCore {
    pub addr: String,
    state: Arc<Mutex<GState>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

fn serve_conn(mut s: TcpStream, state: &Arc<Mutex<GState>>) {
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
    let seen = Seen { method, path, headers, body, claims: None };
    let (status, out) = {
        let mut st = state.lock().unwrap();
        let r = handle(&mut st, &seen);
        st.requests.push(seen);
        r
    };
    let payload = serde_json::to_vec(&out).unwrap();
    let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(&payload);
}

impl GoldenCore {
    /// `cases`: the `(flow, case)` steps the test expects the client to drive, in order.
    pub fn play(cases: &[(&str, &str)]) -> GoldenCore {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let state = Arc::new(Mutex::new(GState { queue: cases.iter().map(|(f, c)| step(f, c)).collect(), ..Default::default() }));
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
        GoldenCore { addr, state, stop, handle: Some(handle) }
    }
    pub fn requests(&self) -> Vec<Seen> {
        self.state.lock().unwrap().requests.clone()
    }
    pub fn served(&self) -> Vec<String> {
        self.state.lock().unwrap().served.clone()
    }
    /// Panics unless every queued golden step was served and no request deviated from its golden.
    pub fn finish(&self) {
        let st = self.state.lock().unwrap();
        assert!(st.failures.is_empty(), "requests deviated from the goldens:\n{}", st.failures.join("\n"));
        assert!(st.queue.is_empty(), "golden steps never requested: {:?}", st.queue.iter().map(|s| &s.case).collect::<Vec<_>>());
    }
}

impl Drop for GoldenCore {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
