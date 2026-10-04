//! Gateway port against a LOCAL FAKE llm-gateway (std TcpListener). Disabled unless explicitly configured; E0/original data and
//! payloads that fail the TPS scan are refused before any byte is sent; only an answered call is `real`.
use engine::models::gateway::Gateway;
use engine::models::{DataClass, Label, ModelError, ModelPort, ModelRequest, Recording, Role};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

struct Fake {
    addr: String,
    seen: Arc<Mutex<Vec<(String, HashMap<String, String>, Value)>>>,
}

fn fake(status: u16, body: Value) -> Fake {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let seen = Arc::new(Mutex::new(vec![]));
    let s2 = seen.clone();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { continue };
            let mut buf = vec![];
            let mut tmp = [0u8; 4096];
            let head_end = loop {
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p;
                }
                match c.read(&mut tmp) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let mut lines = head.split("\r\n");
            let path = lines.next().unwrap_or("").to_string();
            let h: HashMap<String, String> = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
            let len: usize = h.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
            let mut body = buf[head_end + 4..].to_vec();
            while body.len() < len {
                match c.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => body.extend_from_slice(&tmp[..n]),
                }
            }
            s2.lock().unwrap().push((path, h, serde_json::from_slice(&body).unwrap_or(Value::Null)));
            let payload = serde_json::to_vec(&JSON.lock().unwrap().clone()).unwrap();
            let _ = write!(c, "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", STATUS.lock().unwrap(), payload.len());
            let _ = c.write_all(&payload);
        }
    });
    *STATUS.lock().unwrap() = status;
    *JSON.lock().unwrap() = body;
    Fake { addr, seen }
}

static STATUS: Mutex<u16> = Mutex::new(200);
static JSON: Mutex<Value> = Mutex::new(Value::Null);

fn chat(content: &str, model: &str) -> Value {
    json!({"model": model, "choices": [{"message": {"role": "assistant", "content": content}}]})
}

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |k| m.get(k).cloned()
}

fn on(addr: &str, extra: &[(&str, &str)]) -> Gateway {
    let mut p = vec![("PULSO_MODEL_GATEWAY", "enabled"), ("PULSO_GATEWAY_ADDR", addr), ("PULSO_GATEWAY_MODEL", "pulso-evolution-llm"), ("PULSO_GATEWAY_KEY", "k-test")];
    p.extend_from_slice(extra);
    Gateway::from_env(&env(&p)).unwrap()
}

fn req() -> ModelRequest {
    ModelRequest {
        role: Role::Scout,
        system: "stage prompt".into(),
        payload: json!({"goal": "g", "inputs": {"signal_id": "sig-0001"}, "step": 0, "tools": [], "observations": []}),
        registry: vec!["sig-0001".into()],
        data_class: DataClass::Treated,
    }
}

// The fake keeps its scripted answer in statics, so the tests that use it run one after another.
static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn without_explicit_configuration_the_gateway_is_disabled_and_refuses() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let gw = Gateway::from_env(&env(&[("PULSO_GATEWAY_ADDR", "127.0.0.1:1"), ("PULSO_GATEWAY_MODEL", "m")])).unwrap();
    assert!(matches!(gw.call(&req()), Err(ModelError::Refused(w)) if w.starts_with("gateway_disabled")));
    assert!(Gateway::from_env(&env(&[("PULSO_MODEL_GATEWAY", "yes"), ("PULSO_GATEWAY_ADDR", "127.0.0.1:1"), ("PULSO_GATEWAY_MODEL", "m")])).unwrap().call(&req()).is_err(), "only the exact value enabled turns it on");
    assert!(Gateway::from_env(&env(&[("PULSO_MODEL_GATEWAY", "enabled")])).is_err(), "enabled without address and model is a configuration error");
    assert!(Gateway::from_env(&env(&[("PULSO_MODEL_GATEWAY", "enabled"), ("PULSO_GATEWAY_ADDR", "x"), ("PULSO_GATEWAY_MODEL", "m"), ("PULSO_GATEWAY_KIND", "other")])).is_err());
}

#[test]
fn an_answered_call_is_labelled_gateway_and_is_the_only_real_one() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, chat(r#"{"hypotheses":[{"id":"h_1","signal_id":"sig-0001","claimed_rate":0.3}]}"#, "gpt-local-x"));
    let rec = Recording::new(Rc::new(on(&f.addr, &[])));
    let a = rec.call(&req()).unwrap();
    assert_eq!((a.label, a.model_id.as_str()), (Label::Gateway, "gpt-local-x"));
    assert_eq!(a.content["hypotheses"][0]["signal_id"], "sig-0001");
    let j = rec.calls()[0].to_json();
    assert_eq!((j["real"].as_bool(), j["status"].as_str(), j["provider"].as_str()), (Some(true), Some("real"), Some("gateway:gpt-local-x")));
    assert!(rec.doubles().is_empty(), "an answered gateway call is not a double");
    let seen = f.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (path, h, body) = &seen[0];
    assert!(path.starts_with("POST /v1/chat/completions"), "{path}");
    assert_eq!(h["authorization"], "Bearer k-test");
    assert_eq!(body["model"], "pulso-evolution-llm");
    assert_eq!(body["messages"][0], json!({"role": "system", "content": "stage prompt"}));
    assert_eq!(body["messages"][1]["role"], "user");
    let user: Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(user, req().payload, "the user message is the canonical agent input dict, nothing else");
    assert!(body.get("stream").is_none_or(|s| s == &json!(false)));
}

