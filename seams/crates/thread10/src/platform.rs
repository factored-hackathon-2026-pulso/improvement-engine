//! Release correlation through the control-api (P2R) in process: `App::handle` over a MemStore, a throwaway Ed25519 key,
//! a fixed clock. The platform is a DOUBLE (no socket, no real exporter); the correlation code under test is real.
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub release_id: String,
    pub agent_id: String,
    pub alias: String,
    pub candidate_hash: String,
}

pub struct Platform;

impl Platform {
    pub fn new() -> Platform {
        Platform
    }
    pub fn record_release(&self, _r: &Release) -> Result<(), String> {
        Err("not implemented".into())
    }
    /// `release.published` for `r` under `event_id`: (HTTP status, body).
    pub fn post_published(&self, _event_id: &str, _r: &Release) -> (u16, Value) {
        (0, Value::Null)
    }
    pub fn successor_keys(&self) -> Vec<String> {
        vec!["not implemented".into()]
    }
}

impl Default for Platform {
    fn default() -> Self {
        Platform::new()
    }
}
