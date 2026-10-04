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
    DuplicateEvidence(String),
    AlreadyContradicted(String),
    SensitiveContent(&'static str),
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

/// Second pass: whitespace and zero-width chars removed, fullwidth folded; also flags long base64-like blobs.
fn scan_normalized(text: &str) -> Option<&'static str> {
    let folded: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{2060}' | '\u{feff}' | '\u{00ad}'))
        .map(|c| match c as u32 {
            0xff01..=0xff5e => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
            _ => c,
        })
        .collect::<String>()
        .to_ascii_lowercase();
    for marker in ["password=", "password:", "passwd=", "token=", "secret=", "api_key=", "apikey=", "bearer", "privatekey-----", "ghp_", "gho_", "xoxb-", "xoxp-", "[at]", "(at)"] {
        if folded.contains(marker) {
            return Some("secret");
        }
    }
    let mut run = 0;
    for c in folded.chars().filter(|c| !matches!(c, '-' | '.' | '(' | ')' | '+')) {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= 9 {
            return Some("long_digit_run");
        }
    }
    let mut blob = 0;
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-') {
            blob += 1;
        } else {
            if blob >= 32 {
                return Some("secret");
            }
            blob = 0;
        }
    }
    None
}

/// Conservative, dependency-free scan for secrets and PII. Returns the reason class when sensitive.
pub fn sensitive_reason(text: &str) -> Option<&'static str> {
    if let Some(r) = scan_normalized(text) {
        return Some(r);
    }
    let lower = text.to_ascii_lowercase();
    for marker in ["password=", "password:", "passwd=", "token=", "secret=", "api_key=", "apikey=", "bearer ", "private key-----"] {
        if lower.contains(marker) {
            return Some("secret");
        }
    }
    let run_after = |prefix: &str, min: usize, pred: fn(char) -> bool| {
        text.match_indices(prefix).any(|(i, _)| text[i + prefix.len()..].chars().take_while(|c| pred(*c)).count() >= min)
    };
    if run_after("sk-", 16, |c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || run_after("AKIA", 16, |c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Some("secret");
    }
    for (i, _) in text.match_indices('@') {
        let before = text[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric());
        let dom: String = text[i + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '-').collect();
        if before && dom.contains('.') && !dom.ends_with('.') {
            return Some("email");
        }
    }
    let mut run = 0;
    for c in text.chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= 9 {
            return Some("long_digit_run");
        }
    }
    None
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
        let mut seen = std::collections::BTreeSet::new();
        if let Some(d) = refs.iter().find(|r| !seen.insert(r.as_str())) {
            return Err(MemError::DuplicateEvidence(d.clone()));
        }
        match refs.iter().find(|r| self.evidence.digest(r).is_none()) {
            Some(bad) => Err(MemError::UnresolvedEvidence(bad.clone())),
            None => Ok(()),
        }
    }
    pub fn add_note(&mut self, n: NewNote) -> Result<String, MemError> {
        for t in [&n.claim_key, &n.statement] {
            if let Some(r) = sensitive_reason(t) {
                return Err(MemError::SensitiveContent(r));
            }
        }
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
        if self.notes[id].status == Status::Contradicted {
            return Err(MemError::AlreadyContradicted(id.to_string()));
        }
        self.check_refs(&evidence)?;
        if let Some(d) = evidence.iter().find(|e| self.notes[id].evidence.contains(e)) {
            return Err(MemError::DuplicateEvidence(d.clone()));
        }
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
        if p.status == Status::Contradicted {
            return Err(MemError::AlreadyContradicted(prior.to_string()));
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
