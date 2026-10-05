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
        self.seen.lock().unwrap().push((r.method.into(), r.path.clone(), r.idempotency_key.map(str::to_string), r.body.clone().unwrap_or(Value::Null), r.bearer.reveal().into()));
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

// ---------------------------------------------------------------------------------------------------------------------------------
// SIG1: evidence by route.

fn cell(dims: &[(&str, &str)]) -> reasoning::finding::Finding {
    let mut f = finding();
    f.dims = dims.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
    f
}

const REAL: [&str; 2] = ["CASE-01HZX0000000000000000000A0", "CASE-01HZX0000000000000000000B0"];

fn ids_reply(ids: &[&str]) -> Result<Reply, TransportError> {
    Ok(Reply { status: 200, body: json!({"suppressed": false, "matched": 37, "caseIds": ids}) })
}

#[test]
fn the_cell_maps_to_the_platform_query_only_when_every_dimension_maps() {
    let q = evidence_query(&cell(&[("channel", "web_chat"), ("language", "es"), ("case_type", "undue_charge")])).unwrap();
    assert_eq!(q, "/api/v1/internal/evidence/cases?caseType=undue_charge&channel=chat_web&language=es&limit=8");
    assert_eq!(evidence_query(&cell(&[("channel", "mobile_app"), ("priority", "high")])).unwrap(), "/api/v1/internal/evidence/cases?channel=chat_app&priority=high&limit=8");
    // a dimension of another vocabulary, an ambiguous channel or an empty cell: no partial query (it would return cases OUTSIDE the cell)
    for dims in [&[("reason_category", "Tecnico"), ("channel", "web_chat")][..], &[("channel", "phone")], &[("channel", "whatsapp")], &[("language", "en")], &[("case_type", "free text")], &[]] {
        assert!(evidence_query(&cell(dims)).is_err(), "{dims:?}");
    }
    assert!(platform_allowed("GET", &q), "the query the engine builds is on the allow-list");
}

#[test]
fn the_guard_allows_only_the_evidence_read() {
    let ok = "/api/v1/internal/evidence/cases?channel=chat_web&limit=8";
    assert!(platform_allowed("GET", ok) && platform_allowed("GET", "/api/v1/internal/evidence/cases"));
    for bad in [
        "/api/v1/internal/evidence/cases?channel=chat web",
        "/api/v1/internal/evidence/cases?channel=a%20b",
        "/api/v1/internal/evidence/cases?channel=x#f",
        "/api/v1/internal/evidence/cases/x?channel=a",
        "/api/v1/internal/evidence/cases?",
        "/api/v1/internal/evidence/cases?channel=",
        "/api/v1/internal/evidence/cases?=a",
        "/api/v1/internal/evidence/casesx",
        "/api/v1/internal/builder/proposals",
        "/api/v1/cases",
    ] {
        assert!(!platform_allowed("GET", bad), "{bad}");
    }
    assert!(!platform_allowed("POST", ok) && !platform_allowed("DELETE", ok));
}

#[test]
fn real_links_replace_the_opaque_one_and_an_opaque_fallback_is_labelled() {
    let f = cell(&[("channel", "web_chat")]);
    let real = announce_payload_with(&f, "prp_1", &dossier(), &Links::Real(REAL.iter().map(|s| (*s).to_string()).collect())).unwrap();
    assert_eq!(real["evidenceLinks"], json!(REAL));
    let legacy = announce_payload(&f, "prp_1", &dossier()).unwrap();
    assert_eq!(real["evidence"], legacy["evidence"], "real links add no label");

    let opaque = announce_payload_with(&f, "prp_1", &dossier(), &Links::Opaque("route_unavailable")).unwrap();
    assert_eq!(opaque["evidenceLinks"], legacy["evidenceLinks"], "the opaque link is kept");
    let ev = opaque["evidence"].as_str().unwrap();
    assert!(ev.ends_with(OPAQUE_LABEL) && chars(&opaque["evidence"]) <= 600 && !email_shaped(ev) && !long_number(ev));
    let mut long = dossier();
    long["es"]["sections"]["evidence"] = json!("palabra ".repeat(200));
    assert!(chars(&announce_payload_with(&f, "prp_1", &long, &Links::Opaque("x")).unwrap()["evidence"]) <= 600, "the label fits in the cap");
}

