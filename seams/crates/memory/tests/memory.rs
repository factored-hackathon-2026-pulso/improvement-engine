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
