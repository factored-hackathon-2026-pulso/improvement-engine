//! ANN1: the platform announcement payload (exact body, local bounds) and the Announcer (bounded retries, closed outcome records,
//! redaction, one-route allow-list, config gate).
mod common;
use common::{compiled_patch, finding};
use core_client::authorizer::Jws;
use registry_writer::announce::*;
use registry_writer::guard::platform_allowed;
use registry_writer::transport::{Reply, Request, Transport, TransportError};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TOKEN: &str = "throwaway-service-token-for-tests";

fn story() -> Value {
    let p = format!("{}/../../../scripts/regression/results/story_template_estado_pqr.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

fn dossier() -> Value {
    let f = finding();
    let mut signal = f.to_signal_json();
    signal["source"] = json!(f.source.as_str());
    let record = json!({"proposal": compiled_patch().to_json(), "doubles": [], "rubric": null});
    let labels = reasoning::dossier::Labels { runtime: reasoning::dossier::Runtime::Real, ..Default::default() };
    let d = reasoning::dossier::build(&signal, &record, Some(&story()), &labels).unwrap();
    assert_eq!(d["announce"], true, "fixture must be an announced dossier: {}", d["announce_reason"]);
    d
}

fn chars(v: &Value) -> usize {
    v.as_str().unwrap().chars().count()
}

#[test]
fn the_payload_is_the_exact_platform_body_within_every_bound() {
    let f = finding();
    let b = announce_payload(&f, "prp_1", &dossier()).unwrap();
    let keys: Vec<&str> = b.as_object().unwrap().keys().map(String::as_str).collect();
    let mut want = vec!["proposalId", "title", "problem", "evidence", "expectedEffect", "evidenceLinks"];
    want.sort_unstable();
    let mut got = keys.clone();
    got.sort_unstable();
    assert_eq!(got, want, "unknown fields are rejected by the route");
    assert_eq!(b["proposalId"], "prp_1");
    assert!((1..=120).contains(&chars(&b["title"])));
    assert!((1..=600).contains(&chars(&b["problem"])));
    assert!((1..=600).contains(&chars(&b["evidence"])));
    assert!((1..=400).contains(&chars(&b["expectedEffect"])));
    let links = b["evidenceLinks"].as_array().unwrap();
    assert!(!links.is_empty() && links.len() <= 8);
    assert!(links.iter().all(|l| valid_case_id(l.as_str().unwrap())));
    assert_eq!(b, announce_payload(&f, "prp_1", &dossier()).unwrap(), "deterministic");
    let all = b.to_string();
    assert!(!email_shaped(&all) && !long_number(&all));
}

#[test]
fn evidence_links_are_opaque_derived_from_the_evidence_ref_not_customer_ids() {
    let f = finding();
    let l = opaque_link(&f.evidence_ref(), 0);
    assert!(valid_case_id(&l));
    assert_ne!(l, opaque_link(&f.evidence_ref(), 1));
    assert!(!l.contains(&f.evidence_ref()));
}

#[test]
fn an_overlong_text_is_cut_to_the_cap_and_a_pii_shape_is_refused_locally() {
    let f = finding();
    let mut d = dossier();
    d["es"]["title"] = json!("t ".repeat(200));
    d["es"]["sections"]["problem"] = json!("palabra ".repeat(200));
    let b = announce_payload(&f, "prp_1", &d).unwrap();
    assert!(chars(&b["title"]) <= 120 && chars(&b["problem"]) <= 600);

    for bad in ["escriba a ana@example.com ya", "llame 1234 5678 90 hoy", "doc 123-456-789"] {
        let mut d = dossier();
        d["es"]["sections"]["evidence"] = json!(bad);
        let e = announce_payload(&f, "prp_1", &d).unwrap_err();
        assert_eq!(e.field, "evidence", "{bad}");
    }
    // an artifact id and short counts are fine
    let mut d = dossier();
    d["es"]["sections"]["evidence"] = json!("recepcion@1.0.0 con 117.021 casos, 56,1 %");
    assert!(announce_payload(&f, "prp_1", &d).is_ok());
}

#[test]
fn bad_ids_empty_text_and_non_announced_dossiers_are_refused() {
    let f = finding();
    let d = dossier();
    assert!(announce_payload(&f, "", &d).is_err());
    assert!(announce_payload(&f, &"p".repeat(65), &d).is_err());
    assert!(announce_payload(&f, "a b", &d).is_err());
    let mut e = d.clone();
    e["es"]["sections"]["expected_effect"] = json!("   ");
    assert_eq!(announce_payload(&f, "p", &e).unwrap_err().field, "expectedEffect");
    let mut n = d.clone();
    n["announce"] = json!(false);
    assert_eq!(announce_payload(&f, "p", &n).unwrap_err().field, "dossier");
}

#[test]
fn the_pii_rules_mirror_the_platform_regexes() {
    assert!(email_shaped("x@y.co") && email_shaped("a b@c.de.fg") && !email_shaped("recepcion@1.0.0") && !email_shaped("@y.com") && !email_shaped("a@ b.com"));
    assert!(long_number("123456789") && long_number("1 2 3 4 5 6 7 8 9") && long_number("12.345.678-9") && !long_number("12345678") && !long_number("1  2 3 4 5 6 7 8 9"));
}

type Plan = Vec<Result<Reply, TransportError>>;
struct Scripted {
    plan: Mutex<Plan>,
    seen: Mutex<Vec<(String, String, Option<String>, Value, String)>>,
}
impl Scripted {
    fn new(mut plan: Plan) -> Arc<Scripted> {
        plan.reverse();
        Arc::new(Scripted { plan: Mutex::new(plan), seen: Mutex::new(vec![]) })
    }
}
impl Transport for Scripted {
    fn send(&self, r: &Request) -> Result<Reply, TransportError> {
        self.seen.lock().unwrap().push((r.method.into(), r.path.clone(), r.idempotency_key.map(str::to_string), r.body.clone().unwrap(), r.bearer.reveal().into()));
        self.plan.lock().unwrap().pop().expect("unexpected extra request")
    }
}
fn reply(s: u16) -> Result<Reply, TransportError> {
    Ok(Reply { status: s, body: json!({"proposalId": "prp_1"}) })
}
fn announcer(t: Arc<Scripted>, token: &str) -> Announcer {
    let mut a = Announcer::new(t, Jws::new(token.into()));
    a.sleep = |_| {};
    a.backoff = Duration::ZERO;
    a
}

#[test]
fn a_201_or_200_is_announced_to_the_one_route_with_the_service_token_and_an_idempotency_key() {
    let t = Scripted::new(vec![reply(201), reply(200)]);
    let a = announcer(t.clone(), TOKEN);
    let f = finding();
    let (o1, o2) = (a.announce(&f, "prp_1", &dossier()), a.announce(&f, "prp_1", &dossier()));
    assert_eq!((o1.record(), o2.record()), ("platform_announced".to_string(), "platform_announced".to_string()));
    let seen = t.seen.lock().unwrap();
    assert!(seen.iter().all(|s| s.0 == "POST" && s.1 == ROUTE && s.2.as_deref() == Some("announce:prp_1") && s.4 == TOKEN));
    assert_eq!(seen[0].3, seen[1].3, "the replay sends the same body");
}

#[test]
fn retries_are_bounded_and_only_for_transient_failures() {
    let t = Scripted::new(vec![reply(503), Err(TransportError::NotSent("x".into())), reply(201)]);
    let o = announcer(t.clone(), TOKEN).announce(&finding(), "prp_1", &dossier());
    assert_eq!((o.record(), o.attempts), ("platform_announced".to_string(), 3));

    let t = Scripted::new(vec![reply(502), reply(502), reply(502)]);
    let o = announcer(t.clone(), TOKEN).announce(&finding(), "prp_1", &dossier());
    assert_eq!((o.record(), o.attempts), ("platform_announce_failed:unavailable".to_string(), 3));
    assert_eq!(t.seen.lock().unwrap().len(), 3);

    for (code, class) in [(401, "unauthorized"), (404, "not_found"), (422, "rejected"), (409, "unexpected_status")] {
        let t = Scripted::new(vec![reply(code)]);
        let o = announcer(t.clone(), TOKEN).announce(&finding(), "prp_1", &dossier());
        assert_eq!(o.record(), format!("platform_announce_failed:{class}"));
        assert_eq!(t.seen.lock().unwrap().len(), 1, "{code} is not retried");
    }
}

#[test]
fn a_bad_payload_or_a_missing_token_never_reaches_the_wire() {
    let t = Scripted::new(vec![]);
    let mut d = dossier();
    d["es"]["sections"]["problem"] = json!("contacto ana@example.com");
    assert_eq!(announcer(t.clone(), TOKEN).announce(&finding(), "prp_1", &d).record(), "platform_announce_failed:invalid_payload");
    assert_eq!(announcer(t.clone(), "").announce(&finding(), "prp_1", &dossier()).record(), "platform_announce_failed:token_missing");
    assert!(t.seen.lock().unwrap().is_empty());
}

#[test]
fn the_token_is_never_printed_and_a_mismatched_response_is_flagged() {
    let a = announcer(Scripted::new(vec![]), TOKEN);
    assert!(!format!("{a:?}").contains(TOKEN));
    let t = Scripted::new(vec![Ok(Reply { status: 200, body: json!({"proposalId": "other"}) })]);
    assert_eq!(announcer(t, TOKEN).announce(&finding(), "prp_1", &dossier()).record(), "platform_announce_failed:response_mismatch");
}

#[test]
fn the_platform_allow_list_is_one_route() {
    assert!(platform_allowed("POST", ROUTE));
    for (m, p) in [("GET", ROUTE), ("POST", "/api/v1/internal/builder/proposals/announce/x"), ("POST", "/api/v1/builder/proposals/prp_1/approve"), ("POST", "/api/v1/internal/grants"), ("DELETE", ROUTE)] {
        assert!(!platform_allowed(m, p), "{m} {p}");
    }
}

#[test]
fn the_url_is_loopback_or_private_only_and_the_flag_gates_it() {
    for ok in ["http://127.0.0.1:8000", "http://localhost:8000/", "http://10.1.2.3:80", "http://192.168.0.5", "http://[::1]:9000"] {
        assert!(parse_platform_url(ok).is_ok(), "{ok}");
    }
    assert_eq!(parse_platform_url("http://127.0.0.1:8000").unwrap(), "127.0.0.1:8000");
    for bad in ["https://127.0.0.1:8000", "http://example.com:80", "http://8.8.8.8:80", "http://127.0.0.1:8000/x", "http://u@127.0.0.1:1", "http://127.0.0.1:99999"] {
        assert!(parse_platform_url(bad).is_err(), "{bad}");
    }
    let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
    assert!(Announcer::from_lookup(&env(&[])).unwrap().is_none(), "default OFF");
    assert!(Announcer::from_lookup(&env(&[("PULSO_PLATFORM_URL", "http://127.0.0.1:8000")])).unwrap().is_none(), "URL alone stays off");
    let both: &[(&str, &str)] = &[("PULSO_PLATFORM_URL", "http://127.0.0.1:8000"), ("PULSO_PLATFORM_SERVICE_TOKEN", "t")];
    assert!(Announcer::from_lookup(&env(both)).unwrap().is_some(), "ON in the live path when URL and token are set");
    let off: &[(&str, &str)] = &[("PULSO_ANNOUNCE_TO_PLATFORM", "off"), ("PULSO_PLATFORM_URL", "http://127.0.0.1:8000"), ("PULSO_PLATFORM_SERVICE_TOKEN", "t")];
    assert!(Announcer::from_lookup(&env(off)).unwrap().is_none());
    let on_no_token: &[(&str, &str)] = &[("PULSO_ANNOUNCE_TO_PLATFORM", "on"), ("PULSO_PLATFORM_URL", "http://127.0.0.1:8000")];
    let e = Announcer::from_lookup(&env(on_no_token)).err().unwrap();
    assert!(e.contains("PULSO_PLATFORM_SERVICE_TOKEN") && !e.contains("127.0.0.1"));
}