#[test]
fn resolve_links_degrades_honestly_with_a_closed_reason() {
    let f = cell(&[("channel", "web_chat"), ("language", "pt")]);
    let tok = Jws::new(TOKEN.into());
    let run = |plan: Plan, f: &reasoning::finding::Finding| {
        let t = Scripted::new(plan);
        let l = resolve_links(t.as_ref(), &tok, f);
        let seen = t.seen.lock().unwrap().clone();
        (l, seen)
    };
    let (l, seen) = run(vec![ids_reply(&REAL)], &f);
    assert_eq!(l, Links::Real(REAL.iter().map(|s| (*s).to_string()).collect()));
    assert_eq!((seen[0].0.as_str(), seen[0].1.as_str(), seen[0].2.as_deref(), seen[0].4.as_str()), ("GET", "/api/v1/internal/evidence/cases?channel=chat_web&language=pt&limit=8", None, TOKEN));

    for (plan, why) in [
        (vec![Ok(Reply { status: 200, body: json!({"suppressed": true, "matched": null, "caseIds": []}) })], "suppressed_below_k"),
        (vec![Ok(Reply { status: 404, body: Value::Null })], "route_unavailable"),
        (vec![Ok(Reply { status: 401, body: Value::Null })], "unauthorized"),
        (vec![Ok(Reply { status: 422, body: Value::Null })], "rejected"),
        (vec![Ok(Reply { status: 503, body: Value::Null })], "unavailable"),
        (vec![Err(TransportError::NotSent("down".into()))], "unavailable"),
        (vec![ids_reply(&["CASE-not-a-real-shape"])], "bad_response"),
        (vec![ids_reply(&[])], "bad_response"),
        (vec![Ok(Reply { status: 200, body: Value::Null })], "bad_response"),
        (vec![ids_reply(&[REAL[0]; 9])], "bad_response"),
    ] {
        assert_eq!(run(plan, &f).0, Links::Opaque(why), "{why}");
    }
    // nothing is sent for a cell the platform cannot express, or without a token
    let (l, seen) = run(vec![], &finding());
    assert_eq!((l, seen.len()), (Links::Opaque("unmappable_dimension"), 0));
    let t = Scripted::new(vec![]);
    assert_eq!(resolve_links(t.as_ref(), &Jws::new(String::new()), &f), Links::Opaque("token_missing"));
}

#[test]
fn the_announcer_resolves_then_announces_with_real_ids_and_records_it() {
    let f = cell(&[("channel", "web_chat")]);
    let t = Scripted::new(vec![ids_reply(&REAL), reply(201)]);
    let mut a = announcer(t.clone(), TOKEN);
    a.resolve_evidence = true;
    let o = a.announce(&f, "prp_1", &dossier());
    assert_eq!((o.record(), o.evidence.as_deref()), ("platform_announced".to_string(), Some("real")));
    let seen = t.seen.lock().unwrap();
    assert_eq!((seen[0].0.as_str(), seen[1].0.as_str(), seen[1].1.as_str()), ("GET", "POST", ROUTE));
    assert_eq!(seen[1].3["evidenceLinks"], json!(REAL));
    assert!(seen.iter().all(|s| s.4 == TOKEN));
}

#[test]
fn an_unavailable_route_keeps_the_opaque_link_labels_it_and_still_announces() {
    let f = cell(&[("channel", "web_chat")]);
    let t = Scripted::new(vec![Ok(Reply { status: 404, body: Value::Null }), reply(201)]);
    let mut a = announcer(t.clone(), TOKEN);
    a.resolve_evidence = true;
    let o = a.announce(&f, "prp_1", &dossier());
    assert_eq!((o.record(), o.evidence.as_deref()), ("platform_announced".to_string(), Some("opaque:route_unavailable")));
    let body = t.seen.lock().unwrap()[1].3.clone();
    assert_eq!(body["evidenceLinks"], announce_payload(&f, "prp_1", &dossier()).unwrap()["evidenceLinks"]);
    assert!(body["evidence"].as_str().unwrap().ends_with(OPAQUE_LABEL));

    // a cell the platform cannot express: no GET at all, one POST, labelled
    let t = Scripted::new(vec![reply(201)]);
    let mut a = announcer(t.clone(), TOKEN);
    a.resolve_evidence = true;
    let o = a.announce(&finding(), "prp_1", &dossier());
    assert_eq!(o.evidence.as_deref(), Some("opaque:unmappable_dimension"));
    assert_eq!(t.seen.lock().unwrap().len(), 1);
}

