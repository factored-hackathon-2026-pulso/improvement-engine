//! Offline TDD of the B2 writer against a scripted transport: success, idempotency, resume, base check, closed denial reasons,
//! the allow-list (never approve/publish/promote) and the builder-run path.
mod common;
use common::*;
use registry_writer::guard::allowed;
use registry_writer::{MemoryStore, Reason, Receipt, ReceiptStore, Via, Writer};
use serde_json::{Value, json};

fn deliver(via: Via, steps: Vec<Step>) -> (registry_writer::Outcome, Script) {
    let script = Script::new(steps);
    let store = MemoryStore::new();
    let o = Writer::new(cfg(via), &script, &store).deliver(&submission());
    (o, script)
}

fn reason_of(via: Via, steps: Vec<Step>) -> (Reason, registry_writer::Outcome, Script) {
    let (o, s) = deliver(via, steps);
    (o.reason.unwrap_or_else(|| panic!("expected a denial, got {}", o.to_json())), o, s)
}

#[test]
fn a_clean_delivery_is_proposal_id_valid_and_non_empty_changes() {
    let (o, s) = deliver(Via::RegistryApi, direct_steps());
    assert!(o.delivered(), "{}", o.to_json());
    assert_eq!((o.proposal_id.as_deref(), o.valid, o.changes, o.state.as_deref()), (Some("prp_1"), Some(true), 1, Some("draft")));
    assert!(!o.replayed);
    assert_eq!(s.remaining(), 0);
    let log = s.log.borrow();
    let create = log.iter().find(|l| l.method == "POST" && l.path == "/v1/registry/proposals").unwrap();
    let body = create.body.as_ref().unwrap();
    assert_eq!(body["origin"], "auto_detect");
    assert_eq!(body["agent_id"], "copiloto-asesor");
    assert!(body["title"].as_str().unwrap().starts_with("[improvement-engine] prompt:p/copiloto "));
    assert!(create.idem.as_deref().unwrap().starts_with("pulso-"), "the key travels for the day the registry honours it");
    let put = log.iter().find(|l| l.method == "PUT").unwrap().body.as_ref().unwrap().clone();
    assert_eq!(put["expected_rev"], 0, "put_draft uses the rev the create returned");
    assert_eq!(put["changes"][0]["content"], patch_change("1.0.1")["content"], "the compiled content is sent byte-for-byte");
    let docs = &put["changes"][0]["docs"];
    assert!(!docs["changelog"].as_str().unwrap().is_empty(), "the registry requires a changelog the compiler does not write");
    assert!(docs["rationale"].as_str().unwrap().contains("77 of 100"));
}

#[test]
fn outcome_is_labelled_honestly_and_never_carries_a_token() {
    let (o, _) = deliver(Via::RegistryApi, direct_steps());
    let j = o.to_json().to_string();
    assert!(j.contains("local-stack") && j.contains("engine compiler") && j.contains("registry-api") && j.contains("engine builder principal"));
    assert!(j.contains("\"engine_never_approves_publishes_or_promotes\":true"));
    assert!(!j.contains(REG_TOKEN) && !j.contains(RUN_TOKEN));
    assert!(!format!("{:?}", cfg(Via::RegistryApi).registry_token).contains(REG_TOKEN), "Jws Debug is redacted");
}

