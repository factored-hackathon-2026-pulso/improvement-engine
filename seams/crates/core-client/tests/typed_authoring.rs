//! Typed dry-run, alias read and credential issue against the generated golden FakeCore (authoring, credentials).
mod common;
use common::golden::GoldenCore;
use common::{FakeCore, KID, SEED};
use core_client::dto::{Alias, Change, CredentialRequest, DryRunRequest};
use core_client::{CallError, ClientConfig, CoreClient, OpError};
use serde_json::json;

fn client(addr: &str) -> CoreClient {
    let mut cfg = ClientConfig::new(addr, KID, SEED, "bridge-1");
    cfg.timeout = std::time::Duration::from_secs(5);
    cfg.accept_golden_placeholders = true;
    CoreClient::new(cfg)
}

fn code_of(e: OpError) -> String {
    match e {
        OpError::Call(CallError::Api(a)) => a.code,
        e => panic!("{e:?}"),
    }
}

fn dr(base: Option<&str>) -> DryRunRequest {
    DryRunRequest::new("t1", "atencion", base, vec![])
}

// ---- alias read ------------------------------------------------------------------------------------------------

#[test]
fn alias_read_decodes_the_golden() {
    let g = GoldenCore::play(&[("authoring", "alias_read")]);
    let a = client(&g.addr).read_alias("t1", Some("job-golden"), "atencion", Alias::Prod).unwrap();
    g.finish();
    assert_eq!((a.agent_id.as_str(), a.alias), ("atencion", Alias::Prod));
    assert_eq!(a.release_id.as_deref(), Some("rel-demo"));
    assert_eq!(a.status.as_deref(), Some("active"));
    assert_eq!(a.runtime_profile.as_deref(), Some("agent_core_real"));
}

#[test]
fn unknown_agent_is_alias_unknown_and_only_prod_staging_exist() {
    let g = GoldenCore::play(&[("authoring", "alias_unknown_agent")]);
    let e = client(&g.addr).read_alias("t1", Some("job-golden"), "no-such-agent", Alias::Prod).unwrap_err();
    g.finish();
    assert_eq!(code_of(e), "pulso:alias_unknown");
    assert_eq!(Alias::parse("staging"), Some(Alias::Staging));
    assert_eq!(Alias::parse("canary"), None);
}

#[test]
fn an_alias_answer_for_another_agent_is_a_contract_violation() {
    let f = FakeCore::start();
    f.script(200, json!({"schema_version":"1","agent_id":"other","alias":"prod","release_id":null}));
    assert!(matches!(client(&f.addr).read_alias("t1", None, "atencion", Alias::Prod), Err(OpError::Contract(_))));
}

#[test]
fn an_alias_without_a_release_decodes_as_none() {
    let f = FakeCore::start();
    f.script(200, json!({"schema_version":"1","agent_id":"atencion","alias":"staging","release_id":null}));
    let a = client(&f.addr).read_alias("t1", None, "atencion", Alias::Staging).unwrap();
    assert_eq!(a.release_id, None);
    assert_eq!(f.requests()[0].path, "/internal/v1/core-state/aliases/atencion/staging");
}

// ---- dry-run ----------------------------------------------------------------------------------------------------

#[test]
fn dry_run_encodes_the_golden_request_and_decodes_a_valid_result() {
    let g = GoldenCore::play(&[("authoring", "dry_run_valid")]);
    let r = client(&g.addr).dry_run_raw("job-golden", &dr(Some("rel-demo"))).unwrap();
    g.finish();
    assert!(r.is_valid());
    assert_eq!(r.proposal_created, Some(false));
    assert_eq!(r.release_id_preview.as_deref(), Some("rel-d5a4da2840e74f4b"));
    assert!(r.content_hashes.contains_key("agent:atencion@1.0.0"));
}

#[test]
fn valid_false_is_decoded_and_never_success() {
    let g = GoldenCore::play(&[("authoring", "dry_run_violations")]);
    let req = DryRunRequest::new("t1", "atencion", None, vec![Change::new("no-such-kind", json!({}), json!({}))]);
    let r = client(&g.addr).dry_run_raw("job-golden", &req).unwrap();
    g.finish();
    assert!(!r.is_valid());
    assert_eq!(r.candidate_hash, None);
    assert_eq!(r.violations.len(), 1);
    assert_eq!((r.violations[0].rule.as_str(), r.violations[0].path.as_deref()), ("REG-SCHEMA", Some("changes/0")));
}

