//! ANN1 LIVE (ignored): the real Announcer over real HTTP against the real support-platform backend started by
//! `scripts/ann1/serve_platform_live.py` (PR 17 code, own sqlite file, in-process agent registry double).
//! Env (process environment only, never printed): PULSO_PLATFORM_URL, PULSO_PLATFORM_SERVICE_TOKEN, PULSO_ANN1_PROPOSALS (path of the
//! launcher's proposals.json). Run: `cargo test -j 1 -p registry-writer --test live_ann1 -- --ignored --nocapture`.
mod common;
use common::{compiled_patch, finding};
use core_client::authorizer::Jws;
use registry_writer::announce::Announcer;
use registry_writer::transport::HttpTransport;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

fn dossier() -> Value {
    let f = finding();
    let mut signal = f.to_signal_json();
    signal["source"] = json!(f.source.as_str());
    let story: Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/../../../scripts/regression/results/story_template_estado_pqr.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
    let record = json!({"proposal": compiled_patch().to_json(), "doubles": [], "rubric": null});
    reasoning::dossier::build(&signal, &record, Some(&story), &reasoning::dossier::Labels { runtime: reasoning::dossier::Runtime::Real, ..Default::default() }).unwrap()
}

#[test]
#[ignore = "needs the live platform (scripts/ann1/serve_platform_live.py)"]
fn live_announce_replay_notifies_once_and_refusals_hold() {
    let env = |k: &str| std::env::var(k).ok();
    let a = Announcer::from_lookup(&env).expect("config").expect("PULSO_PLATFORM_URL and PULSO_PLATFORM_SERVICE_TOKEN must be set");
    let ids: Value = serde_json::from_str(&std::fs::read_to_string(env("PULSO_ANN1_PROPOSALS").unwrap()).unwrap()).unwrap();
    let (p0, p1, by_hand) = (ids["ids"][0].as_str().unwrap(), ids["ids"][1].as_str().unwrap(), ids["by_hand"].as_str().unwrap());
    let (f, d) = (finding(), dossier());

    let first = a.announce(&f, p0, &d);
    let again = a.announce(&f, p0, &d);
    println!("announce {p0}: {} / replay: {}", first.record(), again.record());
    assert!(first.announced() && again.announced());
    let second = a.announce(&f, p1, &d);
    println!("announce {p1}: {}", second.record());
    assert!(second.announced());

    let mut bad = d.clone();
    bad["es"]["sections"]["problem"] = json!("contacto ana.perez@example.com");
    let local = a.announce(&f, p0, &bad);
    println!("bad payload: {} (attempts on the wire: {})", local.record(), local.attempts);
    assert_eq!((local.record().as_str(), local.attempts), ("platform_announce_failed:invalid_payload", 0));

    let url = env("PULSO_PLATFORM_URL").unwrap();
    let addr = registry_writer::announce::parse_platform_url(&url).unwrap();
    let wrong = Announcer::new(Arc::new(HttpTransport::new(&addr, Duration::from_secs(15))), Jws::new("not-the-token".into()));
    println!("wrong token: {}", wrong.announce(&f, p0, &d).record());
    let empty = Announcer::new(Arc::new(HttpTransport::new(&addr, Duration::from_secs(15))), Jws::new(String::new()));
    let e = empty.announce(&f, p0, &d);
    println!("token missing: {} (attempts on the wire: {})", e.record(), e.attempts);
    println!("proposal made by hand (origin != auto_detect): {}", a.announce(&f, by_hand, &d).record());
    println!("unknown proposal: {}", a.announce(&f, "PRP-unknown", &d).record());

    let off = Announcer::from_lookup(&|k| if k == "PULSO_PLATFORM_URL" { Some(url.clone()) } else { None }).unwrap();
    println!("URL without token and no flag: announcer is {}", if off.is_none() { "OFF" } else { "ON" });
    let missing = Announcer::from_lookup(&|k| match k {
        "PULSO_ANNOUNCE_TO_PLATFORM" => Some("on".into()),
        "PULSO_PLATFORM_URL" => Some(url.clone()),
        _ => None,
    });
    println!("flag on, token missing: {}", missing.err().unwrap());
}
