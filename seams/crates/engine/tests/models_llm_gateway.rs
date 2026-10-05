//! `LlmGateway` port (the REAL llm-gateway `POST /v1/generate` contract) against a LOCAL FAKE (std TcpListener). Disabled unless explicitly
//! configured; E0/original data and payloads that fail the TPS scan are refused before any byte is sent; only an answered call is `real`.
use engine::models::llm_gateway::LlmGateway;
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

fn generated(output: Value, model: &str) -> Value {
    json!({"output": output, "model": model, "tokens_in": 100, "tokens_out": 20, "cost_usd": "0.000100", "usage_known": true})
}

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |k| m.get(k).cloned()
}

fn on(addr: &str, extra: &[(&str, &str)]) -> LlmGateway {
    let mut p = vec![("PULSO_LLM_GATEWAY", "enabled"), ("PULSO_LLM_GATEWAY_ADDR", addr), ("PULSO_LLM_GATEWAY_KEY", "k-test-secret")];
    p.extend_from_slice(extra);
    LlmGateway::from_env(&env(&p)).unwrap()
}

fn req() -> ModelRequest {
    ModelRequest {
        role: Role::Scout,
        system: "stage prompt".into(),
        payload: json!({"goal": "g", "inputs": {"signal_id": "sig-0001"}, "step": 0, "tools": [], "observations": [],
                        "output_schema": {"type": "object", "required": ["verdict"], "additionalProperties": false, "properties": {"verdict": {"type": "string"}}}}),
        registry: vec!["sig-0001".into()],
        data_class: DataClass::Treated,
    }
}

static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn without_explicit_configuration_it_is_disabled_and_refuses() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let gw = LlmGateway::from_env(&env(&[("PULSO_LLM_GATEWAY_ADDR", "127.0.0.1:1"), ("PULSO_LLM_GATEWAY_KEY", "k")])).unwrap();
    assert!(matches!(gw.call(&req()), Err(ModelError::Refused(w)) if w.starts_with("llm_gateway_disabled")));
    assert!(LlmGateway::from_env(&env(&[("PULSO_LLM_GATEWAY", "enabled")])).is_err(), "enabled without address and key is a configuration error");
    assert!(LlmGateway::from_env(&env(&[("PULSO_LLM_GATEWAY", "enabled"), ("PULSO_LLM_GATEWAY_ADDR", "8.8.8.8:80"), ("PULSO_LLM_GATEWAY_KEY", "k")])).is_err(), "no TLS: loopback or private hosts only");
}

#[test]
fn an_answered_call_follows_the_generate_contract_and_is_the_only_real_one() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, generated(json!({"verdict": "agree"}), "xiaomi/mimo-v2.6-flash"));
    let rec = Recording::new(Rc::new(on(&f.addr, &[])));
    let a = rec.call(&req()).unwrap();
    assert_eq!((a.label, a.model_id.as_str()), (Label::Gateway, "xiaomi/mimo-v2.6-flash"));
    assert_eq!(a.content, json!({"verdict": "agree"}));
    let j = rec.calls()[0].to_json();
    assert_eq!((j["real"].as_bool(), j["provider"].as_str()), (Some(true), Some("gateway:xiaomi/mimo-v2.6-flash")));
    assert!(rec.doubles().is_empty());
    let seen = f.seen.lock().unwrap();
    let (path, h, body) = &seen[0];
    assert!(path.starts_with("POST /v1/generate"), "{path}");
    assert_eq!(h["authorization"], "Bearer k-test-secret");
    assert_eq!(body["prompt"], "stage prompt");
    assert_eq!(body["inputs"], req().payload, "inputs is the treated agent input dict, nothing else");
    assert_eq!(body["schema"], req().payload["output_schema"], "the role schema travels as the gateway response schema");
    let p = &body["profile"];
    assert_eq!((p["endpoint_alias"].as_str(), p["model"].as_str(), p["structured"].as_str()), (Some("openrouter"), Some("xiaomi/mimo-v2.6-flash"), Some("prompted")));
    assert!(p["price"]["input_per_mtok"].is_string() && p["price"]["output_per_mtok"].is_string(), "money is a string");
    assert_eq!(p["temperature"], 0);
    assert_eq!(body["labels"]["agent"], "pulso-scout");
    assert!(!format!("{:?}", on(&f.addr, &[]).label()).contains("secret"));
}

#[test]
fn model_and_alias_are_configurable_per_port() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, generated(json!({"verdict": "agree"}), "other/model"));
    let gw = on(&f.addr, &[("PULSO_LLM_GATEWAY_MODEL", "other/model"), ("PULSO_LLM_GATEWAY_ALIAS", "alt")]);
    assert_eq!(gw.model_id(), "other/model");
    gw.call(&req()).unwrap();
    let seen = f.seen.lock().unwrap();
    assert_eq!((seen[0].2["profile"]["model"].as_str(), seen[0].2["profile"]["endpoint_alias"].as_str()), (Some("other/model"), Some("alt")));
}

