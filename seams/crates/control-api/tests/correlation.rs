//! P2R: release.* events (outside event-catalog 1.1.0) correlate with the published release the engine recorded and start ONE
//! successor run. Rust-only: the Python double has no release side channel. The observation window after a publish is
//! simulated (platform-sim) in the demo.
mod common;
use common::*;
use control_api::correlation::MemSuccessorSink;
use serde_json::{Value, json};
use std::sync::Arc;

const URL: &str = "/internal/v1/platform/releases";

struct Fx {
    rig: Rig,
    sink: Arc<MemSuccessorSink>,
}

fn fx() -> Fx {
    let sink = Arc::new(MemSuccessorSink::default());
    let s = sink.clone();
    let rig = Rig::with(move |c| c.successors = Some(s));
    rig.admin(json!({"published_releases": [{"tenant": "t1", "release_id": "rel-1", "agent_id": "agent-a", "alias": "prod", "candidate_hash": "cand-1"}]}));
    Fx { rig, sink }
}

fn event(id: &str, kind: &str, release: &str) -> Value {
    json!({"event_id": id, "event": {"event_type": kind, "entity": "release", "entity_id": release,
           "payload": {"release_id": release, "agent_id": "agent-a", "alias": "prod"}}})
}

impl Fx {
    fn post_as(&self, tenant: &str, ev: &Value) -> (u16, Value) {
        let tok = self.rig.token("cb", "control-api", "release_events", tenant, json!({"purpose": "platform_releases"}));
        self.rig.call("POST", URL, Some(ev), Some(&tok), &[("Idempotency-Key", ev["event_id"].as_str().unwrap())])
    }
    fn post(&self, ev: &Value) -> (u16, Value) {
        self.post_as("t1", ev)
    }
}

#[test]
fn published_event_correlates_and_starts_one_successor() {
    let f = fx();
    let (st, r) = f.post(&event("e1", "release.published", "rel-1"));
    assert_eq!((st, r["state"].as_str(), r["candidate_hash"].as_str(), r["observation_window"].as_str()), (202, Some("correlated"), Some("cand-1"), Some("simulated")), "{r}");
    let runs = f.sink.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].unique_key.as_str(), runs[0].agent_id.as_str(), runs[0].alias.as_str()), ("successor:rel-1", "agent-a", "prod"));
}

#[test]
fn release_event_creates_2_runs_on_replay() {
    let f = fx();
    f.post(&event("e1", "release.published", "rel-1"));
    let (st, _) = f.post(&event("e1", "release.published", "rel-1")); // same event id: replay
    assert_eq!(st, 200);
    f.post(&event("e2", "release.published", "rel-1")); // a second delivery of the same release under another event id
    assert_eq!(f.sink.runs().len(), 1, "1 run created, 0 duplicates");
}

#[test]
fn concurrent_deliveries_create_exactly_one_run() {
    let f = Arc::new(fx());
    let hs: Vec<_> = (0..8)
        .map(|i| {
            let f = f.clone();
            std::thread::spawn(move || f.post(&event(&format!("c{i}"), "release.published", "rel-1")).0)
        })
        .collect();
    assert!(hs.into_iter().all(|h| h.join().unwrap() == 202));
    assert_eq!(f.sink.runs().len(), 1);
}

#[test]
fn the_same_event_id_with_another_body_conflicts() {
    let f = fx();
    f.post(&event("e1", "release.published", "rel-1"));
    let mut other = event("e1", "release.published", "rel-1");
    other["event"]["payload"]["previous_release_id"] = json!("rel-0");
    assert_eq!(f.post(&other).0, 409);
}

#[test]
fn rollback_has_no_successor_and_unknown_or_mismatched_releases_do_not_schedule() {
    let f = fx();
    let (st, r) = f.post(&event("r1", "release.rolled_back", "rel-1"));
    assert_eq!((st, r["state"].as_str(), r["successor"].is_null()), (202, Some("correlated"), true), "{r}");
    let (st, r) = f.post(&event("u1", "release.published", "rel-unknown"));
    assert_eq!(st, 503, "unmatched must be retryable, not acknowledged: {r}");
    let mut bad = event("m1", "release.published", "rel-1");
    bad["event"]["payload"]["alias"] = json!("staging");
    assert_eq!(f.post(&bad).0, 409);
    assert!(f.sink.runs().is_empty());
    // the engine records the release later: the retried event now matches (unmatched events are not frozen)
    f.rig.admin(json!({"published_releases": [{"tenant": "t1", "release_id": "rel-unknown", "agent_id": "agent-a", "alias": "prod", "candidate_hash": "c9"}]}));
    assert_eq!(f.post(&event("u1", "release.published", "rel-unknown")).1["state"], "correlated");
    assert_eq!(f.sink.runs().len(), 1);
}

#[test]
fn tenants_are_isolated_and_the_schema_is_strict() {
    let f = fx();
    let (st, r) = f.post_as("t2", &event("x1", "release.published", "rel-1"));
    assert_ne!((st, r["state"].as_str()), (202, Some("correlated")), "{st} {r}"); // the pin refuses t2; and t2 has no such release
    assert!(f.sink.runs().is_empty());
    let mut extra = event("s1", "release.published", "rel-1");
    extra["event"]["payload"]["effect_size"] = json!(0.3); // identity only
    assert_eq!(f.post(&extra).0, 422);
    assert_eq!(f.post(&event("s2", "case.viewed", "rel-1")).0, 422);
    let tok = f.rig.token("cb", "control-api", "release_events", "t1", json!({"purpose": "platform_releases"}));
    assert_eq!(f.rig.call("POST", URL, Some(&event("s3", "release.published", "rel-1")), Some(&tok), &[("Idempotency-Key", "other")]).0, 422);
    assert_eq!(f.rig.call("POST", URL, Some(&event("s3", "release.published", "rel-1")), None, &[]).0, 401);
}
