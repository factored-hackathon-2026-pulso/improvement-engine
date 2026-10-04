//! Typed invoke / read_task / version over the K1 transport, against the GENERATED golden FakeCore: every request
//! must equal the Python bridge golden (method, path, JWT claim profile, Idempotency-Key, body).
mod common;
use common::golden::GoldenCore;
use common::{FakeCore, KID, SEED};
use core_client::dto::{RunState, Stage, TaskInvocation};
use core_client::{CallError, ClientConfig, CoreClient, OpError};
use serde_json::json;

fn client(addr: &str) -> CoreClient {
    let mut cfg = ClientConfig::new(addr, KID, SEED, "bridge-1");
    cfg.timeout = std::time::Duration::from_secs(5);
    CoreClient::new(cfg)
}

fn scout() -> TaskInvocation {
    let mut i = TaskInvocation::new("t1", "job-golden-scout", Stage::Scout, "golden-scout-1");
    i.agent_id = "pulso-scout".into();
    i.agent_version = "1.0.0".into();
    i.release_id = "<release:scout>".into();
    i.pulso_run_ref = "pr-job-golden-scout".into();
    i.lab_grant_ref = "grant-contract".into();
    i.input = json!({"briefing_ref": "wiki/briefing.md"});
    i.extract_manifest_ref = Some("ex-1".into());
    i.memory_snapshot_ref = Some("mem-1".into());
    i
}

fn scout_binding() -> String {
    let key = core_client::canon::idempotency_key("t1", "job-golden-scout", "scout", 1, "golden-scout-1").unwrap();
    core_client::canon::task_binding_ref("t1", &key).unwrap()
}

#[test]
fn invoke_encodes_the_golden_request_and_decodes_the_run() {
    let g = GoldenCore::play(&[("invoke_scout", "invoke_scout_ok")]);
    let r = client(&g.addr).invoke(&scout()).unwrap();
    g.finish();
    assert_eq!(r.http_status, 200);
    assert_eq!(r.state, RunState::TerminalOk);
    assert!(r.is_success());
    assert_eq!(r.core_run_id.as_deref(), Some("<core_run_id#1>"));
    let f = r.fact("pulso_hypotheses").expect("whitelisted fact");
    assert_eq!(f.source_kind, "agent");
    assert_eq!(f.value["hypotheses"][0]["id"], "h1");
    assert!(r.receipt.as_ref().unwrap().budget_known);
}

#[test]
fn idempotent_replay_returns_the_same_run() {
    let g = GoldenCore::play(&[("invoke_scout", "invoke_scout_ok"), ("invoke_scout", "invoke_scout_replay")]);
    let c = client(&g.addr);
    let (a, b) = (c.invoke(&scout()).unwrap(), c.invoke(&scout()).unwrap());
    g.finish();
    assert_eq!(a.core_run_id, b.core_run_id);
    assert_eq!(a.raw, b.raw);
    let reqs = g.requests();
    assert_eq!(reqs[0].headers["idempotency-key"], reqs[1].headers["idempotency-key"]);
    assert_eq!(reqs[0].headers["idempotency-key"], "dbe68ddd4321eac7f2d87367e6782484a65f7a1477172556517de1df4846753f");
}

#[test]
fn same_key_other_body_is_a_digest_conflict() {
    let g = GoldenCore::play(&[("invoke_scout", "invoke_scout_ok"), ("invoke_scout", "invoke_digest_conflict")]);
    let c = client(&g.addr);
    c.invoke(&scout()).unwrap();
    let mut other = scout();
    other.input = json!({"briefing_ref": "wiki/other.md"});
    match c.invoke(&other).unwrap_err() {
        OpError::Call(CallError::Api(a)) => assert_eq!((a.code.as_str(), a.status), ("pulso:digest_conflict", 409)),
        e => panic!("{e:?}"),
    }
    g.finish();
}

#[test]
fn read_task_encodes_the_path_and_decodes_the_receipt() {
    let g = GoldenCore::play(&[("invoke_scout", "invoke_scout_ok"), ("invoke_scout", "read_task_ok")]);
    let c = client(&g.addr);
    let run = c.invoke(&scout()).unwrap();
    let again = c.read_task("t1", Some("job-golden-scout"), run.core_run_id.as_deref().unwrap()).unwrap();
    g.finish();
    assert_eq!(again.core_run_id, run.core_run_id);
    assert_eq!(again.receipt, run.receipt);
}

#[test]
fn read_task_unknown_is_not_found() {
    let g = GoldenCore::play(&[("invoke_scout", "read_task_unknown")]);
    match client(&g.addr).read_task("t1", Some("job-golden-scout"), "run-does-not-exist").unwrap_err() {
        OpError::Call(CallError::Api(a)) => assert_eq!(a.code, "pulso:not_found"),
        e => panic!("{e:?}"),
    }
    g.finish();
}

#[test]
fn release_unavailable_is_a_terminal_conflict() {
    let g = GoldenCore::play(&[("invoke_scout", "invoke_release_unavailable")]);
    let mut i = scout();
    i.logical_key = "golden-scout-2".into();
    i.release_id = "rel-does-not-exist".into();
    let e = client(&g.addr).invoke(&i).unwrap_err();
    g.finish();
    match e {
        OpError::Call(CallError::Api(a)) => assert_eq!(a.code, "pulso:release_pin_unavailable"),
        e => panic!("{e:?}"),
    }
}

