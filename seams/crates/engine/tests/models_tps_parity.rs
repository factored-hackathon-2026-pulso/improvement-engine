//! Parity of the Rust TPS with the Python scanner (roleplay-llm) on a shared corpus. Expected verdicts were computed by the
//! Python scanner (tests/fixtures/gen_tps_parity.py). The Rust scanner may be STRICTER (it rejects what Python rejects and
//! more) but it must never accept a payload the Python scanner rejects.
use engine::models::tps::{DEFAULT_K, scan_payload};
use serde_json::Value;

#[test]
fn rust_tps_never_accepts_what_the_python_scanner_rejects() {
    let doc: Value = serde_json::from_str(include_str!("fixtures/tps_parity.json")).unwrap();
    let reg: Vec<String> = doc["registry"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let (mut looser, mut stricter) = (vec![], vec![]);
    for c in doc["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let py_ok = c["ok"].as_bool().unwrap();
        let rs_ok = scan_payload(&c["payload"], DEFAULT_K, &reg).ok;
        if rs_ok && !py_ok {
            looser.push(name.to_string());
        } else if !rs_ok && py_ok && c["payload"].to_string().is_ascii() {
            // documented difference: without NFKC the Rust TPS also rejects compatibility characters outright (non-ASCII only)
            stricter.push(name.to_string());
        }
    }
    assert!(looser.is_empty(), "Rust TPS accepts what Python rejects: {looser:#?}");
    assert!(stricter.is_empty(), "Rust TPS rejects what Python accepts on ASCII input: {stricter:#?}");
}
