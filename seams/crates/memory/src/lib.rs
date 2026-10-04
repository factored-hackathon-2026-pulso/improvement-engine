//! Thin memory (MEM1, DEMO-1). Labels: `scope=demo1_thin`, `durable=false`.
//! Notes live in process memory only; durable memory (head surviving kill -9) is DMEMC, not claimed here.
use std::collections::BTreeMap;

pub const SCOPE: &str = "demo1_thin";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemError {
    NotImplemented,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Active,
    Confirmed,
    Contradicted,
}

#[derive(Debug, Clone)]
pub struct NewNote {
    pub claim_key: String,
    pub statement: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Note {
    pub id: String,
    pub claim_key: String,
    pub statement: String,
    pub evidence: Vec<String>,
    pub status: Status,
}

#[derive(Default)]
pub struct EvidenceStore {
    items: BTreeMap<String, String>,
}

impl EvidenceStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, id: &str, body: &str) {
        self.items.insert(id.to_string(), body.to_string());
    }
}

pub struct Memory {
    #[allow(dead_code)]
    evidence: EvidenceStore,
}

impl Memory {
    pub fn new(evidence: EvidenceStore) -> Self {
        Self { evidence }
    }
    pub fn add_note(&mut self, _n: NewNote) -> Result<String, MemError> {
        Err(MemError::NotImplemented)
    }
    pub fn note(&self, _id: &str) -> Option<&Note> {
        None
    }
    pub fn artifact(&self, _id: &str) -> Option<serde_json::Value> {
        None
    }
}
