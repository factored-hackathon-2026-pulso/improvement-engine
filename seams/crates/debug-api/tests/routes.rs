//! Rust-side mirror of the console contract (debug-console/tests/contract + fixture-server/server.mjs): same routes,
//! same envelopes, same Problem codes, same transport rules. The zod suite itself runs against the live binary.
use debug_api::{App, Config, NewEvent, Req, Resp, RunEventSink, Store};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

const D: &str = "/internal/v1/debug";

fn req(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Req {
    let (p, q) = path.split_once('?').unwrap_or((path, ""));
    Req { method: method.into(), path: p.into(), query: q.into(), headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>(), body: body.as_bytes().to_vec() }
}
fn body(r: &Resp) -> Value {
    serde_json::from_slice(&r.body).unwrap_or(Value::Null)
}
fn get(app: &App, path: &str) -> (u16, Value) {
    let r = app.handle(&req("GET", path, &[], ""));
    (r.status, body(&r))
}
fn header<'a>(r: &'a Resp, name: &str) -> Vec<&'a str> {
    r.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).collect()
}

fn node(id: &str, status: &str, dep: &[&str]) -> NewEvent {
    NewEvent::new("node_status_changed", "node", id, json!({"node": {"node_id": id, "label": id, "stage": "scout", "status": status, "depends_on": dep, "reason_code": null, "node_kind": "material_step", "trace_id": null}}))
}
fn seeded() -> (Arc<Store>, App) {
    let s = Arc::new(Store::memory());
    s.emit("run-a", NewEvent::new("run_started", "run", "run-a", json!({"title": "Run A", "state": "running", "origin": "manual"}))).unwrap();
    s.emit("run-a", node("scout", "complete", &[])).unwrap();
    s.emit("run-a", node("verify", "running", &["scout"])).unwrap();
    let app = App::new(s.clone(), Config::default());
    (s, app)
}

fn assert_envelope(b: &Value) {
    assert_eq!(b["schema_version"], "1");
    assert!(b["tenant_id"].is_string() && b["status"].is_string());
    assert!(b["projection_revision"].as_i64().is_some_and(|n| n >= 0), "{b}");
    assert!(b["blocking_reasons"].is_array() && b["available_commands"].is_array());
}

#[test]
fn healthz_and_unknown_routes() {
    let (_, app) = seeded();
    assert_eq!(get(&app, "/healthz"), (200, json!({"ok": true})));
    let (st, b) = get(&app, &format!("{D}/nope"));
    assert_eq!((st, b["code"].as_str()), (404, Some("not_found")));
}

#[test]
fn session_sets_exactly_one_http_profile_cookie_and_other_routes_set_none() {
    let (_, app) = seeded();
    let r = app.handle(&req("GET", "/api/v1/auth/session", &[], ""));
    assert_eq!(r.status, 200);
    let b = body(&r);
    assert!(b["csrf_token"].as_str().is_some_and(|t| t.len() >= 16));
    assert_eq!(b["auth"]["simulated"], true, "no real identity provider behind this session");
    assert!(b["scopes"].is_array() && b["principal"].is_string() && b["expires_at"].is_string());
    let c = header(&r, "set-cookie");
    assert_eq!(c.len(), 1);
    assert!(c[0].starts_with("pulso_local_session=") && c[0].contains("HttpOnly") && c[0].contains("SameSite=Lax") && c[0].contains("Path=/"));
    assert!(!c[0].contains("Secure") && !c[0].contains("Domain") && !c[0].starts_with("__Host-"));
    assert!(!b.to_string().contains(c[0].split(['=', ';']).nth(1).unwrap()), "cookie value is never echoed in the body");
    for p in [format!("{D}/runs"), format!("{D}/memory"), format!("{D}/profile")] {
        assert!(header(&app.handle(&req("GET", &p, &[], "")), "set-cookie").is_empty(), "{p}");
    }
}

#[test]
fn profile_shows_every_declared_double_with_its_label() {
    let (s, app) = seeded();
    let (_, p0) = get(&app, &format!("{D}/profile"));
    assert_eq!(p0["doubles"], json!([]));
    s.emit("run-a", NewEvent::new("doubles_declared", "run", "run-a", json!({"doubles": [
        {"id": "scout:agent_roleplay", "what": "agent_roleplay (provider agent_roleplay, data_class generated_sample)"},
        {"id": "gate:stand-in", "what": "stand-in", "until": "real gate"},
    ]}))).unwrap();
    let (st, p) = get(&app, &format!("{D}/profile"));
    assert_eq!(st, 200);
    assert_eq!(p["doubles"], json!(["scout:agent_roleplay", "gate:stand-in"]));
    assert_eq!(p["doubles_detail"][1], json!({"id": "gate:stand-in", "what": "stand-in", "until": "real gate"}));
    assert_eq!(p["mode"], "stand_in", "any double means the console must not present this as real");
    assert!(p["target"].is_string() && p["runtime_profile"].is_string() && p["pin"].is_null());
}