#[test]
fn writer_commitment_and_evaluate_only_requests_match_the_goldens() {
    let g = GoldenCore::play(&[("writer_evaluation", "writer_create_put_freeze"), ("writer_evaluation", "invoke_evaluate_only")]);
    let c = client(&g.addr);
    let mut w = TaskInvocation::new("t1", "job-golden-writer", Stage::Writer, "golden-writer-1");
    w.agent_id = "pulso-writer".into();
    w.agent_version = "1.0.0".into();
    w.release_id = "<release:writer>".into();
    w.pulso_run_ref = "pr-job-golden-writer".into();
    w.lab_grant_ref = "grant-contract".into();
    w.input = json!({"base_release_id":"rel-demo","draft_plan_ref":"plan-golden","evaluate_enabled":false,"proposal_id":null});
    w.registry_mutation_commitment = Some(json!({"base_release_id":"rel-demo","create_agent_id":"atencion","create_origin":"builder_chat",
        "create_title":"pulso-key:golden","mode":"write","operations":["create_proposal","put_draft","freeze"],
        "put_draft_digest":"7e288b22b945d33f5c0832a1a146554736f1c559782a005221bbbd34027bbba9"}));
    let fz = c.invoke(&w).unwrap();
    assert_eq!(fz.state, RunState::TerminalOk);
    let wr = fz.fact("pulso_writer_receipts").unwrap();
    let proposal = wr.value["proposal_id"].as_str().unwrap().to_string();

    let mut e = TaskInvocation::new("t1", "job-golden-eval", Stage::Writer, "golden-eval-1");
    e.agent_id = "pulso-writer".into();
    e.agent_version = "1.0.0".into();
    e.release_id = "<release:writer>".into();
    e.pulso_run_ref = "pr-job-golden-eval".into();
    e.lab_grant_ref = "grant-contract".into();
    e.input = json!({"base_release_id":"rel-demo","draft_plan_ref":"plan-golden","evaluate_enabled":true,
        "evaluation_suite_id":"disputas-suite","evaluation_suite_version":"1.0.0","proposal_id":proposal});
    e.registry_mutation_commitment = Some(json!({"base_release_id":"rel-demo","evaluate_enabled":true,
        "evaluation_context_ref":"<evaluation_context_ref#1>","mode":"evaluate_only","operations":[],"proposal_id":proposal}));
    let ev = c.invoke(&e).unwrap();
    g.finish();
    assert_eq!(ev.fact("pulso_writer_receipts").unwrap().value["native_evaluation"]["verdict"], "pass");
    assert_eq!(ev.receipt.unwrap().task_binding_ref, "f5f19be72642cfcff1ff9ca604402acb7769f67306f25ab257e981945c61ee33");
}

#[test]
fn invalid_invocations_are_refused_before_sending() {
    let f = FakeCore::start();
    let c = client(&f.addr);
    let mut i = scout();
    i.logical_key = "a|b".into();
    assert!(matches!(c.invoke(&i), Err(OpError::Canon(_))));
    let mut i = scout();
    i.release_id = String::new();
    assert!(matches!(c.invoke(&i), Err(OpError::Invalid(_))));
    let mut i = scout();
    i.attempt = 0;
    assert!(matches!(c.invoke(&i), Err(OpError::Invalid(_))));
    let mut i = scout();
    i.input = json!([1]);
    assert!(matches!(c.invoke(&i), Err(OpError::Invalid(_))));
    assert!(f.requests().is_empty());
}

#[test]
fn non_terminal_202_is_a_state_not_a_failure() {
    let f = FakeCore::start();
    f.script(202, json!({"schema_version":"1","state":"sent","core_run_id":null,"reason":null,"outcome":null,"task_binding_ref":scout_binding()}));
    let r = client(&f.addr).invoke(&scout()).unwrap();
    assert_eq!((r.http_status, r.state, r.is_terminal(), r.is_success()), (202, RunState::Sent, false, false));
}

#[test]
fn a_receipt_for_another_binding_is_a_contract_violation() {
    let f = FakeCore::start();
    f.script(200, json!({"schema_version":"1","state":"terminal_ok","core_run_id":"r","reason":"completed","outcome":"completed",
        "task_binding_ref":"0000000000000000000000000000000000000000000000000000000000000000"}));
    assert!(matches!(client(&f.addr).invoke(&scout()), Err(OpError::Contract(_))));
}

#[test]
fn terminal_failed_run_is_decoded_not_raised() {
    let f = FakeCore::start();
    f.script(200, json!({"schema_version":"1","state":"terminal_failed","core_run_id":"r1","reason":"run_failed","outcome":"failed","task_binding_ref":scout_binding()}));
    let r = client(&f.addr).invoke(&scout()).unwrap();
    assert_eq!((r.state, r.is_success(), r.reason.as_deref()), (RunState::TerminalFailed, false, Some("run_failed")));
}

#[test]
fn version_probe_matches_the_pin() {
    let g = GoldenCore::play(&[("auth_and_envelope", "version_ok")]);
    let v = client(&g.addr).version().unwrap();
    g.finish();
    assert_eq!(v.runtime_profile, "agent_core_real");
    assert_eq!(v.doubles.len(), 6);
    assert!(v.check_pin().is_ok());
    let mut bad = v.clone();
    bad.contracts_version = "1.4.0".into();
    assert!(bad.check_pin().is_err());
    let mut bad = v;
    bad.agent_core_sha = "0".repeat(40);
    assert!(bad.check_pin().is_err());
}
