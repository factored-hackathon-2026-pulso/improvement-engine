//! Post-run memory note (MEM1 thin memory: `scope=demo1_thin`, `durable=false`; the note dies with the process).
//! The evidence is the job's own committed events, each a resolvable ref with a digest; the statement carries labels only.
use memory::{EvidenceStore, Memory, NewNote};
use serde_json::Value;

pub fn post_run_note(job: &str, events: &[String], gate: Option<&str>, published: bool) -> Result<Value, String> {
    let mut ev = EvidenceStore::new();
    let refs: Vec<String> = events
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let id = format!("ev-{job}-{i:02}");
            ev.insert(&id, e);
            id
        })
        .collect();
    let mut m = Memory::new(ev);
    let statement = format!(
        "{job} ran the offline thread host=rust: gate={} published={} human=simulated quality_claims=forbidden (Core is a double)",
        gate.unwrap_or("none"),
        if published { "staging-double" } else { "no" }
    );
    let id = m.add_note(NewNote { claim_key: format!("thread10:{job}:outcome"), statement, evidence: refs }).map_err(|e| format!("{e:?}"))?;
    m.artifact(&id).ok_or_else(|| "note vanished".to_string())
}
