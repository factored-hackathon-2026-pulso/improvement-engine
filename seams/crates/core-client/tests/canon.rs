//! Wire formats and digests of the bridge contract: JCS (RFC 8785), lowercase hex, `evc-` refs, key derivations.
//! Reference vectors were computed with the Python implementations (hashlib + sorted compact JSON) and checked
//! against the concrete values in the bridge goldens.
use core_client::canon::{self, CanonError};
use serde_json::json;

#[test]
fn jcs_sorts_keys_by_utf16_code_units_and_escapes_minimally() {
    assert_eq!(canon::jcs(&json!({"b":1,"a":2})).unwrap(), r#"{"a":2,"b":1}"#);
    // U+10000 is the surrogate pair D800 DC00 (< U+FFFF in UTF-16) although it sorts after U+FFFF in UTF-8.
    let v = json!({"\u{ffff}":1,"\u{10000}":2});
    assert_eq!(canon::jcs(&v).unwrap(), "{\"\u{10000}\":2,\"\u{ffff}\":1}");
    let got = canon::jcs(&json!(["é", "\n\u{1}", "\"\\", null, true, -7])).unwrap();
    assert_eq!(got, r#"["é","\n\u0001","\"\\",null,true,-7]"#);
}

#[test]
fn jcs_refuses_non_integer_numbers() {
    assert!(matches!(canon::jcs(&json!({"x": 1.5})), Err(CanonError::NonIntegerNumber)));
}

#[test]
fn request_digest_excludes_digest_credentials_and_trace() {
    let body = json!({"a":1,"request_digest":"x","credentials":{"k":1},"trace":{"t":1},"z":[1,"é","\n\u{1}"],"b":null,"c":true});
    assert_eq!(canon::request_digest(&body).unwrap(), "e3924a1fb93a1e91c3059b78491d83b0dc14aeeb28bb9240d5d4d8e3c430b9a1");
}

#[test]
fn digest_json_matches_the_admission_vector() {
    let adm = json!({"schema_version":"1","binding_ref":"b","proposal_id":"p","candidate_hash":"c","suite_id":"s",
        "suite_version":"1","suite_digest":"d","evaluation_attempt":1,"budget_ref":"bud"});
    assert_eq!(canon::digest_json(&adm).unwrap(), "5d23c57ce4aa3b39142668552616e78bbcc524263a9e22e8b6fb777040049de1");
}

#[test]
fn invoke_idempotency_key_and_binding_ref_reproduce_the_golden() {
    // invoke_scout golden: Idempotency-Key header and receipt.task_binding_ref are concrete values.
    let key = canon::idempotency_key("t1", "job-golden-scout", "scout", 1, "golden-scout-1").unwrap();
    assert_eq!(key, "dbe68ddd4321eac7f2d87367e6782484a65f7a1477172556517de1df4846753f");
    assert_eq!(canon::task_binding_ref("t1", &key).unwrap(), "0fa2a7a4c8a8b2021db9cbba93f986292136c6ae602e1aa64c3ebd38cb49b6bd");
}

#[test]
fn arm_execution_id_is_arm_plus_32_hex() {
    assert_eq!(canon::arm_execution_id("t1", "golden-arm-1").unwrap(), "arm-707546950d0f793fd8f9bb7fda133396");
}

#[test]
fn evaluation_context_ref_is_evc_plus_40_hex() {
    let r = canon::evaluation_context_ref("t1", "job-1", "bind-1", "prop-1", "cand-1", 2).unwrap();
    assert_eq!(r, "evc-aca184b8448c9dcd15af33243474160e9120f134");
    assert_eq!(r.len(), 4 + 40);
}

#[test]
fn a_pipe_in_any_component_is_rejected() {
    assert!(matches!(canon::idempotency_key("t|1", "j", "scout", 1, "k"), Err(CanonError::PipeInComponent(_))));
    assert!(matches!(canon::idempotency_key("t", "j", "scout", 1, "k|x"), Err(CanonError::PipeInComponent(_))));
    assert!(matches!(canon::task_binding_ref("t", "a|b"), Err(CanonError::PipeInComponent(_))));
    assert!(matches!(canon::arm_execution_id("t|", "k"), Err(CanonError::PipeInComponent(_))));
    assert!(matches!(canon::evaluation_context_ref("t", "j", "b", "p|q", "c", 1), Err(CanonError::PipeInComponent(_))));
    assert!(matches!(canon::evaluation_context_ref("t", "j", "b|", "p", "c", 1), Err(CanonError::PipeInComponent(_))));
}

#[test]
fn z_timestamps_are_utc_rfc3339_with_a_literal_z() {
    assert_eq!(canon::z_timestamp(0), "1970-01-01T00:00:00Z");
    assert_eq!(canon::z_timestamp(1_791_072_000), "2026-10-04T00:00:00Z");
    assert_eq!(canon::z_timestamp(951_782_400 + 86_399), "2000-02-29T23:59:59Z");
    for ok in ["2026-10-04T00:00:00Z", "2026-10-04T00:00:00.123456789Z"] {
        assert!(canon::is_z_timestamp(ok), "{ok}");
    }
    for bad in ["2026-10-04T00:00:00", "2026-10-04T00:00:00+00:00", "2026-10-04 00:00:00Z", "2026-10-04T00:00:00z", "2026-10-04T00:00Z"] {
        assert!(!canon::is_z_timestamp(bad), "{bad}");
    }
}

#[test]
fn integers_outside_the_json_safe_range_are_refused_like_rfc8785_python() {
    // rfc8785.dumps raises for |n| > 2^53-1; a digest the bridge cannot reproduce must never be computed.
    assert!(canon::jcs(&json!({"n": 9_007_199_254_740_991_i64})).is_ok());
    assert!(canon::jcs(&json!({"n": -9_007_199_254_740_991_i64})).is_ok());
    for n in [json!(9_007_199_254_740_992_i64), json!(-9_007_199_254_740_992_i64), json!(u64::MAX)] {
        assert!(matches!(canon::jcs(&json!({"n": n})), Err(CanonError::NonIntegerNumber)), "{n}");
    }
}

#[test]
fn z_timestamp_validation_matches_python_fromisoformat() {
    for bad in ["2026-02-30T00:00:00Z", "2026-04-31T00:00:00Z", "2026-10-04T00:00:60Z", "2026-00-10T00:00:00Z", "2026-10-00T00:00:00Z", "2025-02-29T00:00:00Z"] {
        assert!(!canon::is_z_timestamp(bad), "{bad}");
    }
    for ok in ["2024-02-29T23:59:59Z", "2026-12-31T23:59:59.5Z"] {
        assert!(canon::is_z_timestamp(ok), "{ok}");
    }
}
