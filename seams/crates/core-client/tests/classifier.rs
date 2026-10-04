use core_client::errors::{self, Disposition};
use serde_json::Value;
use std::path::PathBuf;

fn contract() -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../bridge-contract/contract.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn classifier_covers_every_wire_error_of_bridge_contract() {
    let c = contract();
    let wire = c["error_codes"]["wire"].as_object().unwrap();
    assert!(wire.len() >= 55);
    for (code, spec) in wire {
        let k = errors::classify(code).unwrap_or_else(|| panic!("unclassified {code}"));
        assert_eq!(k.retryable, spec["retryable"].as_bool().unwrap(), "{code} retryable");
        let statuses: Vec<u64> = spec["status"].as_array().unwrap().iter().map(|s| s.as_u64().unwrap()).collect();
        assert_eq!(k.statuses.iter().map(|s| *s as u64).collect::<Vec<_>>(), statuses, "{code} statuses");
        assert_eq!(k.disposition == Disposition::Retry, k.retryable, "{code}");
    }
    // no stale entries: the table is exactly the contract list
    assert_eq!(errors::TABLE.len(), wire.len());
    for e in errors::TABLE {
        assert!(wire.contains_key(e.code), "stale classifier entry {}", e.code);
    }
}

#[test]
fn auth_reasons_cover_the_contract_and_map_to_statuses() {
    let c = contract();
    let reasons = c["auth"]["reasons"].as_array().unwrap();
    let map = c["auth"]["reason_status"].as_object().unwrap();
    for r in reasons {
        let r = r.as_str().unwrap();
        let want = map.get(r).or(map.get("*")).unwrap().as_u64().unwrap() as u16;
        assert_eq!(errors::auth_reason_status(r), Some(want), "{r}");
    }
    assert_eq!(errors::AUTH_REASONS.len(), reasons.len());
}

#[test]
fn unknown_code_is_unclassified_and_status_fallback_applies() {
    assert!(errors::classify("pulso:made_up").is_none());
    assert_eq!(errors::disposition_for_status(503), Disposition::Retry);
    assert_eq!(errors::disposition_for_status(429), Disposition::Retry);
    assert_eq!(errors::disposition_for_status(422), Disposition::Terminal);
}

#[test]
fn parse_envelope_extracts_code_and_reason() {
    let body = br#"{"schema_version":"1","code":"pulso:auth_denied","retryable":false,"trace_id":"t","details":{"reason":"purpose_denied"}}"#;
    let e = errors::parse_envelope(403, body).unwrap();
    assert_eq!(e.code, "pulso:auth_denied");
    assert_eq!(e.reason.as_deref(), Some("purpose_denied"));
    assert!(!e.retryable);
    assert!(errors::parse_envelope(500, b"<html>").is_none());
}

#[test]
fn unknown_code_fails_closed_even_if_wire_says_retryable() {
    let body = br#"{"code":"pulso:made_up","retryable":true,"details":{}}"#;
    let e = errors::parse_envelope(500, body).unwrap();
    assert!(!e.known);
    assert_eq!(e.disposition(), Disposition::Terminal);
}
