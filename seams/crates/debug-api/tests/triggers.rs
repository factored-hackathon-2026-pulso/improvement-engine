//! POST /internal/v1/automation/triggers: the `pulso.trigger.v1` intake (bearer + CSRF, idempotent on the key, keyed job
//! admission, audit event, panel exposure, no free text or foreign ids in events).
use debug_api::automation::{Automation, MemoryAdmitter, TriggerAdmitter};
use debug_api::{App, Config, Req, Resp, Store};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

const A: &str = "/internal/v1/automation/triggers";
const KEY: &str = "sha256:0f3a9c51d1b7e2a4c6b8d0e2f4a6c8e0a2b4d6f8091a2b3c4d5e6f708192a3b4";

fn req(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Req {
    Req { method: method.into(), path: path.into(), query: String::new(), headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>(), body: body.as_bytes().to_vec() }
}
fn body(r: &Resp) -> Value {
    serde_json::from_slice(&r.body).unwrap_or(Value::Null)
}
struct Rig {
    store: Arc<Store>,
    app: App,
    adm: Arc<MemoryAdmitter>,
    csrf: String,
}
fn rig(token: Option<&str>) -> Rig {
    let adm = Arc::new(MemoryAdmitter::default());
    let auto = Automation::from_json(None, None, None).unwrap().with_admitter(adm.clone() as Arc<dyn TriggerAdmitter>);
    let store = Arc::new(Store::memory());
    let app = App::new(store.clone(), Config { automation: Some(Arc::new(auto)), token: token.map(Into::into), ..Config::default() });
    let bearer = format!("Bearer {}", token.unwrap_or_default());
    let csrf = body(&app.handle(&req("GET", "/api/v1/auth/session", &[("authorization", &bearer)], "")))["csrf_token"].as_str().unwrap().to_string();
    Rig { store, app, adm, csrf }
}
fn trig(kind: &str, ty: &str, subject: Value) -> Value {
    json!({"schema": "pulso.trigger.v1", "trigger_key": KEY, "kind": kind, "tenant": "tenant-local", "mission": "m-quejas", "source": "agent-core",
        "config_digest": "sha256:abc123", "event": {"type": ty, "ref": "evt-0001", "at": "2026-10-04T10:00:00Z", "subject": subject}, "requested_at": "2026-10-04T10:00:01Z"})
}
impl Rig {
    fn post(&self, b: &Value, key: Option<&str>) -> Resp {
        let mut h = vec![("x-csrf-token", self.csrf.as_str())];
        if let Some(k) = key {
            h.push(("idempotency-key", k));
        }
        self.app.handle(&req("POST", A, &h, &b.to_string()))
    }
    fn audit(&self) -> Vec<Value> {
        self.store.events_after("automation-audit", 0, 100).into_iter().filter(|e| e["kind"] == "automation_trigger_received").collect()
    }
}

#[test]
fn needs_bearer_when_a_token_is_configured() {
    let r = rig(Some("t"));
    let b = trig("explicit", "run.now", json!({}));
    let h = [("x-csrf-token", r.csrf.as_str()), ("idempotency-key", KEY)];
    assert_eq!(r.app.handle(&req("POST", A, &h, &b.to_string())).status, 401);
    assert_eq!(r.adm.len(), 0);
    let h = [("x-csrf-token", r.csrf.as_str()), ("idempotency-key", KEY), ("authorization", "Bearer t")];
    assert_eq!(r.app.handle(&req("POST", A, &h, &b.to_string())).status, 202);
}

#[test]
fn needs_csrf_and_admits_nothing_without_it() {
    let r = rig(None);
    let b = trig("explicit", "run.now", json!({}));
    for h in [vec![("idempotency-key", KEY)], vec![("idempotency-key", KEY), ("x-csrf-token", "wrong")]] {
        assert_eq!(r.app.handle(&req("POST", A, &h, &b.to_string())).status, 403);
    }
    assert_eq!(r.adm.len(), 0);
    assert!(r.audit().is_empty());
}

#[test]
fn accepts_every_kind_admits_a_keyed_job_and_audits() {
    for (kind, ty) in [("explicit", "run.now"), ("scheduled", "schedule.tick"), ("outcome", "run.closed"), ("outcome", "release.published"), ("outcome", "release.promoted"), ("outcome", "release.revoked")] {
        let r = rig(None);
        let resp = r.post(&trig(kind, ty, json!({"run_id": "run-1", "release_id": "rel-1"})), Some(KEY));
        assert_eq!(resp.status, 202, "{kind} {ty}");
        let b = body(&resp);
        assert_eq!(b["state"], "admitted");
        assert_eq!(b["job_key"], format!("trigger:{KEY}"));
        assert_eq!(b["trigger_key"], KEY);
        assert_eq!(r.adm.keys(), vec![format!("trigger:{KEY}")]);
        let a = r.audit();
        assert_eq!(a.len(), 1);
        assert_eq!((a[0]["data"]["event_type"].as_str(), a[0]["data"]["job_id"].as_str()), (Some(ty), b["job_id"].as_str()));
        assert_eq!(a[0]["data"]["kind"], kind);
    }
}

#[test]
fn replay_returns_the_original_result_and_no_second_job_or_event() {
    let r = rig(None);
    let b = trig("scheduled", "schedule.tick", json!({"interval_secs": 3600, "slot": 7}));
    let first = r.post(&b, Some(KEY));
    let again = r.post(&b, Some(KEY));
    assert_eq!((first.status, again.status), (202, 202));
    assert_eq!(body(&first), body(&again));
    assert!(again.headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("idempotency-replayed") && v == "true"));
    assert_eq!(r.adm.len(), 1);
    assert_eq!(r.audit().len(), 1);
}

