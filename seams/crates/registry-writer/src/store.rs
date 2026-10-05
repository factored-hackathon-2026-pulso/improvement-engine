//! Local receipts: key -> proposal. The registry at main has no list route and no `Idempotency-Key` on HTTP, so this is what makes a
//! retry for the same finding not open a second proposal. Receipts hold ids and timestamps only (no token, no text).
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub proposal_id: String,
    pub agent_id: String,
    pub created_at: u64,
}

pub trait ReceiptStore {
    fn get(&self, key: &str) -> Option<Receipt>;
    fn put(&self, key: &str, r: Receipt) -> Result<(), String>;
    fn forget(&self, key: &str) -> Result<(), String>;
    /// Receipts created at or after `since` (unix seconds): the local view of the 24 h proposal quota.
    fn created_since(&self, since: u64) -> usize;
}

#[derive(Default)]
pub struct MemoryStore(RefCell<BTreeMap<String, Receipt>>);

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }
}

impl ReceiptStore for MemoryStore {
    fn get(&self, key: &str) -> Option<Receipt> {
        self.0.borrow().get(key).cloned()
    }
    fn put(&self, key: &str, r: Receipt) -> Result<(), String> {
        self.0.borrow_mut().insert(key.into(), r);
        Ok(())
    }
    fn forget(&self, key: &str) -> Result<(), String> {
        self.0.borrow_mut().remove(key);
        Ok(())
    }
    fn created_since(&self, since: u64) -> usize {
        self.0.borrow().values().filter(|r| r.created_at >= since).count()
    }
}

/// One JSON file, rewritten atomically (temp file then rename). Single writer: the engine process.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: impl Into<PathBuf>) -> FileStore {
        FileStore { path: path.into() }
    }

    fn load(&self) -> BTreeMap<String, Receipt> {
        let Ok(raw) = std::fs::read_to_string(&self.path) else { return BTreeMap::new() };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else { return BTreeMap::new() };
        v["receipts"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, r)| Some((k.clone(), Receipt { proposal_id: r["proposal_id"].as_str()?.into(), agent_id: r["agent_id"].as_str()?.into(), created_at: r["created_at"].as_u64()? })))
            .collect()
    }

    fn save(&self, m: &BTreeMap<String, Receipt>) -> Result<(), String> {
        let receipts: serde_json::Map<String, Value> = m.iter().map(|(k, r)| (k.clone(), json!({"proposal_id": r.proposal_id, "agent_id": r.agent_id, "created_at": r.created_at}))).collect();
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&json!({"receipts": receipts})).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

impl ReceiptStore for FileStore {
    fn get(&self, key: &str) -> Option<Receipt> {
        self.load().get(key).cloned()
    }
    fn put(&self, key: &str, r: Receipt) -> Result<(), String> {
        let mut m = self.load();
        m.insert(key.into(), r);
        self.save(&m)
    }
    fn forget(&self, key: &str) -> Result<(), String> {
        let mut m = self.load();
        m.remove(key);
        self.save(&m)
    }
    fn created_since(&self, since: u64) -> usize {
        self.load().values().filter(|r| r.created_at >= since).count()
    }
}
