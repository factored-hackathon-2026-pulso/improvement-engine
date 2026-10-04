//! TPS (treated-payload scan) in Rust: deny-by-default, mirrors roleplay-llm/roleplay_llm/scanner.py.
use engine::models::tps::{DEFAULT_K, scan_payload, suppress_rows};
use serde_json::{Value, json};

fn base() -> Value {
    json!({"goal": "propose hypotheses for one signal", "inputs": {"signal_id": "sig-0001", "metric_id": "synthetic_metric"}, "step": 0, "tools": [], "observations": []})
}

fn scan(p: &Value) -> Vec<String> {
    scan_payload(p, DEFAULT_K, &["sig-0001".to_string()]).violations
}

fn has(v: &[String], needle: &str) -> bool {
    v.iter().any(|x| x.contains(needle))
}

#[test]
fn a_treated_payload_with_registered_ids_passes() {
    assert_eq!(scan(&base()), Vec::<String>::new());
}

#[test]
fn unknown_top_level_key_and_missing_required_keys_are_violations() {
    let mut p = base();
    p["raw_text"] = json!("hola");
    assert!(has(&scan(&p), "payload.raw_text: key not allowed"));
    let v = scan(&json!({"inputs": {}}));
    for k in ["goal", "step", "observations"] {
        assert!(has(&v, &format!("payload.{k}: missing")), "{v:?}");
    }
    assert!(has(&scan(&json!([1])), "not an object"));
}

#[test]
fn free_text_in_inputs_is_rejected_even_when_it_looks_like_an_id() {
    let mut p = base();
    p["inputs"]["note"] = json!("maria.perez@example.com");
    p["inputs"]["comment"] = json!("the customer was angry");
    p["inputs"]["token"] = json!("looks-like-an-id-but-unregistered");
    let v = scan(&p);
    assert!(has(&v, "inputs.note: string is not an opaque id or enum"), "{v:?}");
    assert!(has(&v, "inputs.comment: string is not an opaque id or enum"), "{v:?}");
    assert!(has(&v, "inputs.token: string is not an expected id or registered token"), "{v:?}");
}

#[test]
fn system_issued_id_shapes_pass_without_registration() {
    let mut p = base();
    p["inputs"] = json!({"a": "g_0123456789abcdef", "b": "ev_0123456789abcdef", "c": "123e4567-e89b-12d3-a456-426614174000", "d": "job-0123456789", "e": "hyp_12", "f": "w_2026_06", "g": "w12"});
    assert_eq!(scan(&p), Vec::<String>::new());
    p["inputs"] = json!({"a": "g_0123456789ABCDEF", "b": "ev_xyz", "c": "hyp_12345", "d": "job-abc"});
    assert_eq!(scan(&p).len(), 4, "{:?}", scan(&p));
}

#[test]
fn pii_in_static_text_is_rejected_and_big_free_integers_too() {
    let mut p = base();
    p["goal"] = json!("call 600 123 4567 or write a@b.co");
    assert!(has(&scan(&p), "goal: contains a PII-like pattern"));
    p["goal"] = json!("fine");
    p["inputs"]["n"] = json!(123456789);
    assert!(has(&scan(&p), "inputs.n: free-form integer beyond 1000"));
    p["inputs"]["n"] = json!(5);
    p["goal"] = json!("x".repeat(2001));
    assert!(has(&scan(&p), "goal: longer than 2000"));
    p["goal"] = json!("\u{ff11}\u{ff12}\u{ff13}\u{ff14}\u{ff15}\u{ff16}\u{ff17}\u{ff18}");
    assert!(has(&scan(&p), "goal: contains a PII-like pattern"), "non-ascii digits must not bypass");
}

#[test]
fn observation_rows_need_k_anonymity_and_known_ids() {
    let mut p = base();
    p["tools"] = json!([{"tool": "pulso/lab_query@1.0.0", "description": "aggregate lab query", "args_schema": {"type": "object"}}]);
    p["observations"] = json!([{"tool": "pulso/lab_query@1.0.0", "args": {"metric_id": "recurrence_rate"}, "status": "ok", "error": null,
        "result": {"rows": [{"metric_id": "recurrence_rate", "window_id": "w1", "count": 40, "rate": 0.25, "evidence_ref": "ev_0123456789abcdef", "g_route": "0123456789abcdef"}]}}]);
    assert_eq!(scan(&p), Vec::<String>::new());
    p["observations"][0]["result"]["rows"][0]["count"] = json!(3);
    assert!(has(&scan(&p), "count: must be an integer >= k=10"), "{:?}", scan(&p));
    p["observations"][0]["result"]["rows"][0] = json!({"metric_id": "recurrence_rate", "window_id": "w1", "count": "<k"});
    assert_eq!(scan(&p), Vec::<String>::new());
    p["observations"][0]["result"]["rows"][0] = json!({"metric_id": "recurrence_rate", "window_id": "w1", "count": "<k", "rate": 0.1});
    assert!(has(&scan(&p), "suppressed row carries"));
    p["observations"][0]["result"]["rows"][0] = json!({"metric_id": "recurrence_rate", "window_id": "w1", "count": 40, "rate": 0.255});
    assert!(has(&scan(&p), "rate: must be a number rounded to 2 decimals"));
    p["observations"][0]["result"]["rows"][0] = json!({"metric_id": "recurrence_rate", "window_id": "w1", "count": 40, "customer_name": "x"});
    assert!(has(&scan(&p), "field not allowed"));
}

#[test]
fn suppress_rows_drops_everything_but_enums_below_k() {
    let rows = vec![json!({"metric_id": "recurrence_rate", "window_id": "w1", "count": 3, "rate": 0.5, "evidence_ref": "ev_0123456789abcdef"}), json!({"metric_id": "recurrence_rate", "window_id": "w1", "count": 30, "rate": 0.5})];
    let out = suppress_rows(&rows, DEFAULT_K);
    assert_eq!(out[0], json!({"metric_id": "recurrence_rate", "window_id": "w1", "count": "<k"}));
    assert_eq!(out[1], rows[1]);
}