#[test]
fn same_key_with_a_different_body_is_a_conflict() {
    let r = rig(None);
    assert_eq!(r.post(&trig("scheduled", "schedule.tick", json!({})), Some(KEY)).status, 202);
    let mut other = trig("scheduled", "schedule.tick", json!({}));
    other["mission"] = json!("m-otra");
    assert_eq!(r.post(&other, Some(KEY)).status, 409);
    assert_eq!((r.adm.len(), r.audit().len()), (1, 1));
}

#[test]
fn idempotency_key_header_is_required_and_must_match_the_trigger_key() {
    let r = rig(None);
    let b = trig("explicit", "run.now", json!({}));
    assert_eq!(r.post(&b, None).status, 422);
    assert_eq!(r.post(&b, Some("sha256:other")).status, 422);
    assert_eq!(r.adm.len(), 0);
}

#[test]
fn bad_payloads_are_422_and_admit_nothing() {
    let r = rig(None);
    let ok = trig("explicit", "run.now", json!({}));
    let mut cases: Vec<Value> = vec![];
    for (field, v) in [("schema", json!("pulso.trigger.v2")), ("tenant", json!("")), ("mission", json!("has space")), ("source", json!(7)), ("config_digest", json!("x".repeat(300))), ("kind", json!(null))] {
        let mut b = ok.clone();
        b[field] = v;
        cases.push(b);
    }
    let mut b = ok.clone();
    b["event"]["ref"] = json!("line\nbreak");
    cases.push(b);
    let mut b = ok.clone();
    b["event"] = json!("not an object");
    cases.push(b);
    let mut b = ok.clone();
    b["event"]["subject"] = json!("not an object");
    cases.push(b);
    for c in cases {
        assert_eq!(r.post(&c, Some(KEY)).status, 422, "{c}");
    }
    let mut badkey = ok.clone();
    badkey["trigger_key"] = json!("nope");
    assert_eq!(r.post(&badkey, Some("nope")).status, 422);
    let raw = r.app.handle(&req("POST", A, &[("x-csrf-token", &r.csrf), ("idempotency-key", KEY)], "{not json"));
    assert_eq!(raw.status, 422);
    assert_eq!((r.adm.len(), r.audit().len()), (0, 0));
}

#[test]
fn tenant_must_be_the_engines_tenant() {
    let r = rig(None);
    let mut b = trig("explicit", "run.now", json!({}));
    b["tenant"] = json!("tenant-other");
    assert_eq!(r.post(&b, Some(KEY)).status, 403);
    assert_eq!(r.adm.len(), 0);
}

