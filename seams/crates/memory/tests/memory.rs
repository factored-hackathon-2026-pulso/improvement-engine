use memory::*;

fn store() -> EvidenceStore {
    let mut s = EvidenceStore::new();
    s.insert("ev-1", "digest-only observation A");
    s.insert("ev-2", "digest-only observation B");
    s
}
fn nn(key: &str, st: &str, ev: &[&str]) -> NewNote {
    NewNote { claim_key: key.into(), statement: st.into(), evidence: ev.iter().map(|e| e.to_string()).collect() }
}

#[test]
fn note_artifact_carries_evidence_refs_and_honest_labels() {
    let mut m = Memory::new(store());
    let id = m.add_note(nn("k.latency", "p95 latency is stable", &["ev-1"])).unwrap();
    let a = m.artifact(&id).expect("artifact");
    assert_eq!(a["evidence_refs"], serde_json::json!(["ev-1"]));
    assert_eq!(a["claim_key"], "k.latency");
    assert_eq!(a["status"], "active");
    assert_eq!(a["scope"], "demo1_thin");
    assert_eq!(a["durable"], false);
    assert_eq!(m.note(&id).unwrap().statement, "p95 latency is stable");
}

#[test]
fn wiki_read_renders_notes_for_a_claim_with_status_and_refs() {
    let mut m = Memory::new(store());
    m.add_note(nn("k.latency", "p95 latency is stable", &["ev-1"])).unwrap();
    m.add_note(nn("k.other", "unrelated", &["ev-2"])).unwrap();
    let page = m.wiki_read("k.latency").expect("page");
    assert!(page.contains("# k.latency"));
    assert!(page.contains("p95 latency is stable"));
    assert!(page.contains("[active]"));
    assert!(page.contains("ev-1"));
    assert!(!page.contains("unrelated"));
    assert!(m.wiki_read("k.missing").is_none());
}

#[test]
fn confirm_marks_note_confirmed_and_appends_evidence() {
    let mut m = Memory::new(store());
    let id = m.add_note(nn("k.latency", "p95 latency is stable", &["ev-1"])).unwrap();
    m.confirm(&id, vec!["ev-2".into()]).unwrap();
    let n = m.note(&id).unwrap();
    assert_eq!(n.status, Status::Confirmed);
    assert_eq!(n.evidence, vec!["ev-1", "ev-2"]);
    assert!(m.wiki_read("k.latency").unwrap().contains("[confirmed]"));
    assert!(matches!(m.confirm("note-9999", vec![]), Err(MemError::UnknownNote(_))));
}

#[test]
fn contradict_supersedes_prior_claim_and_links_both_ways() {
    let mut m = Memory::new(store());
    let prior = m.add_note(nn("k.latency", "p95 latency is stable", &["ev-1"])).unwrap();
    let new = m.contradict(&prior, nn("k.latency", "p95 latency regressed", &["ev-2"])).unwrap();
    assert_eq!(m.note(&prior).unwrap().status, Status::Contradicted);
    assert_eq!(m.note(&prior).unwrap().contradicted_by.as_deref(), Some(new.as_str()));
    assert_eq!(m.note(&new).unwrap().contradicts.as_deref(), Some(prior.as_str()));
    assert_eq!(m.note(&new).unwrap().status, Status::Active);
    let page = m.wiki_read("k.latency").unwrap();
    assert!(page.contains("[contradicted]") && page.contains("regressed"));
    assert_eq!(m.artifact(&new).unwrap()["contradicts"], prior.as_str());
    assert!(matches!(m.contradict("note-9999", nn("k.latency", "x", &["ev-1"])), Err(MemError::UnknownNote(_))));
    // different claim key is not a contradiction of the prior claim
    let other = m.add_note(nn("k.a", "a", &["ev-1"])).unwrap();
    assert!(matches!(m.contradict(&other, nn("k.b", "b", &["ev-1"])), Err(MemError::ClaimKeyMismatch)));
}

