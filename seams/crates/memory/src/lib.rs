//! Thin memory (MEM1, DEMO-1). Labels: `scope=demo1_thin`, `durable=false`.
//! Notes live in process memory only; durable memory (head surviving kill -9) is DMEMC, not claimed here.
use std::collections::BTreeMap;

pub const SCOPE: &str = "demo1_thin";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemError {
    NotImplemented,
    UnknownNote(String),
    ClaimKeyMismatch,
    UnresolvedEvidence(String),
    NoEvidence,
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
    pub contradicts: Option<String>,
    pub contradicted_by: Option<String>,
}

#[derive(Default)]
pub struct EvidenceStore {
    items: BTreeMap<String, String>,
}

impl EvidenceStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn digest(&self, id: &str) -> Option<String> {
        use sha2::{Digest, Sha256};
        let body = self.items.get(id)?;
        Some(Sha256::digest(body.as_bytes()).iter().map(|b| format!("{b:02x}")).collect())
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
    fn check_refs(&self, refs: &[String]) -> Result<(), MemError> {
        if refs.is_empty() {
            return Err(MemError::NoEvidence);
        }
        match refs.iter().find(|r| self.evidence.digest(r).is_none()) {
            Some(bad) => Err(MemError::UnresolvedEvidence(bad.clone())),
            None => Ok(()),
        }
    }
    pub fn add_note(&mut self, n: NewNote) -> Result<String, MemError> {
        self.check_refs(&n.evidence)?;
        self.seq += 1;
        let id = format!("note-{:04}", self.seq);
        self.notes.insert(
            id.clone(),
            Note { id: id.clone(), claim_key: n.claim_key, statement: n.statement, evidence: n.evidence, status: Status::Active, contradicts: None, contradicted_by: None },
        );
        Ok(id)
    }
    /// Read-only wiki view of one claim: notes in creation order with status and evidence refs.
    pub fn wiki_read(&self, claim_key: &str) -> Option<String> {
        let mut out = format!("# {claim_key}
");
        let mut any = false;
        for n in self.notes.values().filter(|n| n.claim_key == claim_key) {
            any = true;
            out.push_str(&format!("- {} [{}] {} (evidence: {})
", n.id, n.status.as_str(), n.statement, n.evidence.join(", ")));
        }
        any.then_some(out)
    }
    pub fn confirm(&mut self, id: &str, evidence: Vec<String>) -> Result<(), MemError> {
        if !self.notes.contains_key(id) {
            return Err(MemError::UnknownNote(id.to_string()));
        }
        self.check_refs(&evidence)?;
        let n = self.notes.get_mut(id).unwrap();
        n.evidence.extend(evidence);
        n.status = Status::Confirmed;
        Ok(())
    }
    /// Record a new note that contradicts `prior` (same claim key); the prior is kept, marked contradicted.
    pub fn contradict(&mut self, prior: &str, n: NewNote) -> Result<String, MemError> {
        let p = self.notes.get(prior).ok_or_else(|| MemError::UnknownNote(prior.to_string()))?;
        if p.claim_key != n.claim_key {
            return Err(MemError::ClaimKeyMismatch);
        }
        let id = self.add_note(n)?;
        self.notes.get_mut(&id).unwrap().contradicts = Some(prior.to_string());
        let p = self.notes.get_mut(prior).unwrap();
        p.status = Status::Contradicted;
        p.contradicted_by = Some(id.clone());
        Ok(id)
    }
    pub fn note(&self, id: &str) -> Option<&Note> {
        self.notes.get(id)
    }
    pub fn artifact(&self, id: &str) -> Option<serde_json::Value> {
        let n = self.notes.get(id)?;
        let digests: BTreeMap<&String, Option<String>> = n.evidence.iter().map(|r| (r, self.evidence.digest(r))).collect();
        Some(serde_json::json!({
            "evidence_digests": digests,
            "id": n.id, "claim_key": n.claim_key, "statement": n.statement,
            "evidence_refs": n.evidence, "status": n.status.as_str(),
            "contradicts": n.contradicts, "contradicted_by": n.contradicted_by, "scope": SCOPE, "durable": false,
        }))
    }
}