#[test]
fn the_same_finding_key_never_opens_a_second_proposal() {
    let script = Script::new(direct_steps());
    let store = MemoryStore::new();
    let w = Writer::new(cfg(Via::RegistryApi), &script, &store);
    let first = w.deliver(&submission());
    assert!(first.delivered());
    // retry: the receipt is confirmed in the registry, validated, and returned as a replay
    script.push(("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 1))));
    script.push(("POST /v1/registry/proposals/prp_1/validate", ok(valid())));
    script.push(("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 1))));
    let again = w.deliver(&submission());
    assert!(again.delivered() && again.replayed, "{}", again.to_json());
    assert_eq!(again.proposal_id, first.proposal_id);
    assert_eq!(script.count("POST", "/v1/registry/proposals/prp_1/validate"), 2);
    assert_eq!(script.log.borrow().iter().filter(|l| l.method == "POST" && l.path == "/v1/registry/proposals").count(), 1);
}

#[test]
fn a_crash_between_create_and_put_draft_resumes_without_a_new_proposal() {
    let script = Script::new(vec![
        ("GET /v1/registry/proposals/prp_1", ok(proposal_detail(0, "draft", 0))),
        ("PUT /v1/registry/proposals/prp_1/draft", ok(json!({"rev": 1}))),
        ("POST /v1/registry/proposals/prp_1/validate", ok(valid())),
        ("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 1))),
    ]);
    let store = MemoryStore::new();
    store.put(&submission().key(), Receipt { proposal_id: "prp_1".into(), agent_id: "copiloto-asesor".into(), created_at: 1 }).unwrap();
    let o = Writer::new(cfg(Via::RegistryApi), &script, &store).deliver(&submission());
    assert!(o.delivered() && o.replayed, "{}", o.to_json());
    assert_eq!(script.count("POST", "/v1/registry/proposals"), 1, "only the validate call, no create");
    assert_eq!(script.log.borrow()[1].body.as_ref().unwrap()["expected_rev"], 0, "the resume uses the rev the registry holds");
}

#[test]
fn a_receipt_the_registry_no_longer_knows_is_dropped_and_delivered_fresh() {
    let mut steps = vec![("GET /v1/registry/proposals/prp_old", status(404, "not_found"))];
    steps.extend(direct_steps());
    let script = Script::new(steps);
    let store = MemoryStore::new();
    store.put(&submission().key(), Receipt { proposal_id: "prp_old".into(), agent_id: "copiloto-asesor".into(), created_at: 1 }).unwrap();
    let o = Writer::new(cfg(Via::RegistryApi), &script, &store).deliver(&submission());
    assert!(o.delivered() && !o.replayed, "{}", o.to_json());
    assert_eq!(store.get(&submission().key()).unwrap().proposal_id, "prp_1");
}

#[test]
fn a_changed_live_base_is_refused_before_any_write() {
    let steps = vec![("GET /v1/registry/entities/prompt/p/copiloto", ok(entity("1.0.1", "Texto nuevo que otra persona publico.")))];
    let (r, o, s) = reason_of(Via::RegistryApi, steps);
    assert_eq!(r, Reason::BaseChanged);
    assert!(o.proposal_id.is_none() && s.count("POST", "/v1/registry") == 0);
}

#[test]
fn a_missing_live_artifact_is_base_missing() {
    let (r, ..) = reason_of(Via::RegistryApi, vec![("GET /v1/registry/entities/", status(404, "not_found"))]);
    assert_eq!(r, Reason::BaseMissing);
}

#[test]
fn registry_refusals_map_to_closed_reasons() {
    let create_with = |reply| {
        let steps = vec![("GET /v1/registry/entities/", ok(entity("1.0.0", BASE_ES))), listing_empty(), ("POST /v1/registry/proposals", reply)];
        reason_of(Via::RegistryApi, steps).0
    };
    assert_eq!(create_with(status(429, "quota_exceeded")), Reason::QuotaExceeded);
    assert_eq!(create_with(status(401, "credentials_invalid")), Reason::Unauthorized);
    assert_eq!(create_with(status(403, "forbidden_role")), Reason::ForbiddenRole);
    assert_eq!(create_with(status(403, "step_up_required")), Reason::StepUpRequired);
    assert_eq!(create_with(status(500, "integrity_error")), Reason::RegistryError);
    assert_eq!(create_with(Err(registry_writer::TransportError::NotSent("refused".into()))), Reason::RegistryUnreachable);
    assert_eq!(create_with(Err(registry_writer::TransportError::Unknown("reset".into()))), Reason::OutcomeUnknown);
}

#[test]
fn a_refused_draft_keeps_the_proposal_id_and_never_leaks_free_text() {
    let mut steps = direct_steps();
    steps.truncate(3);
    steps.push(("PUT /v1/registry/proposals/prp_1/draft", Ok(registry_writer::Reply { status: 422, body: json!({"code": "validation_failed", "detail": "SECRET free text", "violations": [{"rule": "REG-SCHEMA"}]}) })));
    let (r, o, _) = reason_of(Via::RegistryApi, steps);
    assert_eq!(r, Reason::ValidationFailed);
    assert_eq!(o.proposal_id.as_deref(), Some("prp_1"));
    assert!(o.detail.contains("REG-SCHEMA") && !o.detail.contains("SECRET"), "{}", o.detail);
}

#[test]
fn stale_proposal_is_closed() {
    let mut steps = direct_steps();
    steps.truncate(3);
    steps.push(("PUT /v1/registry/proposals/prp_1/draft", status(409, "proposal_stale")));
    assert_eq!(reason_of(Via::RegistryApi, steps).0, Reason::ProposalStale);
}

#[test]
fn registry_violations_are_draft_invalid_not_success() {
    let mut steps = direct_steps();
    steps.truncate(4);
    steps.push(("POST /v1/registry/proposals/prp_1/validate", ok(json!({"violations": [{"rule": "REG-REF", "path": "x", "message": "m"}], "candidate_hash": null, "auto_bumped": []}))));
    let (r, o, _) = reason_of(Via::RegistryApi, steps);
    assert_eq!(r, Reason::DraftInvalid);
    assert!(o.proposal_id.is_some() && o.valid.is_none() && !o.delivered());
}

#[test]
fn an_empty_stored_draft_is_not_success() {
    let mut steps = direct_steps();
    steps.truncate(5);
    steps.push(("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 0))));
    assert_eq!(reason_of(Via::RegistryApi, steps).0, Reason::EmptyDraft);
}

#[test]
fn the_local_guard_predicts_the_24h_quota_without_sending_anything() {
    let script = Script::new(vec![]);
    let store = MemoryStore::new();
    for i in 0..10 {
        store.put(&format!("k{i}"), Receipt { proposal_id: format!("p{i}"), agent_id: "a".into(), created_at: 1_000_000 }).unwrap();
    }
    let mut c = cfg(Via::RegistryApi);
    c.check_base = false;
    let o = Writer::new(c, &script, &store).with_clock(|| 1_000_100).deliver(&submission());
    assert_eq!(o.reason, Some(Reason::QuotaExceeded));
    assert!(script.requests().is_empty());
    // a day later the window has moved on
    let mut s2 = direct_steps();
    let _ = s2.remove(0);
    let script2 = Script::new(s2);
    let mut c2 = cfg(Via::RegistryApi);
    c2.check_base = false;
    let o2 = Writer::new(c2, &script2, &store).with_clock(|| 1_000_000 + 25 * 3600).deliver(&submission());
    assert!(o2.delivered(), "{}", o2.to_json());
}

#[test]
fn nothing_to_write_and_unsafe_shapes_send_nothing() {
    let script = Script::new(vec![]);
    let store = MemoryStore::new();
    let w = Writer::new(cfg(Via::RegistryApi), &script, &store);
    let mut s = submission();
    s.kind = "no_change".into();
    assert_eq!(w.deliver(&s).reason, Some(Reason::NothingToPropose));
    let mut s = submission();
    s.changes.clear();
    assert_eq!(w.deliver(&s).reason, Some(Reason::NothingToPropose));
    let mut s = submission();
    s.changes = vec![json!({"kind": "release_settings", "content": {"id": "x", "version": "1"}, "docs": {}})];
    assert_eq!(w.deliver(&s).reason, Some(Reason::InvalidSubmission));
    // INH1: values (interrupts, ruleset, limits) are never written by the engine, not even next to a donor reference
    for content in [json!({"interrupts": []}), json!({"inherit_from": "consultas-demo", "interrupts": []}), json!({"inherit_from": "../x"}), json!({"inherit_from": 7}), json!({"max_input_chars": 100})] {
        let mut s = submission();
        s.changes = vec![json!({"kind": "release_settings", "content": content, "docs": {}})];
        assert_eq!(w.deliver(&s).reason, Some(Reason::InvalidSubmission), "{content}");
    }
    let mut s = submission();
    s.changes = vec![json!({"kind": "prompt", "content": {"id": "p/x"}, "docs": {}})];
    assert_eq!(w.deliver(&s).reason, Some(Reason::InvalidSubmission));
    let mut s = submission();
    s.agent_id = "../admin".into();
    assert_eq!(w.deliver(&s).reason, Some(Reason::InvalidSubmission));
    assert!(script.requests().is_empty());
}

#[test]
fn the_allow_list_refuses_every_management_operation() {
    for (m, p) in [
        ("POST", "/v1/registry/proposals/prp_1/approve"),
        ("POST", "/v1/registry/proposals/prp_1/publish"),
        ("POST", "/v1/registry/proposals/prp_1/reopen"),
        ("POST", "/v1/registry/proposals/prp_1/reject"),
        ("POST", "/v1/registry/aliases/copiloto-asesor/prod"),
        ("POST", "/v1/registry/releases/r1/revoke"),
        ("GET", "/v1/registry/proposals/../../admin"),
        ("DELETE", "/v1/registry/proposals/prp_1"),
        ("GET", "/v1/export/runs"),
    ] {
        assert!(!allowed(m, p), "{m} {p}");
    }
    for (m, p) in [
        ("POST", "/v1/registry/proposals"),
        ("GET", "/v1/registry/proposals/prp_1"),
        ("PUT", "/v1/registry/proposals/prp_1/draft"),
        ("POST", "/v1/registry/proposals/prp_1/validate"),
        ("POST", "/v1/registry/proposals/prp_1/freeze"),
        ("POST", "/v1/registry/proposals/prp_1/evaluate"),
        ("GET", "/v1/registry/entities/prompt/p/copiloto"),
        ("POST", "/v1/runs"),
    ] {
        assert!(allowed(m, p), "{m} {p}");
    }
}

/// W11: freeze and evaluate are allowed, the human decisions are not, in every spelling an attacker or a bug could try.
#[test]
fn every_human_decision_verb_stays_refused_whatever_the_method_suffix_or_case() {
    for verb in ["approve", "publish", "promote", "reject", "reopen", "revoke", "rollback", "release", "alias", "delete"] {
        for m in ["POST", "PUT", "PATCH", "GET", "DELETE"] {
            for p in [
                format!("/v1/registry/proposals/prp_1/{verb}"),
                format!("/v1/registry/proposals/prp_1/{verb}/"),
                format!("/v1/registry/proposals/prp_1/freeze/{verb}"),
                format!("/v1/registry/proposals/prp_1/evaluate/{verb}"),
                format!("/v1/registry/{verb}/prp_1"),
                format!("/v1/registry/proposals/prp_1/{}", verb.to_uppercase()),
            ] {
                assert!(!allowed(m, &p), "{m} {p}");
            }
        }
    }
    for (m, p) in [
        ("POST", "/v1/registry/releases/r1/revoke"),
        ("POST", "/v1/registry/aliases/consultas/prod"),
        ("POST", "/v1/registry/proposals/prp_1/approve"),
        ("POST", "/v1/registry/proposals/prp_1/publish"),
        ("POST", "/v1/registry/proposals/prp_1/reject"),
    ] {
        assert!(!allowed(m, p), "{m} {p}");
    }
}

#[test]
fn freeze_and_evaluate_are_post_only_and_take_exactly_one_safe_proposal_id() {
    let crlf = format!("{}{}", char::from(13), char::from(10));
    for verb in ["freeze", "evaluate"] {
        assert!(allowed("POST", &format!("/v1/registry/proposals/prp_1/{verb}")));
        for m in ["GET", "PUT", "PATCH", "DELETE", "post", "HEAD"] {
            assert!(!allowed(m, &format!("/v1/registry/proposals/prp_1/{verb}")), "{m} {verb}");
        }
        for p in [
            format!("/v1/registry/proposals/../{verb}"),
            format!("/v1/registry/proposals//{verb}"),
            format!("/v1/registry/proposals/prp 1/{verb}"),
            format!("/v1/registry/proposals/prp%2F1/{verb}"),
            format!("/v1/registry/proposals/prp_1/extra/{verb}"),
            format!("/v1/registry/proposals/prp_1/{verb}/extra"),
            format!("/v1/registry/proposals/prp_1/{verb}?approve=1"),
            format!("/v1/registry/proposals/prp_1/{verb}?agent_id=a&limit=2"),
            format!("/v1/registry/proposals/prp_1/{verb}#approve"),
            format!("/v1/registry/proposals/prp_1/{verb}{}", char::from(0)),
            format!("/v1/registry/proposals/prp_1/{verb}{crlf}X: y"),
            format!("/v1/registry/proposals/prp_1{crlf}/{verb}"),
            format!("/v1/registry/proposals/{}/{verb}", "a".repeat(121)),
        ] {
            assert!(!allowed("POST", &p), "{p:?}");
        }
    }
}

#[test]
fn the_allow_list_refuses_query_fragment_and_control_bytes_so_nothing_can_be_smuggled_into_the_request_line() {
    let crlf = format!("{}{}", char::from(13), char::from(10));
    for (m, p) in [
        ("POST", "/v1/runs?x=1".to_string()),
        ("GET", "/v1/registry/proposals?agent_id=a&limit=200&approve=1".to_string()),
        ("GET", "/v1/registry/proposals?agent_id=a/../b&limit=200".to_string()),
        ("POST", "/v1/registry/proposals?agent_id=a&limit=200".to_string()),
        ("GET", "/v1/registry/proposals/prp_1?agent_id=a&limit=2".to_string()),
        ("POST", format!("/v1/runs?x HTTP/1.1{crlf}Authorization: Bearer evil{crlf}X: ")),
        ("POST", format!("/v1/runs HTTP/1.1{crlf}X: y")),
        ("GET", "/v1/registry/proposals/prp_1?../../approve".to_string()),
        ("GET", "/v1/registry/proposals/prp_1#x".to_string()),
        ("GET", format!("/v1/registry/proposals/prp_1{crlf}X-A: b")),
        ("POST", "/v1/registry/proposals ".to_string()),
        ("POST", format!("/v1/runs{}", char::from(10))),
    ] {
        assert!(!allowed(m, &p), "{m} {p:?}");
    }
}

#[test]
fn registry_rule_ids_in_a_denial_are_a_closed_vocabulary_never_prose() {
    let mut steps = direct_steps();
    steps.truncate(3);
    steps.push(("PUT /v1/registry/proposals/prp_1/draft", Ok(registry_writer::Reply { status: 422, body: json!({"code": "validation_failed", "violations": [{"rule": "customer jane@example.com said hello"}]}) })));
    let (_, o, _) = reason_of(Via::RegistryApi, steps);
    assert!(!o.detail.contains("jane") && !o.detail.contains("hello"), "{}", o.detail);
}

#[test]
fn a_whole_delivery_only_ever_uses_the_proposal_routes() {
    let (_, s) = deliver(Via::RegistryApi, direct_steps());
    for (m, p) in s.requests() {
        assert!(allowed(&m, &p), "{m} {p}");
        assert!(!["approve", "publish", "promote", "freeze", "evaluate"].iter().any(|w| p.contains(w)), "{p}");
    }
}

#[test]
fn the_key_is_stable_per_finding_and_target() {
    let a = Submission::new(&finding(), &compiled_patch());
    let b = Submission::new(&finding(), &compiled_patch());
    assert_eq!(a.key(), b.key());
    assert!(a.key().starts_with("pulso-") && a.key().len() == 30);
    let mut other = compiled_patch();
    other.target_ref = "prompt:p/otro".into();
    assert_ne!(Submission::new(&finding(), &other).key(), a.key());
    assert!(a.title().len() <= 120);
}

#[test]
fn the_registry_title_is_capped_at_the_platform_120_keeping_the_key_suffix() {
    let mut c = compiled_patch();
    c.target_ref = format!("prompt:p/{}", "nombre_largo ".repeat(30));
    let s = Submission::new(&finding(), &c);
    let t = s.title();
    assert!(t.chars().count() <= 120, "{t}");
    assert!(t.starts_with("[improvement-engine] ") && t.ends_with(&s.key()[6..14]), "{t}");
    assert_eq!(t, Submission::new(&finding(), &c).title(), "deterministic, so the title lookup still finds it");
}

use registry_writer::Submission;

// ---- builder-run -----------------------------------------------------------------------------------------------------------

fn run_ok(valid_slot: bool) -> Step {
    ("POST /v1/runs", created(json!({"status": "closed", "outcome": "completed", "output_map": {"proposal_id": "prp_1", "rev": 0, "valid": valid_slot}})))
}

fn run_steps() -> Vec<Step> {
    vec![
        ("GET /v1/registry/entities/", ok(entity("1.0.0", BASE_ES))),
        run_ok(true),
        ("POST /v1/registry/proposals/prp_1/validate", ok(valid())),
        ("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 1))),
    ]
}

#[test]
fn a_builder_run_delivery_reads_back_the_changes_the_agent_wrote() {
    let (o, s) = deliver(Via::BuilderRun, run_steps());
    assert!(o.delivered(), "{}", o.to_json());
    assert_eq!((o.proposal_id.as_deref(), o.valid, o.changes), (Some("prp_1"), Some(true), 1));
    let j = o.to_json().to_string();
    assert!(j.contains("pulso-builder agent") && j.contains("builder-run"), "the draft is labelled as the agent's");
    let log = s.log.borrow();
    let run = log.iter().find(|l| l.path == "/v1/runs").unwrap();
    assert!(run.bearer_is_run_token, "the run uses the engine-signed builder token");
    assert!(run.idem.as_deref().unwrap().starts_with("pulso-"), "the run is idempotent on the server too");
    let b = run.body.as_ref().unwrap();
    assert_eq!(b["agent"], "pulso-builder");
    assert_eq!(b["input"]["agente"], "copiloto-asesor");
    let goal = b["input"]["objetivo"].as_str().unwrap();
    assert!(goal.chars().count() <= 200 && goal.starts_with("[improvement-engine]"));
    let evidence = b["input"]["evidencia"].as_str().unwrap();
    let longest_digits = evidence.split(|c: char| !c.is_ascii_digit()).map(str::len).max().unwrap_or(0);
    assert!(longest_digits < 6, "long digit runs would be tokenised by agent-core: {evidence}");
    assert!(log.iter().filter(|l| l.path != "/v1/runs").all(|l| !l.bearer_is_run_token), "registry calls use the registry credential");
}

#[test]
fn a_builder_run_that_escalates_or_answers_nothing_is_closed() {
    let esc = ("POST /v1/runs", created(json!({"status": "closed", "outcome": "escalated", "reason_code": "tool_failure"})));
    let (r, o, _) = reason_of(Via::BuilderRun, vec![("GET /v1/registry/entities/", ok(entity("1.0.0", BASE_ES))), esc]);
    assert_eq!(r, Reason::RunNotCompleted);
    assert!(o.detail.contains("tool_failure"));
    let empty = ("POST /v1/runs", created(json!({"outcome": "completed", "output_map": {}})));
    assert_eq!(reason_of(Via::BuilderRun, vec![("GET /v1/registry/entities/", ok(entity("1.0.0", BASE_ES))), empty]).0, Reason::RunMalformed);
    let q = ("POST /v1/runs", status(429, "quota_exceeded"));
    assert_eq!(reason_of(Via::BuilderRun, vec![("GET /v1/registry/entities/", ok(entity("1.0.0", BASE_ES))), q]).0, Reason::QuotaExceeded);
}

#[test]
fn a_run_that_reports_an_invalid_draft_is_not_success() {
    let steps = vec![("GET /v1/registry/entities/", ok(entity("1.0.0", BASE_ES))), run_ok(false)];
    let (r, o, _) = reason_of(Via::BuilderRun, steps);
    assert_eq!(r, Reason::DraftInvalid);
    assert_eq!(o.proposal_id.as_deref(), Some("prp_1"));
}

#[test]
fn without_a_readable_registry_the_run_cannot_claim_success() {
    let mut steps = run_steps();
    steps.truncate(2);
    steps.push(("POST /v1/registry/proposals/prp_1/validate", status(401, "credentials_invalid")));
    assert_eq!(reason_of(Via::BuilderRun, steps).0, Reason::Unauthorized);
    let mut steps = run_steps();
    steps.truncate(3);
    steps.push(("GET /v1/registry/proposals/prp_1", status(403, "forbidden_role")));
    assert_eq!(reason_of(Via::BuilderRun, steps).0, Reason::ReadbackUnavailable);
}

#[test]
fn a_builder_run_retry_does_not_start_a_second_run() {
    let script = Script::new(run_steps());
    let store = MemoryStore::new();
    let w = Writer::new(cfg(Via::BuilderRun), &script, &store);
    assert!(w.deliver(&submission()).delivered());
    script.push(("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 1))));
    script.push(("POST /v1/registry/proposals/prp_1/validate", ok(valid())));
    script.push(("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 1))));
    let again = w.deliver(&submission());
    assert!(again.delivered() && again.replayed);
    assert_eq!(script.count("POST", "/v1/runs"), 1);
}

#[test]
fn the_builder_run_needs_its_token() {
    let script = Script::new(vec![]);
    let store = MemoryStore::new();
    let mut c = cfg(Via::BuilderRun);
    c.run_token = None;
    let o = Writer::new(c, &script, &store).deliver(&submission());
    assert_eq!(o.reason, Some(Reason::InvalidSubmission));
}

// ---- live baseline ---------------------------------------------------------------------------------------------------------

#[test]
fn the_live_baseline_replaces_the_fixture_and_says_so() {
    let catalog = reasoning::catalog::Catalog::bundled();
    assert_eq!(catalog.label, "fixture-baseline");
    let script = Script::new((0..catalog.target_refs().len()).map(|_| ("GET /v1/registry/entities/", status(404, "not_found"))).collect());
    let store = MemoryStore::new();
    let w = Writer::new(cfg(Via::RegistryApi), &script, &store);
    let none = w.refresh_catalog(&catalog);
    assert!(none.live.is_empty() && none.catalog.label == "fixture-baseline", "nothing live: the fixture label stays");
    assert!(none.fixture.iter().all(|(_, c)| *c == "base_missing"));

    let live = |r: &str| -> Result<reasoning::catalog::Artifact, (Reason, String)> {
        if r == "prompt:p/copiloto" { Ok(live_artifact()) } else { Err((Reason::BaseMissing, String::new())) }
    };
    let mixed = registry_writer::baseline::refresh_with(&catalog, &catalog.target_refs(), &live);
    assert_eq!(mixed.catalog.label, "mixed-live-and-fixture");
    assert_eq!(mixed.live, vec!["prompt:p/copiloto".to_string()]);
    assert_eq!(mixed.catalog.get("prompt:p/copiloto").unwrap().locales["es"], BASE_ES, "the live text is the baseline now");
    let all = registry_writer::baseline::refresh_with(&catalog, &["prompt:p/copiloto".to_string()], &live);
    assert_eq!(all.catalog.label, "live-registry");
}

#[test]
fn an_entity_answer_becomes_an_artifact_with_its_other_fields() {
    let a = live_artifact();
    assert_eq!((a.kind.as_str(), a.id.as_str(), a.version.as_str()), ("prompt", "p/copiloto", "1.0.0"));
    assert_eq!(a.extra["model_profile"], "perfil-generacion@1");
    assert!(registry_writer::baseline::parse_entity(&json!({"ref": {"kind": "prompt", "id": "x", "version": "1"}, "content": {"locales": {}}}), vec![]).is_none());
    assert!(registry_writer::baseline::entity_path("prompt", "../x").is_none());
    let _: Value = entity("1", "x");
}

#[test]
fn the_fixed_listing_query_is_allowed() {
    assert!(allowed("GET", "/v1/registry/proposals?agent_id=copiloto-asesor&limit=200"));
    assert!(allowed("GET", "/v1/registry/proposals?agent_id=copiloto-asesor&limit=50&offset=100"));
}

#[test]
fn a_lost_receipt_is_recovered_from_the_registry_listing_and_no_second_proposal_is_opened() {
    let title = submission().title();
    let listing = ok(json!({"items": [{"proposal_id": "prp_1", "agent_id": "copiloto-asesor", "state": "draft", "title": title, "rev": 1}], "total": 1}));
    let script = Script::new(vec![
        ("GET /v1/registry/entities/", ok(entity("1.0.0", BASE_ES))),
        ("GET /v1/registry/proposals?agent_id=copiloto-asesor", listing),
        ("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 1))),
        ("POST /v1/registry/proposals/prp_1/validate", ok(valid())),
        ("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 1))),
    ]);
    let store = MemoryStore::new();
    let o = Writer::new(cfg(Via::RegistryApi), &script, &store).deliver(&submission());
    assert!(o.delivered() && o.replayed, "{}", o.to_json());
    assert!(!script.requests().contains(&("POST".to_string(), "/v1/registry/proposals".to_string())), "nothing was created");
    assert!(store.get(&submission().key()).is_some(), "the receipt is rebuilt");
}

#[test]
fn put_draft_carries_its_own_idempotency_key_derived_from_the_finding_key() {
    let (_, s) = deliver(Via::RegistryApi, direct_steps());
    let log = s.log.borrow();
    let put = log.iter().find(|l| l.method == "PUT").unwrap();
    assert_eq!(put.idem.as_deref(), Some(format!("{}-draft", submission().key()).as_str()));
}

// ---- W15 / R11: no generated text carries a digit run of 6 or more ---------------------------------------------------------

fn longest_digit_run(s: &str) -> usize {
    let (mut best, mut cur) = (0, 0);
    for c in s.chars() {
        cur = if c.is_ascii_digit() { cur + 1 } else { 0 };
        best = best.max(cur);
    }
    best
}

#[test]
fn the_key_title_and_default_changelog_never_carry_a_digit_run_of_six() {
    for i in 0..600u32 {
        let mut s = submission();
        s.evidence_ref = format!("ev_{i:016x}");
        let key = s.key();
        assert!(longest_digit_run(&key) < 6 && key.starts_with("pulso-") && key.len() == 30, "{key}");
        assert!(longest_digit_run(&s.title()) < 6, "{}", s.title());
        assert_eq!(key, s.key(), "stable");
    }
}

#[test]
fn delivered_docs_are_free_of_digit_runs_of_six_even_when_the_text_came_with_one() {
    let mut s = submission();
    s.changes[0]["docs"] = json!({"description": "evidence ev_1234567890123456 ticket 9876543", "rationale": "case 12345678 repeats", "changelog": "key pulso-111111222222333333444444"});
    let script = Script::new(direct_steps());
    let store = MemoryStore::new();
    let o = Writer::new(cfg(Via::RegistryApi), &script, &store).deliver(&s);
    assert!(o.delivered(), "{}", o.to_json());
    let log = script.log.borrow();
    let put = log.iter().find(|l| l.method == "PUT").unwrap().body.as_ref().unwrap().clone();
    for k in ["description", "rationale", "changelog"] {
        let t = put["changes"][0]["docs"][k].as_str().unwrap();
        assert!(longest_digit_run(t) < 6, "{k}: {t}");
    }
    let create = log.iter().find(|l| l.method == "POST" && l.path == "/v1/registry/proposals").unwrap().body.as_ref().unwrap().clone();
    assert!(longest_digit_run(create["title"].as_str().unwrap()) < 6);
}

/// One-shot local HTTP server: returns the raw request head it received (empty when nothing connected within the wait).
fn one_shot_server() -> (String, std::thread::JoinHandle<String>) {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.set_nonblocking(true).unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        loop {
            match l.accept() {
                Ok((mut c, _)) => {
                    c.set_nonblocking(false).unwrap();
                    let mut buf = vec![0u8; 8192];
                    let n = c.read(&mut buf).unwrap_or(0);
                    let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                    return String::from_utf8_lossy(&buf[..n]).to_string();
                }
                Err(_) if t0.elapsed() < std::time::Duration::from_millis(1500) => std::thread::sleep(std::time::Duration::from_millis(10)),
                Err(_) => return String::new(),
            }
        }
    });
    (addr, h)
}

#[test]
fn the_http_transport_adds_the_story_traceparent_as_a_header_only_and_refuses_a_crlf_header_value() {
    use core_client::authorizer::Jws;
    use core_client::trace::{self, TraceCtx};
    use registry_writer::{HttpTransport, Request, Transport, TransportError};
    let jws = Jws::new("tok".to_string());
    let (addr, srv) = one_shot_server();
    let t = HttpTransport::new(&addr, std::time::Duration::from_secs(3));
    {
        let _s = trace::enter(TraceCtx { finding_key: "ev_3f9a1c07d2b84e51".into(), run_id: "value-loop-trg-20261005-0001".into(), release: "r1".into(), agent: "pulso-writer".into(), locale: String::new(), case_type: "prompt".into() });
        trace::set_stage("deliver", 1);
        let r = t.send(&Request { method: "POST", path: "/v1/runs".into(), bearer: &jws, idempotency_key: Some("k1"), body: Some(json!({"a": 1})) }).unwrap();
        assert_eq!(r.status, 200);
    }
    let head = srv.join().unwrap().to_ascii_lowercase();
    assert!(head.starts_with("post /v1/runs http/1.1\r\n"), "path unchanged: {head}");
    assert!(head.contains("\r\ntraceparent: 00-7e1ffba44834058839ef1a914c474128-91881332db42dd99-01\r\n") || head.contains("\r\ntraceparent: 00-"), "{head}");
    assert!(head.contains("\r\nbaggage: session.id=value-loop-trg-20261005-0001,release=r1,langfuse.trace.tags=agent%3apulso-writer%2ccase-type%3aprompt%2cstage%3adeliver\r\n"), "{head}");
    assert!(head.contains("\r\nidempotency-key: k1\r\n") && head.contains("\r\nauthorization: bearer tok\r\n"));
    assert!(allowed("POST", "/v1/runs"), "the allow-list is untouched: headers only");

    // header injection: CR/LF in a header value is refused before anything is written
    let (addr, srv) = one_shot_server();
    let t = HttpTransport::new(&addr, std::time::Duration::from_secs(3));
    let crlf = format!("{}{}", char::from(13), char::from(10));
    let evil = format!("k1{crlf}X-Evil: 1");
    let r = t.send(&Request { method: "POST", path: "/v1/runs".into(), bearer: &jws, idempotency_key: Some(&evil), body: None });
    assert!(matches!(r, Err(TransportError::NotSent(ref m)) if m.contains("refused")), "{r:?}");
    assert_eq!(srv.join().unwrap(), "", "nothing reached the server");
}
