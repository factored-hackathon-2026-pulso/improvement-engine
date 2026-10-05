//! Roleplay port: replay-only client of the roleplay-llm queue (`responses/<key>.json`); never writes a request.
use engine::models::roleplay::{Roleplay, replay_key};
use engine::models::{DataClass, Label, ModelError, ModelPort, ModelRequest, Role};
use serde_json::{Value, json};
use std::path::PathBuf;

fn queue(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("responses")).unwrap();
    d
}

fn payload() -> Value {
    json!({"goal": "g", "inputs": {"signal_id": "sig-0001"}, "step": 0, "tools": [], "observations": []})
}

fn req(role: Role) -> ModelRequest {
    ModelRequest { role, system: "stage prompt".into(), payload: payload(), registry: vec!["sig-0001".into()], data_class: DataClass::Synthetic }
}

fn respond(q: &PathBuf, key: &str, doc: Value) {
    std::fs::write(q.join("responses").join(format!("{key}.json")), doc.to_string()).unwrap();
}

fn good(key: &str, role: &str, output: Value) -> Value {
    json!({"protocol": "roleplay-queue/1", "key": key, "provenance": "agent_roleplay", "quality_claims": "forbidden",
           "responder": {"id": "resp-a", "role": role}, "content": {"kind": "final", "output": output}})
}

#[test]
fn replay_key_equals_the_python_shim_key() {
    // computed with roleplay_llm.shim.replay_key("stage prompt", ...) on 2026-10-04
    let mut with_volatile = payload();
    with_volatile["run_id"] = json!("r1");
    assert_eq!(replay_key("stage prompt", &with_volatile).unwrap(), "1114790a2c04284f8b465ab1348df89a");
    let p = json!({"goal": "g", "inputs": {"signal_id": "sig-0001", "rate": 0.3, "n": 5}, "step": 0, "tools": [], "observations": []});
    assert_eq!(replay_key("stage prompt", &p).unwrap(), "abdf93f94c935df26f3ba7289ccb5a7f");
}

#[test]
fn a_recorded_answer_is_replayed_and_labelled_roleplay() {
    let q = queue("hit");
    let key = replay_key("stage prompt", &payload()).unwrap();
    respond(&q, &key, good(&key, "scout", json!({"hypotheses": [{"id": "h_1", "signal_id": "sig-0001", "claimed_rate": 0.3}]})));
    let a = Roleplay::new(&q).call(&req(Role::Scout)).unwrap();
    assert_eq!(a.label, Label::Roleplay);
    assert_eq!(a.model_id, "agent_roleplay:resp-a");
    assert_eq!(a.content["hypotheses"][0]["claimed_rate"], 0.3);
    assert!(!q.join("requests").exists(), "replay only: nothing is ever queued");
}

#[test]
fn a_miss_is_unavailable_and_never_enqueues() {
    let q = queue("miss");
    let e = Roleplay::new(&q).call(&req(Role::Scout)).unwrap_err();
    assert!(matches!(&e, ModelError::Unavailable(w) if w.starts_with("replay_miss:")), "{e:?}");
    assert!(!q.join("requests").exists());
}

#[test]
fn e0_and_original_payloads_and_untreated_payloads_are_refused_before_the_queue_is_read() {
    let q = queue("refuse");
    for dc in [DataClass::E0, DataClass::Original] {
        let r = ModelRequest { data_class: dc, ..req(Role::Scout) };
        assert!(matches!(Roleplay::new(&q).call(&r), Err(ModelError::Refused(w)) if w.contains("data_class")), "{dc:?}");
    }
    let mut r = req(Role::Scout);
    r.payload["inputs"]["comment"] = json!("the customer called me");
    assert!(matches!(Roleplay::new(&q).call(&r), Err(ModelError::Refused(w)) if w.starts_with("tps:")));
}

#[test]
fn a_response_with_extra_fields_a_wrong_label_a_wrong_role_or_a_quality_claim_is_invalid() {
    let q = queue("bad");
    let key = replay_key("stage prompt", &payload()).unwrap();
    let mut cases: Vec<(&str, Value)> = vec![];
    let mut d = good(&key, "scout", json!({"x": 1}));
    d["extra"] = json!(1);
    cases.push(("extra field", d));
    let mut d = good(&key, "scout", json!({"x": 1}));
    d["provenance"] = json!("real_model");
    cases.push(("label", d));
    cases.push(("role", good(&key, "verifier", json!({"x": 1}))));
    cases.push(("quality", good(&key, "scout", json!({"nested": {"confidence": 0.9}}))));
    let mut d = good(&key, "scout", json!({"x": 1}));
    d["key"] = json!("other");
    cases.push(("key", d));
    let mut d = good(&key, "scout", json!({"x": 1}));
    d["content"] = json!({"kind": "tool_call", "tool": "pulso/lab_query@1.0.0", "args": {}});
    cases.push(("not final", d));
    for (name, doc) in cases {
        respond(&q, &key, doc);
        assert!(matches!(Roleplay::new(&q).call(&req(Role::Scout)), Err(ModelError::Invalid(_))), "{name}");
    }
}

#[test]
fn ids_the_python_key_normalises_are_refused_instead_of_guessed() {
    let q = queue("ids");
    let mut r = req(Role::Scout);
    r.payload["inputs"]["binding_ref"] = json!("binding-0123456789abcdef");
    assert!(matches!(Roleplay::new(&q).call(&r), Err(ModelError::Refused(w)) if w.starts_with("replay_key_unsupported_ids")));
}