#[test]
fn refs_that_do_not_resolve_are_rejected_without_mutation() {
    let mut m = Memory::new(store());
    assert_eq!(m.add_note(nn("k", "s", &["ev-404"])), Err(MemError::UnresolvedEvidence("ev-404".into())));
    assert_eq!(m.add_note(nn("k", "s", &[])), Err(MemError::NoEvidence));
    assert!(m.wiki_read("k").is_none());
    let id = m.add_note(nn("k", "s", &["ev-1"])).unwrap();
    assert_eq!(m.confirm(&id, vec!["ev-404".into()]), Err(MemError::UnresolvedEvidence("ev-404".into())));
    assert_eq!(m.note(&id).unwrap().status, Status::Active);
    assert_eq!(m.note(&id).unwrap().evidence, vec!["ev-1"]);
    assert_eq!(m.contradict(&id, nn("k", "t", &["ev-404"])), Err(MemError::UnresolvedEvidence("ev-404".into())));
    assert_eq!(m.note(&id).unwrap().status, Status::Active);
    assert_eq!(m.wiki_read("k").unwrap().matches("- note-").count(), 1);
}

#[test]
fn artifact_pins_each_resolved_ref_to_a_sha256_digest() {
    let mut m = Memory::new(store());
    let id = m.add_note(nn("k", "s", &["ev-1"])).unwrap();
    let d = m.artifact(&id).unwrap()["evidence_digests"]["ev-1"].as_str().unwrap().to_string();
    assert_eq!(d.len(), 64);
    assert_eq!(d, sha_hex("digest-only observation A"));
}
fn sha_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn notes_with_secrets_or_pii_are_rejected_on_every_path() {
    let mut m = Memory::new(store());
    let cases = [
        ("contact ana@example.com about it", "email"),
        ("key sk-abcdefghijklmnop1234", "secret"),
        ("aws AKIAABCDEFGHIJKLMNOP", "secret"),
        ("password=hunter2", "secret"),
        ("Authorization: Bearer abc.def.ghi", "secret"),
        ("call 5551234567 now", "digits"),
    ];
    for (text, _) in cases {
        assert!(matches!(m.add_note(nn("k", text, &["ev-1"])), Err(MemError::SensitiveContent(_))), "{text}");
    }
    assert!(matches!(m.add_note(nn("k.ana@example.com", "ok", &["ev-1"])), Err(MemError::SensitiveContent(_))));
    let id = m.add_note(nn("k", "clean statement, 42 samples", &["ev-1"])).unwrap();
    assert!(matches!(m.contradict(&id, nn("k", "token=abcd1234", &["ev-2"])), Err(MemError::SensitiveContent(_))));
    assert_eq!(m.note(&id).unwrap().status, Status::Active);
    assert!(m.wiki_read("k").unwrap().matches("- note-").count() == 1);
}

#[test]
fn contradicted_note_cannot_be_reconfirmed_or_recontradicted() {
    let mut m = Memory::new(store());
    let prior = m.add_note(nn("k", "a", &["ev-1"])).unwrap();
    let new = m.contradict(&prior, nn("k", "b", &["ev-2"])).unwrap();
    assert!(matches!(m.contradict(&prior, nn("k", "c", &["ev-2"])), Err(MemError::AlreadyContradicted(_))));
    assert!(matches!(m.confirm(&prior, vec!["ev-2".into()]), Err(MemError::AlreadyContradicted(_))));
    assert_eq!(m.note(&prior).unwrap().contradicted_by.as_deref(), Some(new.as_str()));
    assert_eq!(m.note(&prior).unwrap().status, Status::Contradicted);
    assert_eq!(m.wiki_read("k").unwrap().matches("- note-").count(), 2);
}

#[test]
fn duplicate_evidence_refs_are_rejected() {
    let mut m = Memory::new(store());
    assert!(matches!(m.add_note(nn("k", "s", &["ev-1", "ev-1"])), Err(MemError::DuplicateEvidence(_))));
    let id = m.add_note(nn("k", "s", &["ev-1"])).unwrap();
    assert!(matches!(m.confirm(&id, vec!["ev-1".into()]), Err(MemError::DuplicateEvidence(_))));
    assert_eq!(m.note(&id).unwrap().status, Status::Active);
}

#[test]
fn scan_bypass_attempts_are_rejected() {
    let mut m = Memory::new(store());
    for text in ["Password = hunter2", "pass\u{200b}word=x", "call 555 123 4567", "call 555-123-4567", "ghp_abcdefghijklmnopqrstuvwxyz0123456789", "blob QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo0MTIz", "ana [at] x.com", "\u{ff50}assword=x"] {
        assert!(matches!(m.add_note(nn("k", text, &["ev-1"])), Err(MemError::SensitiveContent(_))), "{text:?}");
    }
    assert!(m.add_note(nn("k", "p95 stable over 42 samples, run 2026-10-04", &["ev-1"])).is_ok());
}
