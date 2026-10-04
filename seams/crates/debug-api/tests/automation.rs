//! /internal/v1/automation: case-type maturity read model, team thresholds (admin + audit) and the SIMULATED approve /
//! publish-to-staging path. Fixtures are the simulator's SIM-ONLY draft stream and a synthetic proposal.
use debug_api::automation::Automation;
use debug_api::{App, Config, Req, Resp, Store};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

const A: &str = "/internal/v1/automation";
const SIM: &str = include_str!("fixtures/automation_sim_draft_stream.json");
const PROP: &str = include_str!("fixtures/automation_proposals.json");

fn req(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Req {
    Req { method: method.into(), path: path.into(), query: String::new(), headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>(), body: body.as_bytes().to_vec() }
}
fn body(r: &Resp) -> Value {
    serde_json::from_slice(&r.body).unwrap_or(Value::Null)
}
fn app_with(auto: Automation, cfg: Config) -> (Arc<Store>, App) {
    let s = Arc::new(Store::memory());
    (s.clone(), App::new(s, Config { automation: Some(Arc::new(auto)), ..cfg }))
}
fn full() -> Automation {
    let sim: Value = serde_json::from_str(SIM).unwrap();
    let prop: Value = serde_json::from_str(PROP).unwrap();
    Automation::from_json(None, Some(&sim), Some(&prop)).unwrap()
}
fn get(app: &App, p: &str) -> (u16, Value) {
    let r = app.handle(&req("GET", p, &[], ""));
    (r.status, body(&r))
}
fn item<'a>(b: &'a Value, id: &str) -> &'a Value {
    b["case_types"].as_array().unwrap().iter().find(|c| c["type_id"] == id).unwrap()
}

#[test]
fn not_configured_means_404() {
    let app = App::new(Arc::new(Store::memory()), Config::default());
    assert_eq!(get(&app, &format!("{A}/case-types")).0, 404);
}

