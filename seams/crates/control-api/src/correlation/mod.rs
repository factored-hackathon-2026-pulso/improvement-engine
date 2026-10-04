//! P2R: release correlation and the successor trigger.
//!
//! `release.*` events are NOT in event-catalog 1.1.0 (the ingest path quarantines them as unknown) and that catalog stays
//! untouched, so they arrive on a Claude-owned side channel, `POST /internal/v1/platform/releases`, carrying the identity-only
//! projection of `platform-contract/release-contract/release_event.schema.json` wrapped as `{"event_id", "event"}`.
//! A `release.published` event is matched with the release the engine recorded (`record_published`: release id, agent id,
//! alias, candidate hash); the correlation schedules exactly ONE successor run through a `SuccessorSink` keyed by release id.
//! In the DEMO the observation window that follows is simulated (platform-sim): the schedule says so (`observation_window`).
use crate::app::{App, Req, Resp, code, error, json_resp};
use crate::store::Store;
use core_client::canon::{jcs, sha256_hex};
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};

pub const PUBLISHED_NS: &str = "engine_release";
pub const CORRELATION_NS: &str = "release_correlation";
pub const EVENT_NS: &str = "release_event";
pub const SUCCESSOR_NS: &str = "successor_run";

/// What the engine recorded when it published a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedRelease {
    pub release_id: String,
    pub agent_id: String,
    pub alias: String,
    pub candidate_hash: String,
}

/// One successor run to schedule. `unique_key` is `successor:<release id>`: the sink creates at most one run per key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuccessorRun {
    pub tenant: String,
    pub unique_key: String,
    pub release_id: String,
    pub agent_id: String,
    pub alias: String,
    pub candidate_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheduled {
    Created,
    Duplicate,
}

/// Port to the engine job store: it implements this to turn a correlation into a job. `schedule` is atomic and
/// single-winner per `(tenant, unique_key)`: replays and concurrent deliveries get `Duplicate` and create nothing.
pub trait SuccessorSink: Send + Sync {
    fn schedule(&self, run: &SuccessorRun) -> Scheduled;
}

/// In-memory sink for tests.
#[derive(Default)]
pub struct MemSuccessorSink(Mutex<Vec<SuccessorRun>>);

impl MemSuccessorSink {
    pub fn runs(&self) -> Vec<SuccessorRun> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

impl SuccessorSink for MemSuccessorSink {
    fn schedule(&self, run: &SuccessorRun) -> Scheduled {
        let mut g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if g.iter().any(|r| r.tenant == run.tenant && r.unique_key == run.unique_key) {
            return Scheduled::Duplicate;
        }
        g.push(run.clone());
        Scheduled::Created
    }
}

/// Default sink: a durable queue of `successor_run` documents in the `Store` (MemStore or PgStore) that the engine drains.
pub struct StoreSuccessorSink(pub Arc<dyn Store>);

impl SuccessorSink for StoreSuccessorSink {
    fn schedule(&self, run: &SuccessorRun) -> Scheduled {
        let doc = json!({"release_id": run.release_id, "agent_id": run.agent_id, "alias": run.alias, "candidate_hash": run.candidate_hash, "window": "simulated"});
        if self.0.put_doc_new(SUCCESSOR_NS, &run.tenant, &run.unique_key, doc) { Scheduled::Created } else { Scheduled::Duplicate }
    }
}

/// Records a release the engine published (idempotent: the first record wins).
pub fn record_published(store: &dyn Store, tenant: &str, r: &PublishedRelease) -> bool {
    store.put_doc_new(PUBLISHED_NS, tenant, &r.release_id, json!({"agent_id": r.agent_id, "alias": r.alias, "candidate_hash": r.candidate_hash}))
}

fn non_empty(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.is_empty() && s.chars().count() <= 256 && !s.contains('\0'))
}