#[test]
fn runs_graph_events_have_the_contract_shapes() {
    let (_, app) = seeded();
    let (st, list) = get(&app, &format!("{D}/runs"));
    assert_eq!(st, 200);
    assert_envelope(&list);
    assert_eq!(list["items"], json!([{"run_id": "run-a", "title": "Run A", "state": "running", "origin": "manual", "projection_revision": 3}]));
    assert!(list["next_cursor"].is_null());

    let (st, g) = get(&app, &format!("{D}/runs/run-a/graph"));
    assert_eq!(st, 200);
    assert_envelope(&g);
    assert_eq!(g["projection_revision"], 3);
    assert_eq!(g["status"], "running");
    assert_eq!(g["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(g["nodes"][1]["depends_on"], json!(["scout"]));

    let (st, ev) = get(&app, &format!("{D}/runs/run-a/events?after_sequence=1"));
    assert_eq!(st, 200);
    let seqs: Vec<i64> = ev["items"].as_array().unwrap().iter().map(|e| e["sequence"].as_i64().unwrap()).collect();
    assert_eq!(seqs, vec![2, 3]);
    assert!(ev["next_cursor"].is_null());
    assert_eq!(get(&app, &format!("{D}/runs/run-a/events")).1["items"].as_array().unwrap().len(), 3);
}

#[test]
fn unknown_run_is_a_404_problem() {
    let (_, app) = seeded();
    for tail in ["graph", "events", "investigation", "gates", "alternatives"] {
        let (st, b) = get(&app, &format!("{D}/runs/nope/{tail}"));
        assert_eq!(st, 404, "{tail}");
        assert_eq!(b["code"], "not_found");
        assert!(b["message"].is_string() && b["correlation_id"].is_string() && b["retryable"] == false);
    }
}

#[test]
fn investigation_gates_alternatives_default_to_honest_not_evaluated() {
    let (_, app) = seeded();
    let (_, inv) = get(&app, &format!("{D}/runs/run-a/investigation"));
    assert_envelope(&inv);
    assert!(inv["hypothesis"].is_null());
    assert_eq!((inv["verifier"].as_str(), inv["evidence"].as_array().map(Vec::len)), (Some("unknown"), Some(0)));
    let (_, g) = get(&app, &format!("{D}/runs/run-a/gates"));
    assert_envelope(&g);
    assert_eq!(g["native"]["status"], "not_evaluable");
    assert_eq!(g["improvement"]["status"], "not_evaluable");
    assert_eq!(g["combined"], json!({"decision": "not_applicable", "reason_code": "no_evaluation_in_run"}));
    assert!(g["proposal_id"].is_null() && g["attempts"] == json!([]));
    assert_eq!(get(&app, &format!("{D}/runs/run-a/alternatives")).1["items"], json!([]));
}

#[test]
fn gates_diff_decision_memory_follow_events() {
    let (s, app) = seeded();
    s.emit("run-a", NewEvent::new("gates_set", "run", "run-a", json!({
        "native": {"status": "pass", "reason_code": null, "report_ref": null},
        "improvement": {"status": "insufficient_power", "reason_code": "insufficient_power"},
        "combined": {"decision": "hold", "reason_code": "improvement_gate_not_pass"}, "proposal_id": "prop-9", "attempts": [],
    }))).unwrap();
    s.emit("run-a", NewEvent::new("diff_set", "proposal", "prop-9", json!({"proposal_id": "prop-9", "lines": [{"op": "add", "text": "x: 1"}]}))).unwrap();
    s.emit("run-a", NewEvent::new("decision_set", "decision", "dec-9", json!({"decision_id": "dec-9", "available_commands": [], "needs_step_up": true, "domain_revision": 3}))).unwrap();
    s.emit("run-a", NewEvent::new("memory_set", "memory", "m", json!({"items": [{"memory_id": "mem-1", "title": "T", "status": "active", "revoked": false}]}))).unwrap();
    s.emit("run-a", NewEvent::new("alternatives_set", "run", "run-a", json!({"items": [{"id": "a1", "kind": "do_nothing", "summary": "s"}]}))).unwrap();

    let (_, g) = get(&app, &format!("{D}/runs/run-a/gates"));
    assert_eq!((g["native"]["status"].as_str(), g["proposal_id"].as_str(), g["combined"]["decision"].as_str()), (Some("pass"), Some("prop-9"), Some("hold")));
    let (st, d) = get(&app, &format!("{D}/proposals/prop-9/diff"));
    assert_eq!(st, 200);
    assert_envelope(&d);
    assert_eq!(d["lines"], json!([{"op": "add", "text": "x: 1"}]));
    assert_eq!(get(&app, &format!("{D}/proposals/nope/diff")).0, 404);
    let (st, dec) = get(&app, &format!("{D}/decisions/dec-9"));
    assert_eq!(st, 200);
    assert_envelope(&dec);
    assert_eq!((dec["needs_step_up"].as_bool(), dec["domain_revision"].as_i64(), dec["available_commands"].clone()), (Some(true), Some(3), json!([])));
    assert_eq!(get(&app, &format!("{D}/decisions/nope")).0, 404);
    assert_eq!(get(&app, &format!("{D}/memory")).1["items"][0]["memory_id"], "mem-1");
    assert_eq!(get(&app, &format!("{D}/runs/run-a/alternatives")).1["items"][0]["kind"], "do_nothing");
}

#[test]
fn spec25_panel_routes_are_empty_pages_not_invented_data() {
    let (_, app) = seeded();
    for tail in ["model-calls", "queries", "evals", "external-commands"] {
        let (st, b) = get(&app, &format!("{D}/runs/run-a/{tail}"));
        assert_eq!(st, 200, "{tail}");
        assert_envelope(&b);
        assert_eq!(b["items"], json!([]));
    }
    let (st, b) = get(&app, &format!("{D}/health/dependencies"));
    assert_eq!((st, b["items"].clone()), (200, json!([])));
    assert_eq!(get(&app, &format!("{D}/runs/nope/model-calls")).0, 404);
}

fn csrf(app: &App) -> String {
    body(&app.handle(&req("GET", "/api/v1/auth/session", &[], "")))["csrf_token"].as_str().unwrap().to_string()
}
fn post(app: &App, headers: &[(&str, &str)], b: &str) -> (u16, Value) {
    let mut h = vec![("content-type", "application/json")];
    h.extend_from_slice(headers);
    let r = app.handle(&req("POST", &format!("{D}/decisions/dec-1/responses"), &h, b));
    (r.status, body(&r))
}

#[test]
fn commands_are_validated_like_the_contract_and_never_executed() {
    let (s, app) = seeded();
    s.emit("run-a", NewEvent::new("decision_set", "decision", "dec-1", json!({"decision_id": "dec-1", "available_commands": [], "needs_step_up": false, "domain_revision": 1}))).unwrap();
    let ok = r#"{"expected_revision":1,"response":"approve","note":"n"}"#;
    let t = csrf(&app);
    assert_eq!(post(&app, &[("idempotency-key", "k")], ok).0, 403);
    assert_eq!(post(&app, &[("idempotency-key", "k")], ok).1["code"], "csrf_failed");
    assert_eq!(post(&app, &[("x-csrf-token", &t)], ok).0, 422, "missing Idempotency-Key");
    let (st, b) = post(&app, &[("x-csrf-token", &t), ("idempotency-key", "k")], r#"{"expected_revision":1,"response":"approve","note":{"a":1}}"#);
    assert_eq!(st, 422);
    assert!(b.to_string().contains("note"), "{b}");
    let (st, b) = post(&app, &[("x-csrf-token", &t), ("idempotency-key", "k")], r#"{"response":"approve","note":"n"}"#);
    assert_eq!(st, 422);
    assert!(b.to_string().contains("expected_revision"));
    let (st, b) = post(&app, &[("x-csrf-token", &t), ("idempotency-key", "k")], r#"{"expected_revision":9,"response":"approve","note":"n"}"#);
    assert_eq!((st, b["code"].as_str()), (409, Some("stale_revision")));
    assert_eq!(b["conflict"], json!({"expected_revision": 9, "current_revision": 1, "diff_ref": null}));
    let (st, b) = post(&app, &[("x-csrf-token", &t), ("idempotency-key", "k")], ok);
    assert_eq!((st, b["code"].as_str()), (403, Some("command_not_available")), "read-only by default: available_commands[] is empty");
    assert_eq!(s.head("run-a"), Some(4), "nothing was written");
}

#[test]
fn local_dev_token_gates_the_debug_routes_but_not_healthz() {
    let s = Arc::new(Store::memory());
    let app = App::new(s, Config { token: Some("dev-token".into()), ..Config::default() });
    assert_eq!(app.handle(&req("GET", "/healthz", &[], "")).status, 200);
    assert_eq!(get(&app, &format!("{D}/runs")).0, 401);
    assert_eq!(app.handle(&req("GET", &format!("{D}/runs"), &[("authorization", "Bearer wrong")], "")).status, 401);
    assert_eq!(app.handle(&req("GET", &format!("{D}/runs"), &[("authorization", "Bearer dev-token")], "")).status, 200);
}

#[test]
fn admin_surface_does_not_exist_unless_configured_and_needs_its_token() {
    let (s, app) = seeded();
    let path = "/__admin/v1/runs/run-a/events";
    let ev = r#"{"kind":"node_status_changed","entity_kind":"node","entity_id":"x","data":{"node":{"node_id":"x","label":"x","stage":"scout","status":"running","depends_on":[],"reason_code":null,"node_kind":"material_step","trace_id":null}}}"#;
    assert_eq!(app.handle(&req("POST", path, &[], ev)).status, 404, "off by default");
    let on = App::new(s.clone(), Config { admin_token: Some("adm".into()), ..Config::default() });
    assert_eq!(on.handle(&req("POST", path, &[], ev)).status, 401);
    assert_eq!(on.handle(&req("POST", path, &[("authorization", "Bearer nope")], ev)).status, 401);
    let r = on.handle(&req("POST", path, &[("authorization", "Bearer adm")], ev));
    assert_eq!(r.status, 200);
    assert_eq!(body(&r)["sequence"], 4);
    assert_eq!(s.head("run-a"), Some(4));
    assert_eq!(on.handle(&req("POST", path, &[("authorization", "Bearer adm")], "{not json")).status, 422);
    let r = on.handle(&req("POST", "/__admin/v1/runs/run-a/purge", &[("authorization", "Bearer adm")], r#"{"through":2}"#));
    assert_eq!(r.status, 200);
    assert_eq!(s.floor("run-a"), Some(2));
}

fn plan(app: &App, path: &str, headers: &[(&str, &str)]) -> Result<(String, i64), (u16, Value)> {
    match app.stream_route(&req("GET", path, headers, "")).expect("a stream route") {
        Ok(p) => Ok((p.run, p.after)),
        Err(r) => Err((r.status, body(&r))),
    }
}

#[test]
fn stream_routes_resume_from_last_event_id_or_after_sequence_the_larger_wins() {
    let (_, app) = seeded();
    for p in [format!("{D}/runs/run-a/events/stream"), format!("{D}/runs/run-a/stream")] {
        assert_eq!(plan(&app, &p, &[]), Ok(("run-a".into(), 0)));
        assert_eq!(plan(&app, &p, &[("last-event-id", "2")]), Ok(("run-a".into(), 2)));
        assert_eq!(plan(&app, &format!("{p}?after_sequence=1"), &[("last-event-id", "2")]), Ok(("run-a".into(), 2)));
        assert_eq!(plan(&app, &format!("{p}?after_sequence=3"), &[("last-event-id", "2")]), Ok(("run-a".into(), 3)));
    }
    assert!(app.stream_route(&req("GET", &format!("{D}/runs/run-a/graph"), &[], "")).is_none());
    assert!(app.stream_route(&req("POST", &format!("{D}/runs/run-a/stream"), &[], "")).is_none());
    assert_eq!(plan(&app, &format!("{D}/runs/nope/events/stream"), &[]).unwrap_err().0, 404);
    assert_eq!(plan(&app, &format!("{D}/runs/run-a/stream"), &[("last-event-id", "x")]).unwrap_err().0, 400);
}

#[test]
fn a_purged_or_unknown_future_cursor_is_410_with_both_recovery_shapes() {
    let (s, app) = seeded();
    s.purge_through("run-a", 2).unwrap();
    let (st, b) = plan(&app, &format!("{D}/runs/run-a/events/stream"), &[("last-event-id", "1")]).unwrap_err();
    assert_eq!(st, 410);
    assert_eq!(b["code"], "cursor_expired");
    assert!(b["message"].is_string() && b["correlation_id"].is_string() && b["retryable"] == false);
    assert_eq!(b["recovery_after_sequence"], 3, "snapshot is at the head");
    assert_eq!(b["snapshot_url"], format!("{D}/runs/run-a/graph"));
    assert_eq!(b["recovery"], json!({"snapshot_ref": format!("{D}/runs/run-a/graph"), "after_sequence": 3}));
    assert_eq!(b["current_ref"], json!({"kind": "run", "id": "run-a"}));
    // exactly the floor is resumable; beyond the head is a different history (e.g. a restarted server)
    assert_eq!(plan(&app, &format!("{D}/runs/run-a/stream"), &[("last-event-id", "2")]), Ok(("run-a".into(), 2)));
    assert_eq!(plan(&app, &format!("{D}/runs/run-a/stream"), &[("last-event-id", "99")]).unwrap_err().0, 410);
    // the events page honours the same floor
    assert_eq!(get(&app, &format!("{D}/runs/run-a/events?after_sequence=0")).0, 410);
    assert_eq!(get(&app, &format!("{D}/runs/run-a/events?after_sequence=2")).0, 200);
}