#[test]
fn e0_original_and_untreated_payloads_are_refused_before_any_byte_is_sent() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, generated(json!({}), "m"));
    let gw = on(&f.addr, &[]);
    for dc in [DataClass::E0, DataClass::Original] {
        assert!(matches!(gw.call(&ModelRequest { data_class: dc, ..req() }), Err(ModelError::Refused(w)) if w.contains("never reaches a hosted model")), "{dc:?}");
    }
    let mut r = req();
    r.payload["inputs"]["comment"] = json!("maria.perez@example.com wrote that she was angry");
    assert!(matches!(gw.call(&r), Err(ModelError::Refused(w)) if w.starts_with("tps:")));
    assert!(f.seen.lock().unwrap().is_empty(), "nothing reached the gateway");
}

#[test]
fn failures_map_to_typed_errors_and_never_to_real() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(502, json!({"error": {"kind": "invalid_output", "message": "x"}}));
    assert!(matches!(on(&f.addr, &[]).call(&req()), Err(ModelError::Invalid(w)) if w.contains("invalid_output")));
    let f = fake(401, json!({"error": {"kind": "unauthorized", "message": "x"}}));
    assert!(matches!(on(&f.addr, &[]).call(&req()), Err(ModelError::Unavailable(w)) if w.contains("401") && w.contains("unauthorized")));
    let f = fake(502, json!({"error": {"kind": "unavailable", "message": "x"}}));
    assert!(matches!(on(&f.addr, &[]).call(&req()), Err(ModelError::Unavailable(w)) if w.contains("unavailable")));
    let f = fake(504, json!({"error": {"kind": "timeout", "message": "x"}}));
    assert!(matches!(on(&f.addr, &[]).call(&req()), Err(ModelError::Unavailable(w)) if w.contains("504")));
    let f = fake(200, generated(json!("prose, not an object"), "m"));
    assert!(matches!(on(&f.addr, &[]).call(&req()), Err(ModelError::Invalid(_))));
    assert!(matches!(on("127.0.0.1:1", &[]).call(&req()), Err(ModelError::Unavailable(w)) if w.contains("unreachable")));
}

#[test]
fn text_mode_sends_no_schema_and_extracts_one_object_from_whatever_the_model_wrapped_it_in() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let wrapped = "<think>maybe</think>Sure! Here you go:\n```json\n{\"verdict\": \"agree\",}\n```\nAnything else?";
    let f = fake(200, generated(json!(wrapped), "xiaomi/mimo-v2.6-flash"));
    let gw = on(&f.addr, &[("PULSO_LLM_GATEWAY_STRUCTURED", "text")]);
    let a = gw.call(&req()).unwrap();
    assert_eq!(a.content, json!({"verdict": "agree"}));
    let seen = f.seen.lock().unwrap();
    assert!(seen[0].2.get("schema").is_none(), "text mode asks the gateway for plain text: no schema is sent");
    drop(seen);
    // the typed failure modes keep their cause in the error (the report says WHY the answer was unusable)
    for (text, code) in [("I cannot help with that.", "no_json"), ("{\"verdict\": \"agree\", \"x\": {\"y\": \"cut o", "truncated"), ("", "empty"), ("[1, 2]", "not_object")] {
        let f = fake(200, generated(json!(text), "m"));
        let gw = on(&f.addr, &[("PULSO_LLM_GATEWAY_STRUCTURED", "text")]);
        assert!(matches!(gw.call(&req()), Err(ModelError::Invalid(w)) if w == format!("answer_not_json:{code}")), "{text}");
    }
}

#[test]
fn native_mode_sends_the_schema_as_a_provider_response_format_and_the_default_stays_prompted() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, generated(json!({"verdict": "agree"}), "m"));
    on(&f.addr, &[("PULSO_LLM_GATEWAY_STRUCTURED", "native")]).call(&req()).unwrap();
    on(&f.addr, &[]).call(&req()).unwrap();
    let seen = f.seen.lock().unwrap();
    assert_eq!((seen[0].2["profile"]["structured"].as_str(), seen[0].2["schema"].is_object()), (Some("native"), true));
    assert_eq!(seen[1].2["profile"]["structured"], "prompted");
    assert!(LlmGateway::from_env(&env(&[("PULSO_LLM_GATEWAY", "enabled"), ("PULSO_LLM_GATEWAY_ADDR", "127.0.0.1:1"), ("PULSO_LLM_GATEWAY_KEY", "k"), ("PULSO_LLM_GATEWAY_STRUCTURED", "magic")])).is_err());
}

#[test]
fn every_call_keeps_its_usage_even_when_the_gateway_rejected_the_answer() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, generated(json!({"verdict": "agree"}), "xiaomi/mimo-v2.6-flash"));
    let rec = Recording::new(Rc::new(on(&f.addr, &[])));
    rec.call(&req()).unwrap();
    let c = &rec.calls()[0];
    let u = c.usage.as_ref().expect("usage of an answered call");
    assert_eq!((u.tokens_in, u.tokens_out, u.cost_usd.as_str()), (100, 20, "0.000100"));
    assert_eq!(c.to_json()["cost_usd"], "0.000100");
    let f = fake(502, json!({"error": {"kind": "invalid_output", "message": "x", "model": "m", "tokens_in": 300, "tokens_out": 50, "cost_usd": "0.000250"}}));
    let rec = Recording::new(Rc::new(on(&f.addr, &[])));
    assert!(rec.call(&req()).is_err());
    let u = rec.calls()[0].usage.clone().expect("the provider charged for the rejected answer too");
    assert_eq!((u.tokens_in, u.tokens_out, u.cost_usd.as_str()), (300, 50, "0.000250"));
}