#[test]
fn list_shows_stages_sources_banner_and_what_is_simulated() {
    let (_, app) = app_with(full(), Config::default());
    let (st, b) = get(&app, &format!("{A}/case-types"));
    assert_eq!(st, 200);
    assert_eq!(b["data_origin"], "simulated");
    assert_eq!(b["doubles"][0]["id"], "sim_draft_stream");
    let ids: Vec<&str> = b["case_types"].as_array().unwrap().iter().map(|c| c["type_id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["cobro_indebido", "cargo_no_reconocido", "problema_app", "calidad_servicio", "atencion_sucursal", "tarjeta_virtual"]);
    let stages: Vec<Value> = b["case_types"].as_array().unwrap().iter().map(|c| c["stage"].clone()).collect();
    assert_eq!(stages, [json!(3), json!("agent"), json!(2), json!(1), json!(1), json!(0)]);
    let c = item(&b, "cobro_indebido");
    assert_eq!(c["agent_proposed"], true);
    assert_eq!(c["measure"]["kind"], "draft_accept_100");
    assert_eq!((c["measure"]["numerator"].as_u64(), c["measure"]["denominator"].as_u64()), (Some(84), Some(100)));
    assert_eq!((c["measure"]["source"].as_str(), c["measure"]["simulated"].as_bool()), (Some("sim_draft_stream"), Some(true)));
    assert_eq!(c["cases_today"], 21);
    assert_eq!(item(&b, "cargo_no_reconocido")["measure"]["kind"], "agent_resolved");
    assert_eq!(b["banner"]["type_id"], "cobro_indebido");
    assert_eq!(b["banner"]["proposal_id"], "prop-cobro-001");
    assert_eq!(b["banner"]["simulated"], true);
    assert_eq!(b["thresholds"]["repeat_q_min_cases"], 20);
}

#[test]
fn detail_has_history_drafts_window_thresholds_today_and_the_proposal_target() {
    let (_, app) = app_with(full(), Config::default());
    let (st, b) = get(&app, &format!("{A}/case-types/cobro_indebido"));
    assert_eq!(st, 200);
    let h: Vec<&str> = b["history"].as_array().unwrap().iter().map(|x| x["stage"].as_str().unwrap()).collect();
    assert_eq!(h, ["1", "2", "3", "agent_proposed"]);
    assert_eq!(b["history"][0]["since"], "2026-08-04");
    let d = &b["drafts_last_100"];
    assert_eq!((d["as_is"].as_u64(), d["minor"].as_u64(), d["discarded"].as_u64(), d["simulated"].as_bool()), (Some(61), Some(23), Some(16), Some(true)));
    let t = b["thresholds_today"].as_array().unwrap();
    assert_eq!(t.len(), 3);
    assert!(t.iter().all(|x| x["met"] == true));
    assert_eq!(t[0]["today"]["numerator"], 41);
    let p = &b["proposal"];
    assert_eq!(p["proposal_id"], "prop-cobro-001");
    assert_eq!(p["target"]["agent_id"], "copiloto-asesor");
    assert_eq!(p["target"]["artifact"], "p/copiloto@1.0.0");
    assert_eq!(p["state"], "proposed");
    assert_eq!(p["simulated"], true);
    assert_eq!(p["links"]["run"], "run-cobro-001");
    assert_eq!(get(&app, &format!("{A}/case-types/zzz")).0, 404);
}

#[test]
fn e0_only_is_honest_draft_acceptance_is_not_computable_and_no_banner() {
    let e0 = json!({"case_types": [{"type_id": "cobro_indebido", "copilot_questions": 200, "repeat_q_cases": 154, "tool_applicable": 154, "tool_used": 120}]});
    let (_, app) = app_with(Automation::from_json(Some(&e0), None, None).unwrap(), Config::default());
    let (_, b) = get(&app, &format!("{A}/case-types"));
    assert_eq!(b["data_origin"], "e0_treated");
    assert!(b["banner"].is_null());
    let c = item(&b, "cobro_indebido");
    assert_eq!(c["stage"], 3);
    assert_eq!(c["agent_proposed"], false);
    assert_eq!(c["simulated"], false);
    assert_eq!(c["metrics"]["draft_accept_100"]["status"], "not_computable");
    assert_eq!(c["blocked_by"], "draft_accept_100: no_draft_rows");
    let (_, d) = get(&app, &format!("{A}/case-types/cobro_indebido"));
    assert_eq!(d["drafts_last_100"]["status"], "not_computable");
}

#[test]
fn e0_real_metrics_win_over_the_simulated_ones_and_each_keeps_its_badge() {
    let e0 = json!({"case_types": [{"type_id": "cobro_indebido", "copilot_questions": 200, "repeat_q_cases": 154, "tool_applicable": 154, "tool_used": 120}]});
    let sim: Value = serde_json::from_str(SIM).unwrap();
    let (_, app) = app_with(Automation::from_json(Some(&e0), Some(&sim), None).unwrap(), Config::default());
    let (_, b) = get(&app, &format!("{A}/case-types/cobro_indebido"));
    assert_eq!(b["metrics"]["repeat_q"]["source"], "e0_treated");
    assert_eq!(b["metrics"]["repeat_q"]["simulated"], false);
    assert_eq!(b["metrics"]["draft_accept_100"]["source"], "sim_draft_stream");
    let (_, l) = get(&app, &format!("{A}/case-types"));
    assert_eq!(l["data_origin"], "mixed");
    assert_eq!(l["doubles"].as_array().unwrap().len(), 1);
}

#[test]
fn bearer_token_is_required_when_configured() {
    let (_, app) = app_with(full(), Config { token: Some("t".into()), ..Config::default() });
    assert_eq!(app.handle(&req("GET", &format!("{A}/case-types"), &[], "")).status, 401);
    assert_eq!(app.handle(&req("GET", &format!("{A}/case-types"), &[("authorization", "Bearer t")], "")).status, 200);
}

#[test]
fn put_config_needs_admin_validates_audits_and_recomputes() {
    let (s, app) = app_with(full(), Config { admin_token: Some("adm".into()), ..Config::default() });
    let put = |h: &[(&str, &str)], b: &str| app.handle(&req("PUT", &format!("{A}/config"), h, b));
    assert_eq!(put(&[], r#"{"repeat_q_min_cases":50}"#).status, 401);
    assert_eq!(put(&[("authorization", "Bearer adm")], r#"{"k_min":3}"#).status, 422);
    assert_eq!(put(&[("authorization", "Bearer adm")], r#"{"nope":1}"#).status, 422);
    let r = put(&[("authorization", "Bearer adm")], r#"{"repeat_q_min_cases":50}"#);
    assert_eq!(r.status, 200);
    assert_eq!(body(&r)["thresholds"]["repeat_q_min_cases"], 50);
    assert_eq!(body(&r)["revision"], 1);
    let (_, b) = get(&app, &format!("{A}/case-types"));
    assert_eq!(item(&b, "cobro_indebido")["stage"], 1);
    assert!(b["banner"].is_null());
    let ev = s.events_after("automation-audit", 0, 10);
    assert!(ev.iter().any(|e| e["kind"] == "automation_config_changed" && e["data"]["after"]["repeat_q_min_cases"] == 50 && e["data"]["before"]["repeat_q_min_cases"] == 20), "{ev:?}");
}

#[test]
fn put_config_does_not_exist_without_an_admin_token() {
    let (_, app) = app_with(full(), Config::default());
    assert_eq!(app.handle(&req("PUT", &format!("{A}/config"), &[], "{}")).status, 404);
}

#[test]
fn approve_is_simulated_needs_csrf_and_publish_needs_approval_first() {
    let (s, app) = app_with(full(), Config::default());
    let csrf = body(&app.handle(&req("GET", "/api/v1/auth/session", &[], "")))["csrf_token"].as_str().unwrap().to_string();
    let ap = format!("{A}/case-types/cobro_indebido/proposal/approve");
    let pu = format!("{A}/case-types/cobro_indebido/proposal/publish-staging");
    let hash = r#"{"candidate_hash":"sha256:0f3a9c51d1b7e2a4c6b8d0e2f4a6c8e0a2b4d6f8091a2b3c4d5e6f708192a3b4","step_up":"simulated"}"#;
    assert_eq!(app.handle(&req("POST", &ap, &[], hash)).status, 403);
    assert_eq!(app.handle(&req("POST", &pu, &[("x-csrf-token", &csrf)], "{}")).status, 409);
    assert_eq!(app.handle(&req("POST", &ap, &[("x-csrf-token", &csrf)], r#"{"candidate_hash":"sha256:bad"}"#)).status, 409);
    let r = app.handle(&req("POST", &ap, &[("x-csrf-token", &csrf)], hash));
    assert_eq!(r.status, 200);
    assert_eq!((body(&r)["state"].as_str(), body(&r)["simulated"].as_bool()), (Some("approved_simulated"), Some(true)));
    let r = app.handle(&req("POST", &pu, &[("x-csrf-token", &csrf)], "{}"));
    assert_eq!((r.status, body(&r)["state"].as_str()), (200, Some("staged_simulated")));
    assert_eq!(get(&app, &format!("{A}/case-types/cobro_indebido")).1["proposal"]["state"], "staged_simulated");
    let ev = s.events_after("automation-audit", 0, 20);
    assert!(ev.iter().any(|e| e["kind"] == "automation_proposal_approved_simulated"));
    assert!(ev.iter().any(|e| e["kind"] == "automation_publish_staging_simulated"));
}
