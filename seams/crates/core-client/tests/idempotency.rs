mod common;
use common::*;
use core_client::{CallError, ClientConfig, CoreClient, routes};
use serde_json::json;

fn client(f: &FakeCore) -> CoreClient {
    let mut cfg = ClientConfig::new(&f.addr, KID, SEED, "bridge-1");
    cfg.timeout = std::time::Duration::from_secs(5);
    CoreClient::new(cfg)
}

#[test]
fn same_idempotency_key_twice_makes_one_run() {
    let f = FakeCore::start();
    let c = client(&f);
    let body = json!({"stage":"scout","job_id":"j1"});
    let a = c.call(&routes::INVOKE, "tenant-a", Some("j1"), &[], Some(&body), Some("key-1")).unwrap();
    let b = c.call(&routes::INVOKE, "tenant-a", Some("j1"), &[], Some(&body), Some("key-1")).unwrap();
    assert_eq!(f.runs(), 1);
    assert_eq!(a.body["run_id"], b.body["run_id"]);
    let reqs = f.requests();
    assert_eq!(reqs[0].headers["idempotency-key"], "key-1");
    assert_eq!(reqs[1].headers["idempotency-key"], "key-1");
    // every HTTP attempt uses a fresh jti
    assert_ne!(reqs[0].claims.as_ref().unwrap()["jti"], reqs[1].claims.as_ref().unwrap()["jti"]);
}

#[test]
fn different_body_same_key_is_a_digest_conflict() {
    let f = FakeCore::start();
    let c = client(&f);
    c.call(&routes::INVOKE, "t", Some("j"), &[], Some(&json!({"a":1})), Some("k")).unwrap();
    let e = c.call(&routes::INVOKE, "t", Some("j"), &[], Some(&json!({"a":2})), Some("k")).unwrap_err();
    match e {
        CallError::Api(a) => assert_eq!(a.code, "pulso:digest_conflict"),
        o => panic!("{o:?}"),
    }
    assert_eq!(f.runs(), 1);
}

#[test]
fn missing_key_on_mandatory_route_is_refused_before_sending() {
    let f = FakeCore::start();
    let c = client(&f);
    let e = c.call(&routes::RUN_ARM, "t", None, &[], Some(&json!({})), None).unwrap_err();
    assert!(matches!(e, CallError::MissingIdempotencyKey), "{e:?}");
    assert!(f.requests().is_empty());
}

#[test]
fn retry_after_503_reuses_key_and_still_one_run() {
    let f = FakeCore::start();
    f.script(503, envelope("pulso:core_unavailable", true));
    let c = client(&f);
    let body = json!({"x":1});
    let out = c.call_with_retry(&routes::INVOKE, "t", Some("j"), &[], Some(&body), Some("k9"), 3).unwrap();
    assert_eq!(out.status, 200);
    assert_eq!(f.runs(), 1);
    let reqs = f.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[1].headers["idempotency-key"], "k9");
}

#[test]
fn non_retryable_error_is_not_retried() {
    let f = FakeCore::start();
    f.script(409, envelope("pulso:digest_conflict", false));
    let c = client(&f);
    let e = c.call_with_retry(&routes::INVOKE, "t", Some("j"), &[], Some(&json!({})), Some("k"), 3).unwrap_err();
    assert!(matches!(e, CallError::Api(_)));
    assert_eq!(f.requests().len(), 1);
}

#[test]
fn jwt_carries_worker_sub_purpose_audience_and_short_ttl() {
    let f = FakeCore::start();
    let c = client(&f);
    let r = c.call(&routes::VERSION, "", None, &[], None, None).unwrap();
    assert_eq!(r.body["contracts_version"], core_client::pins::CONTRACTS_VERSION);
    let claims = f.requests()[0].claims.clone().unwrap();
    assert_eq!(claims["sub"], "worker:bridge-1");
    assert_eq!(claims["aud"], "core-bridge");
    assert_eq!(claims["iss"], "control-api");
    assert_eq!(claims["purpose"], "version_probe");
    assert!(claims.get("tenant_id").is_none());
    assert!(claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap() <= 300);
}

#[test]
fn path_params_are_substituted_and_job_id_claim_is_set() {
    let f = FakeCore::start();
    f.script(200, json!({}));
    let c = client(&f);
    c.call(&routes::READ_ARM_BY_KEY, "t", None, &["a b"], None, None).unwrap();
    let r = &f.requests()[0];
    assert_eq!(r.path, "/internal/v1/evaluation/arms/by-key/a%20b");
    c.call(&routes::INVOKE, "t", Some("job-7"), &[], Some(&json!({})), Some("k")).unwrap();
    assert_eq!(f.requests()[1].claims.as_ref().unwrap()["job_id"], "job-7");
}

#[test]
fn connect_failure_is_not_sent() {
    // Nothing listens: connect failure means the request was never sent.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    drop(l);
    let c = CoreClient::new(ClientConfig::new(&addr, KID, SEED, "w"));
    let e = c.call(&routes::VERSION, "", None, &[], None, None).unwrap_err();
    match e {
        CallError::Transport { sent, .. } => assert!(!sent),
        o => panic!("{o:?}"),
    }
}
