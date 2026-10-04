//! Q1: the successor run through the control-api release correlation, and the post-run memory note.
use serde_json::Value;
use thread10::platform::{Platform, Release};
use thread10::{Opts, run};

fn rel(id: &str) -> Release {
    Release { release_id: id.into(), agent_id: "atencion-tarea".into(), alias: "staging".into(), candidate_hash: "ab".repeat(32) }
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("t10s-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn a_published_event_matching_the_recorded_release_creates_one_successor() {
    let p = Platform::new();
    p.record_release(&rel("rel-1")).unwrap();
    let (st, body) = p.post_published("e1", &rel("rel-1"));
    assert_eq!((st, body["state"].as_str(), body["observation_window"].as_str()), (202, Some("correlated"), Some("simulated")), "{body}");
    assert_eq!(p.successor_keys(), ["successor:rel-1"]);
}

#[test]
fn a_replayed_event_creates_one_successor() {
    let p = Platform::new();
    p.record_release(&rel("rel-1")).unwrap();
    assert_eq!(p.post_published("e1", &rel("rel-1")).0, 202);
    assert_eq!(p.post_published("e1", &rel("rel-1")).0, 200, "same event id: replay");
    assert_eq!(p.post_published("e2", &rel("rel-1")).0, 202, "same release, another delivery");
    assert_eq!(p.successor_keys(), ["successor:rel-1"], "exactly one run");
}

#[test]
fn an_unmatched_release_is_retryable_and_schedules_nothing() {
    let p = Platform::new();
    let (st, body) = p.post_published("u1", &rel("rel-unknown"));
    assert_eq!(st, 503, "{body}");
    assert!(p.successor_keys().is_empty());
    p.record_release(&rel("rel-unknown")).unwrap(); // the engine records it later; the retried event now matches
    assert_eq!(p.post_published("u1", &rel("rel-unknown")).0, 202);
    assert_eq!(p.successor_keys(), ["successor:rel-unknown"]);
}

fn opts(name: &str) -> Opts {
    Opts { human_override: true, ..Opts::new(tmp(name), env!("CARGO_BIN_EXE_synth_runner").into()) }
}

#[test]
fn the_thread_publishes_then_correlates_one_successor_and_says_the_window_is_simulated() {
    let r = run(&opts("thread")).expect("run");
    assert_eq!(r.error, None);
    let s = &r.report["successor"];
    let rid = r.report["steps"].as_array().unwrap().iter().find(|x| x["id"] == "publish").unwrap()["detail"]["publish"]["release_id"].as_str().unwrap().to_string();
    assert_eq!(s["unique_key"], format!("successor:{rid}"));
    assert_eq!(s["runs"], 1);
    assert_eq!(s["replay_status"], 200);
    assert_eq!(s["unmatched_status"], 503);
    assert_eq!(s["observation_window"], "simulated");
}

#[test]
fn no_publish_means_no_successor() {
    let mut o = opts("nopub");
    o.human_override = false;
    let r = run(&o).expect("run");
    assert_eq!(r.report["successor"], Value::Null);
}

#[test]
fn the_post_run_note_is_a_thin_memory_note_with_resolving_evidence() {
    let r = run(&opts("note")).expect("run");
    let n = &r.report["memory_note"];
    assert_eq!((n["scope"].as_str(), n["durable"].as_bool(), n["status"].as_str()), (Some("demo1_thin"), Some(false), Some("active")), "{n}");
    let st = n["statement"].as_str().unwrap();
    assert!(st.contains("gate=fail") && st.contains("host=rust") && st.contains("quality_claims=forbidden"), "{st}");
    let refs = n["evidence_refs"].as_array().unwrap();
    assert!(refs.len() >= 9, "one evidence ref per committed event: {refs:?}");
    assert!(n["evidence_digests"].as_object().unwrap().values().all(|d| d.as_str().is_some_and(|d| d.len() == 64)), "{n}");
}
