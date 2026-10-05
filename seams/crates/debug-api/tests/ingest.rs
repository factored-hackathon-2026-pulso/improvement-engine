use debug_api::ingest::{contract_seed, engine_run_report};
use debug_api::{App, Config, Req, Resp, Store};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

const D: &str = "/internal/v1/debug";
const REPORT: &str = include_str!("fixtures/engine-run-demo0.json");

fn report() -> Value {
    serde_json::from_str(REPORT).unwrap()
}
fn get(app: &App, path: &str) -> (u16, Value) {
    let r: Resp = app.handle(&Req { method: "GET".into(), path: path.into(), query: String::new(), headers: HashMap::new(), body: vec![] });
    (r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null))
}
fn ingest(s: &Store, r: &Value) -> Result<String, String> {
    engine_run_report(s, &|id| s.head(id).is_some(), r)
}

#[test]
fn a_report_becomes_one_finished_run_with_every_step_labelled() {
    let s = Arc::new(Store::memory());
    let id = ingest(&s, &report()).unwrap();
    assert_eq!(id, "run-demo-0-aa2f9472");
    let app = App::new(s.clone(), Config::default());
    let (_, list) = get(&app, &format!("{D}/runs"));
    let item = &list["items"][0];
    assert_eq!((item["run_id"].as_str(), item["state"].as_str()), (Some(id.as_str()), Some("completed")));
    assert!(item["title"].as_str().unwrap().contains("DEMO-0"), "{item}");

    let (_, g) = get(&app, &format!("{D}/runs/{id}/graph"));
    let nodes = g["nodes"].as_array().unwrap();
    let steps = report()["steps"].as_array().unwrap().clone();
    assert_eq!(nodes.len(), steps.len());
    for (n, st) in nodes.iter().zip(&steps) {
        assert_eq!(n["node_id"], st["id"]);
        assert!(n["label"].as_str().unwrap().contains(&format!("[{}]", st["status"].as_str().unwrap())), "the honesty label is part of the visible label: {n}");
    }
    assert_eq!(nodes[1]["depends_on"], json!(["trigger"]));
    let revision = nodes.iter().find(|n| n["node_id"] == "revision").unwrap();
    assert_eq!((revision["status"].as_str(), revision["reason_code"].as_str()), (Some("planned"), Some("not_exercised")));
    assert_eq!(nodes[0]["status"], "complete");
    assert!(nodes.iter().all(|n| n["trace_id"].is_null()));
}

#[test]
fn every_double_of_the_report_is_shown_none_hidden() {
    let s = Arc::new(Store::memory());
    ingest(&s, &report()).unwrap();
    let app = App::new(s, Config::default());
    let (_, p) = get(&app, &format!("{D}/profile"));
    assert_eq!(p["mode"], "stand_in");
    let shown: Vec<String> = p["doubles"].as_array().unwrap().iter().map(|d| d.as_str().unwrap().to_string()).collect();
    for d in report()["doubles"].as_array().unwrap() {
        let (part, status) = (d["part"].as_str().unwrap(), d["status"].as_str().unwrap());
        assert!(shown.contains(&format!("{part}:{status}")), "{part}:{status} missing from {shown:?}");
    }
    assert!(shown.contains(&"scout:agent_roleplay".to_string()));
    let detail = p["doubles_detail"].as_array().unwrap();
    assert_eq!(detail.len(), shown.len());
    assert!(detail.iter().find(|d| d["id"] == "scout:agent_roleplay").unwrap()["what"].as_str().unwrap().contains("agent_roleplay"));
}

#[test]
fn the_gate_is_reported_as_stated_and_the_improvement_gate_is_not_invented() {
    let s = Arc::new(Store::memory());
    let id = ingest(&s, &report()).unwrap();
    let app = App::new(s, Config::default());
    let (_, g) = get(&app, &format!("{D}/runs/{id}/gates"));
    assert_eq!(g["native"]["status"], "pass");
    assert_eq!(g["improvement"]["status"], "not_evaluable");
    assert_eq!(g["combined"]["decision"], "hold", "a structural stand-in verdict never opens the combined gate");
}

#[test]
fn a_report_is_ingested_once_and_a_quality_claim_is_refused() {
    let s = Store::memory();
    ingest(&s, &report()).unwrap();
    assert!(ingest(&s, &report()).unwrap_err().contains("already"));
    let mut claim = report();
    claim["quality_claims"] = json!("allowed");
    claim["sha"] = json!("bbbbbbbbbbbb");
    assert!(ingest(&s, &claim).is_err());
    assert!(ingest(&s, &json!({"label": "DEMO-0"})).is_err());
    assert!(ingest(&s, &json!({"label": "DEMO-0", "sha": "abc", "quality_claims": "forbidden", "steps": []})).is_err());
}