#[test]
fn base_release_unknown_and_tenant_mismatch_are_errors() {
    let g = GoldenCore::play(&[("authoring", "dry_run_base_release_unknown"), ("authoring", "dry_run_tenant_mismatch")]);
    let c = client(&g.addr);
    assert_eq!(code_of(c.dry_run_raw("job-golden", &dr(Some("rel-nope"))).unwrap_err()), "pulso:base_release_unknown");
    // body tenant != JWT tenant is only reachable through the explicit diagnostic entry point
    let e = c.dry_run_as_tenant("t1", "job-golden", &DryRunRequest::new("t2", "atencion", None, vec![])).unwrap_err();
    assert_eq!(code_of(e), "pulso:tenant_mismatch");
    g.finish();
}

#[test]
fn base_release_null_is_always_sent() {
    let b = dr(None).to_json();
    assert!(b.as_object().unwrap().contains_key("base_release_id") && b["base_release_id"].is_null());
    assert_eq!((b["schema_version"].as_str(), b["changes"].as_array().map(Vec::len)), (Some("1"), Some(0)));
}

fn digest_of(req: &DryRunRequest) -> String {
    core_client::canon::request_digest(&req.to_json()).unwrap()
}

fn valid_result(req: &DryRunRequest, digest: &str, hash: &str) -> serde_json::Value {
    json!({"auto_bumped":[],"candidate_hash":hash,"content_hashes":{},"new_versions":[],"proposal_created":false,
        "release_hash":"e".repeat(64),"release_id_preview":format!("rel-{}", &hash[..16]),"request_digest":digest,
        "runtime_profile":"agent_core_real","schema_version":"1","valid":true,"violations":[],"_for":req.agent_id})
}

#[test]
fn a_valid_answer_must_be_bound_to_the_request_digest() {
    let f = FakeCore::start();
    let req = dr(Some("rel-demo"));
    let hash = "a".repeat(64);
    f.script(200, valid_result(&req, &digest_of(&req), &hash));
    let ok = client(&f.addr).dry_run("job-1", &req).unwrap();
    assert_eq!(ok.candidate_hash.as_deref(), Some(hash.as_str()));
    // an answer about some other request is rejected
    f.script(200, valid_result(&req, &"0".repeat(64), &hash));
    assert!(matches!(client(&f.addr).dry_run("job-1", &req), Err(OpError::Contract(m)) if m.contains("request_digest")));
}

#[test]
fn dry_run_that_reports_a_created_proposal_is_refused() {
    let f = FakeCore::start();
    let req = dr(Some("rel-demo"));
    let mut body = valid_result(&req, &digest_of(&req), &"a".repeat(64));
    body["proposal_created"] = json!(true);
    f.script(200, body);
    assert!(matches!(client(&f.addr).dry_run("job-1", &req), Err(OpError::Contract(m)) if m.contains("proposal_created")));
}

#[test]
fn a_valid_answer_needs_a_well_formed_candidate_hash() {
    let f = FakeCore::start();
    let req = dr(Some("rel-demo"));
    f.script(200, valid_result(&req, &digest_of(&req), "not-a-hash-0000000000000000"));
    assert!(matches!(client(&f.addr).dry_run("job-1", &req), Err(OpError::Contract(m)) if m.contains("candidate_hash")));
}

#[test]
fn a_sha256_prefixed_hash_is_normalised_to_bare_hex() {
    let f = FakeCore::start();
    let req = dr(None);
    f.script(200, valid_result(&req, &digest_of(&req), &format!("sha256:{}", "b".repeat(64))));
    assert_eq!(client(&f.addr).dry_run("job-1", &req).unwrap().candidate_hash.as_deref(), Some("b".repeat(64).as_str()));
}

#[test]
fn valid_false_is_refused_by_the_strict_call_with_its_violations() {
    let f = FakeCore::start();
    let req = dr(None);
    f.script(200, json!({"auto_bumped":[],"candidate_hash":null,"content_hashes":{},"new_versions":[],"proposal_created":false,
        "release_hash":null,"release_id_preview":null,"request_digest":digest_of(&req),"runtime_profile":"agent_core_real",
        "schema_version":"1","valid":false,"violations":[{"flow":null,"message":"bad","node_id":null,"path":"changes/0","rule":"REG-SCHEMA"}]}));
    match client(&f.addr).dry_run("job-1", &req) {
        Err(OpError::DryRunRefused(v)) => assert_eq!(v[0].rule, "REG-SCHEMA"),
        o => panic!("{o:?}"),
    }
}

