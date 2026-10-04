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
