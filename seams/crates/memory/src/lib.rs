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
    notes: BTreeMap<String, Note>,
    seq: u64,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Confirmed => "confirmed",
            Status::Contradicted => "contradicted",
        }
    }
}

impl Memory {
    pub fn new(evidence: EvidenceStore) -> Self {
        Self { evidence, notes: BTreeMap::new(), seq: 0 }
    }
    pub fn add_note(&mut self, n: NewNote) -> Result<String, MemError> {
        self.seq += 1;
        let id = format!("note-{:04}", self.seq);
        self.notes.insert(
            id.clone(),
            Note { id: id.clone(), claim_key: n.claim_key, statement: n.statement, evidence: n.evidence, status: Status::Active },
        );
        Ok(id)
    }
    pub fn note(&self, id: &str) -> Option<&Note> {
        self.notes.get(id)
    }
    pub fn artifact(&self, id: &str) -> Option<serde_json::Value> {
        let n = self.notes.get(id)?;
        Some(serde_json::json!({
            "id": n.id, "claim_key": n.claim_key, "statement": n.statement,
            "evidence_refs": n.evidence, "status": n.status.as_str(),
            "scope": SCOPE, "durable": false,
        }))
    }
}
