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
        let steps = vec![("GET /v1/registry/entities/", ok(entity("1.0.0", BASE_ES))), ("POST /v1/registry/proposals", reply)];
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
    steps.truncate(2);
    steps.push(("PUT /v1/registry/proposals/prp_1/draft", Ok(registry_writer::Reply { status: 422, body: json!({"code": "validation_failed", "detail": "SECRET free text", "violations": [{"rule": "REG-SCHEMA"}]}) })));
    let (r, o, _) = reason_of(Via::RegistryApi, steps);
    assert_eq!(r, Reason::ValidationFailed);
    assert_eq!(o.proposal_id.as_deref(), Some("prp_1"));
    assert!(o.detail.contains("REG-SCHEMA") && !o.detail.contains("SECRET"), "{}", o.detail);
}

#[test]
fn stale_proposal_is_closed() {
    let mut steps = direct_steps();
    steps.truncate(2);
    steps.push(("PUT /v1/registry/proposals/prp_1/draft", status(409, "proposal_stale")));
    assert_eq!(reason_of(Via::RegistryApi, steps).0, Reason::ProposalStale);
}

#[test]
fn registry_violations_are_draft_invalid_not_success() {
    let mut steps = direct_steps();
    steps.truncate(3);
    steps.push(("POST /v1/registry/proposals/prp_1/validate", ok(json!({"violations": [{"rule": "REG-REF", "path": "x", "message": "m"}], "candidate_hash": null, "auto_bumped": []}))));
    let (r, o, _) = reason_of(Via::RegistryApi, steps);
    assert_eq!(r, Reason::DraftInvalid);
    assert!(o.proposal_id.is_some() && o.valid.is_none() && !o.delivered());
}

#[test]
fn an_empty_stored_draft_is_not_success() {
    let mut steps = direct_steps();
    steps.truncate(4);
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
        ("POST", "/v1/registry/proposals/prp_1/freeze"),
        ("POST", "/v1/registry/proposals/prp_1/evaluate"),
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
        ("GET", "/v1/registry/entities/prompt/p/copiloto"),
        ("POST", "/v1/runs"),
    ] {
        assert!(allowed(m, p), "{m} {p}");
    }
}

#[test]
fn the_allow_list_refuses_query_fragment_and_control_bytes_so_nothing_can_be_smuggled_into_the_request_line() {
    let crlf = format!("{}{}", char::from(13), char::from(10));
    for (m, p) in [
        ("POST", "/v1/runs?x=1".to_string()),
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
    steps.truncate(2);
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
    assert!(a.title().len() <= 200);
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