#[test]
fn contract_seed_provides_dec_1_and_prop_1_and_says_it_is_not_an_engine_run() {
    let s = Arc::new(Store::memory());
    let id = contract_seed(&*s).unwrap();
    let app = App::new(s, Config::default());
    let (st, d) = get(&app, &format!("{D}/decisions/dec-1"));
    assert_eq!((st, d["available_commands"].clone(), d["domain_revision"].as_i64()), (200, json!([]), Some(1)));
    assert_eq!(get(&app, &format!("{D}/proposals/prop-1/diff")).0, 200);
    let (_, list) = get(&app, &format!("{D}/runs"));
    assert!(list["items"][0]["title"].as_str().unwrap().contains("not an engine run"));
    assert_eq!(id, "run-active", "the contract suite reads the graph of run-active unconditionally");
    let (_, graph) = get(&app, &format!("{D}/runs/run-active/graph"));
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert!(graph["nodes"].as_array().unwrap().iter().all(|n| n["trace_id"].is_null()));
    let (_, g) = get(&app, &format!("{D}/runs/{id}/gates"));
    assert_eq!(g["proposal_id"], "prop-1");
    let (_, p) = get(&app, &format!("{D}/profile"));
    assert!(p["doubles"].as_array().unwrap().iter().any(|d| d == "contract_seed:fixture"));
}

#[test]
fn admin_ingest_route_takes_a_report_body() {
    let s = Arc::new(Store::memory());
    let app = App::new(s.clone(), Config { admin_token: Some("adm".into()), ..Config::default() });
    let h: HashMap<String, String> = [("authorization".to_string(), "Bearer adm".to_string())].into();
    let r = app.handle(&Req { method: "POST".into(), path: "/__admin/v1/ingest/engine-run".into(), query: String::new(), headers: h.clone(), body: REPORT.as_bytes().to_vec() });
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap()["run_id"], "run-demo-0-aa2f9472");
    let again = app.handle(&Req { method: "POST".into(), path: "/__admin/v1/ingest/engine-run".into(), query: String::new(), headers: h, body: REPORT.as_bytes().to_vec() });
    assert_eq!(again.status, 409);
}

#[test]
fn a_double_without_part_or_status_is_not_silently_dropped() {
    let s = Arc::new(Store::memory());
    let mut r = report();
    r["doubles"].as_array_mut().unwrap().push(json!({"part": "mystery", "note": "no status given"}));
    r["doubles"].as_array_mut().unwrap().push(json!({"status": "fake_only"}));
    // either refused outright or shown (never hidden)
    match ingest(&s, &r) {
        Err(_) => {}
        Ok(_) => {
            let app = App::new(s, Config::default());
            let (_, p) = get(&app, &format!("{D}/profile"));
            let shown = p["doubles"].to_string();
            assert!(shown.contains("mystery") && shown.contains("fake_only"), "dropped: {shown}");
        }
    }
}

fn with_committed() -> Value {
    let mut r = report();
    r["committed"] = serde_json::from_str(include_str!("fixtures/committed-demo0.json")).unwrap();
    r
}

#[test]
fn a_report_that_carries_its_committed_payload_fills_the_panels_with_the_same_projection_the_live_stream_uses() {
    let s = Arc::new(Store::memory());
    let id = ingest(&s, &with_committed()).unwrap();
    let app = App::new(s.clone(), Config::default());
    let (_, inv) = get(&app, &format!("{D}/runs/{id}/investigation"));
    assert!(inv["hypothesis"].as_str().unwrap().contains("Scripted scout claims"), "{inv}");
    assert_eq!(inv["verifier"], "corroborated");
    assert_eq!(inv["hypotheses"].as_array().unwrap().len(), 2);
    let (_, g) = get(&app, &format!("{D}/runs/{id}/gates"));
    assert_eq!((g["improvement"]["status"].as_str(), g["improvement"]["reason_code"].as_str()), (Some("fail"), Some("no_structural_improvement")));
    let pid = g["proposal_id"].as_str().unwrap().to_string();
    let (st, d) = get(&app, &format!("{D}/proposals/{pid}/diff"));
    assert_eq!(st, 200);
    assert!(d["lines"].to_string().contains("prompt:resumen_radicado@2"));
    let (st, dec) = get(&app, &format!("{D}/runs/{id}/decision"));
    assert_eq!(st, 200, "{dec}");
    assert_eq!((dec["card"]["label"].as_str(), dec["card"]["state"].as_str()), (Some("SIMULATED"), Some("approved")));
    assert_eq!(dec["entity_ref"], json!({"kind": "decision", "id": dec["decision_id"]}));
    let (_, alts) = get(&app, &format!("{D}/runs/{id}/alternatives"));
    assert_eq!(alts["items"].as_array().unwrap().len(), 2);
    // the events are exactly those of the shared projection (one projection, two producers)
    let kinds: Vec<String> = s.events_after(&id, 0, 1000).iter().map(|e| e["kind"].as_str().unwrap_or("").to_string()).collect();
    for k in ["investigation_set", "alternatives_set", "diff_set", "gates_set", "decision_set"] {
        assert_eq!(kinds.iter().filter(|x| *x == k).count(), 1, "{k} once in {kinds:?}");
    }
}

#[test]
fn a_report_without_the_committed_payload_keeps_the_panels_honestly_empty() {
    let s = Arc::new(Store::memory());
    let id = ingest(&s, &report()).unwrap();
    let app = App::new(s, Config::default());
    let (_, inv) = get(&app, &format!("{D}/runs/{id}/investigation"));
    assert!(inv["hypothesis"].is_null() && inv["verifier"] == "unknown");
    assert_eq!(get(&app, &format!("{D}/runs/{id}/decision")).0, 404, "no decision was committed, none is invented");
}