#[test]
fn violations_with_valid_true_are_still_a_refusal() {
    let f = FakeCore::start();
    let req = dr(None);
    let mut body = valid_result(&req, &digest_of(&req), &"a".repeat(64));
    body["violations"] = json!([{"message":"m","rule":"REG-LIMIT"}]);
    f.script(200, body);
    assert!(matches!(client(&f.addr).dry_run("job-1", &req), Err(OpError::DryRunRefused(_))));
}

#[test]
fn dry_run_requests_are_validated_client_side() {
    let f = FakeCore::start();
    let c = client(&f.addr);
    let mut r = dr(None);
    r.agent_id = String::new();
    assert!(matches!(c.dry_run("j", &r), Err(OpError::Invalid(_))));
    let mut r = dr(None);
    r.changes = vec![Change::new("", json!({}), json!({}))];
    assert!(matches!(c.dry_run("j", &r), Err(OpError::Invalid(_))));
    let mut r = dr(None);
    r.changes = vec![Change::new("k", json!([]), json!({}))];
    assert!(matches!(c.dry_run("j", &r), Err(OpError::Invalid(_))));
    let mut r = dr(None);
    r.changes = vec![Change::new("k", json!({"x":1.5}), json!({}))];
    assert!(matches!(c.dry_run("j", &r), Err(OpError::Invalid(_)) | Err(OpError::Canon(_))));
    assert!(f.requests().is_empty());
}

// ---- credentials ------------------------------------------------------------------------------------------------

#[test]
fn credential_issue_decodes_and_never_prints_the_jws() {
    let g = GoldenCore::play(&[("credentials", "issue_core_task"), ("credentials", "issue_registry_write")]);
    let c = client(&g.addr);
    let a = c.issue_credential("job-golden", &CredentialRequest::new("t1", "constructor", "core_task")).unwrap();
    let b = c.issue_credential("job-golden", &CredentialRequest::new("t1", "constructor", "registry_write")).unwrap();
    g.finish();
    assert_eq!((a.kid.as_str(), b.kid.as_str()), ("id1", "st1"));
    assert_eq!(a.jws(), "<jws>");
    let shown = format!("{a:?}");
    assert!(!shown.contains("<jws>") && shown.contains("redacted"), "{shown}");
}

#[test]
fn credential_refusals() {
    let g = GoldenCore::play(&[("credentials", "issue_not_issuable"), ("credentials", "issue_tenant_mismatch")]);
    let c = client(&g.addr);
    assert_eq!(code_of(c.issue_credential("job-golden", &CredentialRequest::new("t1", "approver", "registry_write")).unwrap_err()), "pulso:credential_not_issuable");
    let e = c.issue_credential_as_tenant("t1", "job-golden", &CredentialRequest::new("t2", "constructor", "registry_write")).unwrap_err();
    assert_eq!(code_of(e), "pulso:tenant_mismatch");
    g.finish();
}

#[test]
fn credential_requests_are_validated_and_exp_is_an_integer_when_concrete() {
    let f = FakeCore::start();
    let c = client(&f.addr);
    assert!(matches!(c.issue_credential("j", &CredentialRequest::new("", "r", "p")), Err(OpError::Invalid(_))));
    assert!(matches!(c.issue_credential("j", &CredentialRequest::new("t", "r", "")), Err(OpError::Invalid(_))));
    assert!(f.requests().is_empty());
    f.script(200, json!({"exp": 1791072900, "jws": "x.y.z", "kid": "id1"}));
    assert_eq!(c.issue_credential("j", &CredentialRequest::new("t1", "constructor", "core_task")).unwrap().exp(), Some(1_791_072_900));
}

#[test]
fn credential_issue_is_never_retried() {
    let f = FakeCore::start();
    f.script(503, json!({"schema_version":"1","code":"pulso:credential_signing_unavailable","retryable":true,"trace_id":"t","details":{}}));
    let e = client(&f.addr).issue_credential("j", &CredentialRequest::new("t1", "constructor", "core_task")).unwrap_err();
    assert_eq!(e.disposition(), core_client::errors::Disposition::Retry);
    assert_eq!(f.requests().len(), 1, "minting twice would issue two credentials");
}