#[test]
fn e0_original_and_untreated_payloads_are_refused_before_any_byte_is_sent() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, chat("{}", "m"));
    let gw = on(&f.addr, &[]);
    for dc in [DataClass::E0, DataClass::Original] {
        assert!(matches!(gw.call(&ModelRequest { data_class: dc, ..req() }), Err(ModelError::Refused(w)) if w.contains("never reaches a hosted model")), "{dc:?}");
    }
    let mut r = req();
    r.payload["inputs"]["comment"] = json!("maria.perez@example.com wrote that she was angry");
    assert!(matches!(gw.call(&r), Err(ModelError::Refused(w)) if w.starts_with("tps:")));
    let rec = Recording::new(Rc::new(on(&f.addr, &[])));
    let _ = rec.call(&r);
    assert_eq!(rec.calls()[0].to_json()["real"], false);
    assert!(f.seen.lock().unwrap().is_empty(), "nothing reached the gateway");
}

#[test]
fn gateway_failures_map_to_typed_errors_and_never_to_real() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(422, json!({"error": {"type": "treated_payload_rejected", "message": "x"}}));
    assert!(matches!(on(&f.addr, &[]).call(&req()), Err(ModelError::Refused(w)) if w.starts_with("remote_tps")));
    let f = fake(504, json!({"error": {"type": "responder_timeout", "message": "x"}}));
    assert!(matches!(on(&f.addr, &[]).call(&req()), Err(ModelError::Unavailable(w)) if w.contains("504")));
    let f = fake(200, chat("this is prose, not JSON", "m"));
    assert!(matches!(on(&f.addr, &[]).call(&req()), Err(ModelError::Invalid(_))));
    let f = fake(200, json!({"choices": []}));
    assert!(matches!(on(&f.addr, &[]).call(&req()), Err(ModelError::Invalid(_))));
    assert!(matches!(on("127.0.0.1:1", &[]).call(&req()), Err(ModelError::Unavailable(w)) if w.contains("unreachable")));
}

#[test]
fn a_local_model_endpoint_is_labelled_local_model_not_real() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, chat(r#"{"verdict":"agree"}"#, "llama-local"));
    let rec = Recording::new(Rc::new(on(&f.addr, &[("PULSO_GATEWAY_KIND", "local-model")])));
    let a = rec.call(&ModelRequest { role: Role::Verifier, ..req() }).unwrap();
    assert_eq!(a.label, Label::LocalModel);
    let j = rec.calls()[0].to_json();
    assert_eq!((j["real"].as_bool(), j["provider"].as_str()), (Some(false), Some("local-model")));
    assert_eq!(rec.doubles().len(), 1);
}

#[test]
fn plain_http_is_only_allowed_toward_loopback_or_private_hosts() {
    let ok = |addr: &str| Gateway::from_env(&env(&[("PULSO_MODEL_GATEWAY", "enabled"), ("PULSO_GATEWAY_ADDR", addr), ("PULSO_GATEWAY_MODEL", "m")])).is_ok();
    for a in ["127.0.0.1:8080", "localhost:8080", "[::1]:8080", "10.1.2.3:80", "192.168.0.5:80", "172.16.0.1:80", "172.31.255.255:80"] {
        assert!(ok(a), "{a} should be accepted");
    }
    for a in ["8.8.8.8:80", "gateway.example.com:80", "127.0.0.1.evil.com:80", "172.32.0.1:80", "169.253.1.1:80", "localhost.evil.com:80", "no-port", "@evil.com:80", "127.0.0.1@evil.com:80"] {
        assert!(!ok(a), "{a} must be refused (no TLS in this client)");
    }
    let forced = Gateway::from_env(&env(&[("PULSO_MODEL_GATEWAY", "enabled"), ("PULSO_GATEWAY_ADDR", "gateway.example.com:80"), ("PULSO_GATEWAY_MODEL", "m"), ("PULSO_GATEWAY_ALLOW_REMOTE_PLAINTEXT", "yes")]));
    assert!(forced.is_ok(), "an explicit operator override exists");
}

#[test]
fn an_oversized_payload_split_across_fields_is_refused_before_any_byte_is_sent() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, chat("{}", "m"));
    let gw = on(&f.addr, &[]);
    let mut r = req();
    r.payload["tools"] = json!((0..400).map(|i| json!({"tool": format!("pulso/t{i}@1.0.0"), "description": "x".repeat(1900), "args_schema": {}})).collect::<Vec<_>>());
    assert!(matches!(gw.call(&r), Err(ModelError::Refused(w)) if w.contains("too large")));
    assert!(f.seen.lock().unwrap().is_empty());
}