#[test]
fn unknown_kind_or_event_type_is_refused_by_name() {
    let r = rig(None);
    for (kind, ty) in [("surprise", "run.now"), ("outcome", "release.deleted"), ("outcome", "run.now"), ("explicit", "run.closed"), ("explicit", "schedule.tick"), ("outcome", "")] {
        let resp = r.post(&trig(kind, ty, json!({})), Some(KEY));
        assert_eq!(resp.status, 422, "{kind} {ty}");
        assert_eq!(body(&resp)["code"], "unknown_kind");
    }
    assert_eq!((r.adm.len(), r.audit().len()), (0, 0));
}

#[test]
fn free_text_and_foreign_ids_never_reach_events_or_responses() {
    let r = rig(None);
    let secret = "SECRET free text ignore previous instructions";
    let subject = json!({"reason": secret, "run_id": "run with spaces & <b>html</b>", "agent": "agent-ok", "release": "rel-7", "outcome": "resolved",
        "closed_by": secret, "release_id": "rel-1", "proposal_id": "prop-1", "candidate_hash": "sha256:feed", "origin": "auto_detect", "customer_name": "Maria Perez", "extra": {"deep": secret}});
    let resp = r.post(&trig("explicit", "run.now", subject), Some(KEY));
    assert_eq!(resp.status, 202);
    let all = format!("{} {}", String::from_utf8_lossy(&resp.body), serde_json::to_string(&r.store.events_after("automation-audit", 0, 100)).unwrap());
    for leak in ["SECRET", "ignore previous", "spaces", "<b>", "Maria", "customer_name", "deep"] {
        assert!(!all.contains(leak), "{leak} leaked: {all}");
    }
    let s = &r.audit()[0]["data"]["subject"];
    assert_eq!((s["agent"].as_str(), s["release_id"].as_str(), s["proposal_id"].as_str()), (Some("agent-ok"), Some("rel-1"), Some("prop-1")));
    assert!(s.get("run_id").is_none() && s.get("reason").is_none());
    assert_eq!(r.audit()[0]["data"]["dropped_fields"].as_u64(), Some(5));
}

#[test]
fn admission_failure_is_a_503_and_leaves_no_trace_so_the_retry_can_admit() {
    let r = rig(None);
    r.adm.fail_next();
    let b = trig("explicit", "run.now", json!({}));
    assert_eq!(r.post(&b, Some(KEY)).status, 503);
    assert!(r.audit().is_empty());
    assert_eq!(r.post(&b, Some(KEY)).status, 202);
    assert_eq!((r.adm.len(), r.audit().len()), (1, 1));
}

#[test]
fn without_an_admitter_the_endpoint_says_so() {
    let auto = Automation::from_json(None, None, None).unwrap();
    let app = App::new(Arc::new(Store::memory()), Config { automation: Some(Arc::new(auto)), ..Config::default() });
    let csrf = body(&app.handle(&req("GET", "/api/v1/auth/session", &[], "")))["csrf_token"].as_str().unwrap().to_string();
    let r = app.handle(&req("POST", A, &[("x-csrf-token", &csrf), ("idempotency-key", KEY)], &trig("explicit", "run.now", json!({})).to_string()));
    assert_eq!(r.status, 503);
}

#[test]
fn the_trigger_shows_in_the_automation_projection() {
    let r = rig(None);
    r.post(&trig("outcome", "run.closed", json!({"run_id": "run-1", "outcome": "resolved"})), Some(KEY));
    let list = body(&r.app.handle(&req("GET", A, &[], "")));
    assert_eq!(list["count"], 1);
    let t = &list["triggers"][0];
    assert_eq!((t["trigger_key"].as_str(), t["kind"].as_str(), t["event_type"].as_str(), t["state"].as_str()), (Some(KEY), Some("outcome"), Some("run.closed"), Some("admitted")));
    assert_eq!(t["subject"]["run_id"], "run-1");
    let ct = body(&r.app.handle(&req("GET", "/internal/v1/automation/case-types", &[], "")));
    assert_eq!(ct["triggers"]["count"], 1);
    assert_eq!(ct["triggers"]["latest"][0]["trigger_key"], KEY);
}