#[test]
fn resolution_is_off_by_default_and_on_from_the_environment_with_either_token_name() {
    let t = Scripted::new(vec![reply(201)]);
    let o = announcer(t.clone(), TOKEN).announce(&cell(&[("channel", "web_chat")]), "prp_1", &dossier());
    assert_eq!((o.record(), o.evidence), ("platform_announced".to_string(), None), "legacy path: no GET, no label");
    assert_eq!(t.seen.lock().unwrap().len(), 1);

    let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
    let primary: &'static [(&'static str, &'static str)] = &[("PULSO_PLATFORM_URL", "http://127.0.0.1:8000"), ("PULSO_PLATFORM_SERVICE_TOKEN", "t")];
    let platform_name: &'static [(&'static str, &'static str)] = &[("PULSO_PLATFORM_URL", "http://127.0.0.1:8000"), ("CC_INTERNAL_SERVICE_TOKEN", "t")];
    for pairs in [primary, platform_name] {
        let a = Announcer::from_lookup(&env(pairs)).unwrap().expect("the token must switch the announcer on");
        assert!(a.resolve_evidence);
    }
}

/// A real socket: the fake platform answers the evidence read and the announce over HTTP, so the query string and the bearer reach
/// it exactly as the real platform would see them. Returns the request lines (and the announce body).
fn fake_platform(evidence: String) -> (String, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = std::thread::spawn(move || {
        let mut lines = vec![];
        for _ in 0..2 {
            let (mut c, _) = l.accept().unwrap();
            let mut buf = vec![0u8; 65536];
            let mut n = 0;
            loop {
                let k = c.read(&mut buf[n..]).unwrap();
                n += k;
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(h) = text.find("\r\n\r\n") {
                    let len = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                    if n >= h + 4 + len {
                        break;
                    }
                }
                assert!(k > 0, "eof before the request was complete");
            }
            let text = String::from_utf8_lossy(&buf[..n]).to_string();
            let first = text.lines().next().unwrap().to_string();
            let auth = text.lines().find(|l| l.to_ascii_lowercase().starts_with("authorization:")).unwrap_or("").to_string();
            lines.push(format!("{first} | {}", if auth.contains(TOKEN) { "bearer-ok" } else { "bearer-wrong" }));
            let (status, body) = if first.starts_with("GET") {
                (if evidence.is_empty() { 404 } else { 200 }, evidence.clone())
            } else {
                lines.push(text.split("\r\n\r\n").nth(1).unwrap_or("").to_string());
                (201, json!({"proposalId": "prp_1"}).to_string())
            };
            write!(c, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        lines
    });
    (addr, h)
}

#[test]
fn over_a_real_socket_the_fake_platform_sees_the_cell_query_and_the_real_ids_arrive_in_the_announce() {
    let (addr, h) = fake_platform(json!({"suppressed": false, "matched": 12, "caseIds": REAL}).to_string());
    let mut a = Announcer::new(Arc::new(registry_writer::transport::HttpTransport::new(&addr, Duration::from_secs(5))), Jws::new(TOKEN.into()));
    a.resolve_evidence = true;
    let o = a.announce(&cell(&[("channel", "web_chat"), ("language", "es")]), "prp_1", &dossier());
    assert_eq!((o.record(), o.evidence.as_deref()), ("platform_announced".to_string(), Some("real")));
    let seen = h.join().unwrap();
    assert_eq!(seen[0], "GET /api/v1/internal/evidence/cases?channel=chat_web&language=es&limit=8 HTTP/1.1 | bearer-ok");
    assert!(seen[1].starts_with("POST /api/v1/internal/builder/proposals/announce HTTP/1.1 | bearer-ok"), "{}", seen[1]);
    let body: Value = serde_json::from_str(&seen[2]).unwrap();
    assert_eq!(body["evidenceLinks"], json!(REAL));
}

#[test]
fn over_a_real_socket_a_404_route_degrades_to_the_labelled_opaque_link() {
    let (addr, h) = fake_platform(String::new());
    let mut a = Announcer::new(Arc::new(registry_writer::transport::HttpTransport::new(&addr, Duration::from_secs(5))), Jws::new(TOKEN.into()));
    a.resolve_evidence = true;
    let o = a.announce(&cell(&[("channel", "web_chat")]), "prp_1", &dossier());
    assert_eq!((o.record(), o.evidence.as_deref()), ("platform_announced".to_string(), Some("opaque:route_unavailable")));
    let seen = h.join().unwrap();
    let body: Value = serde_json::from_str(&seen[2]).unwrap();
    assert!(body["evidence"].as_str().unwrap().ends_with(OPAQUE_LABEL));
}