/// Strict model of `release_event.schema.json`: `(entity_id, kind, payload)`.
fn validate_event(e: &Value) -> Option<(String, &'static str, Map<String, Value>)> {
    let m = e.as_object()?;
    if m.len() != 4 || !["event_type", "entity", "entity_id", "payload"].iter().all(|k| m.contains_key(*k)) || m["entity"] != "release" {
        return None;
    }
    let kind = match m["event_type"].as_str()? {
        "release.published" => "published",
        "release.rolled_back" => "rolled_back",
        _ => return None,
    };
    let entity_id = non_empty(&m["entity_id"])?.to_string();
    let p = m["payload"].as_object()?;
    let ok = ["release_id", "agent_id", "alias"].iter().all(|k| p.get(*k).and_then(non_empty).is_some())
        && p.keys().all(|k| ["release_id", "agent_id", "alias", "previous_release_id"].contains(&k.as_str()))
        && p.get("previous_release_id").is_none_or(|v| non_empty(v).is_some());
    ok.then(|| (entity_id, kind, p.clone()))
}

impl App {
    // ---- POST /internal/v1/platform/releases ---------------------------------------------------------------------
    pub(crate) fn release_event(&self, r: &Req) -> Resp {
        let claims = match self.ingest_claims(r, "release_events", Some("platform_releases")) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default().to_string();
        let Some(body) = serde_json::from_slice::<Value>(&r.body).ok().filter(|b| b.as_object().is_some_and(|o| o.len() == 2)) else { return error(422, "schema_invalid") };
        let (Some(event_id), Some((entity_id, kind, p))) = (non_empty(&body["event_id"]), validate_event(&body["event"])) else { return error(422, "schema_invalid") };
        if r.headers.get("idempotency-key").map(String::as_str) != Some(event_id) {
            return error(422, "idempotency_key_mismatch");
        }
        if p["release_id"] != entity_id.as_str() {
            return error(422, "entity_mismatch");
        }
        let Ok(canon) = jcs(&body) else { return error(422, "schema_invalid") };
        let digest = sha256_hex(canon.as_bytes());
        let release_id = entity_id;

        let _w = self.write_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(prior) = self.store.get_doc(EVENT_NS, &tenant, event_id) {
            return if prior["digest"] == digest { json_resp(200, prior["response"].clone()) } else { error(409, "event_conflict") };
        }
        let Some(rec) = self.store.get_doc(PUBLISHED_NS, &tenant, &release_id) else {
            return error(503, "release_unmatched"); // nothing persisted; non-2xx so the sender retries once the engine has recorded the release
        };
        if rec["agent_id"] != p["agent_id"] || rec["alias"] != p["alias"] {
            return code(409, "release_mismatch");
        }
        let mut response = json!({"state": "correlated", "kind": kind, "release_id": release_id, "agent_id": rec["agent_id"], "alias": rec["alias"],
                                  "candidate_hash": rec["candidate_hash"], "observation_window": "simulated", "successor": null});
        let correlation = json!({"kind": kind, "release_id": release_id, "event_id": event_id, "agent_id": rec["agent_id"], "alias": rec["alias"], "candidate_hash": rec["candidate_hash"]});
        self.store.put_doc_new(CORRELATION_NS, &tenant, &format!("{release_id}:{kind}"), correlation);
        if kind == "published" {
            let run = SuccessorRun {
                tenant: tenant.clone(),
                unique_key: format!("successor:{release_id}"),
                release_id: release_id.clone(),
                agent_id: p["agent_id"].as_str().unwrap_or_default().into(),
                alias: p["alias"].as_str().unwrap_or_default().into(),
                candidate_hash: rec["candidate_hash"].as_str().unwrap_or_default().into(),
            };
            let created = self.sink().schedule(&run) == Scheduled::Created;
            response["successor"] = json!({"unique_key": run.unique_key, "created": created});
        }
        self.store.put_doc(EVENT_NS, &tenant, event_id, json!({"digest": digest, "response": response}));
        json_resp(202, response)
    }

    fn sink(&self) -> Arc<dyn SuccessorSink> {
        self.cfg.successors.clone().unwrap_or_else(|| Arc::new(StoreSuccessorSink(self.store.clone())))
    }
}