#[test]
fn every_gateway_call_of_a_finding_carries_the_story_traceparent_and_baggage_and_nothing_changes_in_the_path() {
    use core_client::trace::{self, TraceCtx};
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fake(200, generated(json!({"verdict": "agree"}), "xiaomi/mimo-v2.6-flash"));
    let port = on(&f.addr, &[]);
    // outside a story scope: no trace headers at all
    port.call(&req()).unwrap();
    {
        let _s = trace::enter(TraceCtx { finding_key: "ev_3f9a1c07d2b84e51".into(), run_id: "value-loop-trg-20261005-0001".into(), release: "rel 1".into(), agent: "pulso-scout".into(), locale: "es".into(), case_type: "prompt".into() });
        trace::set_stage("scout", 1);
        port.call(&req()).unwrap();
        trace::set_stage("scout", 2);
        port.call(&req()).unwrap();
    }
    let seen = f.seen.lock().unwrap();
    assert!(!seen[0].1.contains_key("traceparent") && !seen[0].1.contains_key("baggage"));
    assert_eq!(seen[1].1["traceparent"], "00-7e1ffba44834058839ef1a914c474128-69dba9a106f229ee-01", "the stage span of the python vectors");
    assert_eq!(seen[1].1["baggage"], "session.id=value-loop-trg-20261005-0001,release=rel%201,langfuse.trace.tags=agent%3Apulso-scout%2Clocale%3Aes%2Ccase-type%3Aprompt%2Cstage%3Ascout");
    assert_ne!(seen[1].1["traceparent"], seen[2].1["traceparent"], "a retry is another attempt of the stage");
    assert!(seen[2].1["traceparent"].starts_with("00-7e1ffba44834058839ef1a914c474128-"));
    for s in &seen[..] {
        assert!(s.0.starts_with("POST /v1/generate HTTP/1.1"), "{}", s.0);
        assert_eq!(s.1["authorization"], "Bearer k-test-secret");
    }
}

#[test]
fn a_recorded_call_becomes_a_model_call_record_with_content_tokens_price_table_cost_and_story_ids_and_no_credential() {
    use core_client::trace::{self, TraceCtx};
    use engine::models::record::call_record;
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let leak = "answer with Bearer abcDEF123456 and sk-live1234567890 inside";
    let f = fake(200, generated(json!(leak), "xiaomi/mimo-v2.6-flash"));
    let rec = Recording::new(Rc::new(on(&f.addr, &[("PULSO_LLM_GATEWAY_STRUCTURED", "text")])));
    let _s = trace::enter(TraceCtx { finding_key: "ev_3f9a1c07d2b84e51".into(), run_id: "value-loop-trg-20261005-0001".into(), ..Default::default() });
    trace::set_stage("scout", 1);
    let r = rec.call(&req());
    assert!(r.is_err(), "the text is not JSON: the call is invalid but its content is still recorded");
    let c = &rec.calls()[0];
    let ctx = trace::current();
    let v = call_record(c, 1, ctx.as_ref());
    assert_eq!(v["schema"], "pulso.model_call/1");
    assert_eq!((v["role"].as_str(), v["model_id"].as_str(), v["tier"].as_str(), v["stage"].as_str(), v["attempt"].as_u64(), v["retries"].as_u64()), (Some("scout"), Some("xiaomi/mimo-v2.6-flash"), Some("flash"), Some("scout"), Some(1), Some(0)));
    assert_eq!((v["tokens_in"].as_u64(), v["tokens_out"].as_u64()), (Some(100), Some(20)));
    assert_eq!((v["cost_usd"].as_f64(), v["cost_source"].as_str()), (Some(1.96e-5), Some("price_table")));
    assert_eq!(v["outcome"], "invalid");
    assert_eq!(v["request"]["messages"][0], json!({"role": "system", "content": "stage prompt"}));
    assert!(v["request"]["messages"][1]["content"].as_str().unwrap().contains("output_schema"), "the treated payload is the user message");
    let resp = v["response"].as_str().unwrap();
    assert!(resp.contains("answer with") && !resp.contains("abcDEF123456") && !resp.contains("sk-live"), "{resp}");
    assert!(!v.to_string().contains("k-test-secret"), "the gateway key is never part of a record");
    assert_eq!(v["trace_id"], "7e1ffba44834058839ef1a914c474128");
    assert_eq!(v["parent_span_id"], "69dba9a106f229ee", "the scout stage span of the python vectors");
    assert_eq!(v["span_id"], trace::generation_span_id("7e1ffba44834058839ef1a914c474128", "scout", 1));
    assert!(v["started_at"].as_str().unwrap().ends_with('Z') && v["duration_ms"].is_u64());
    assert_eq!(v["truncated"], false);
}
