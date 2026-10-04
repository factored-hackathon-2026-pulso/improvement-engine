//! The golden FakeCore data (`tests/common/golden_flows.json`) is generated from
//! `bridge-contract/examples/flows`. It must be regenerated when the goldens change:
//! `python seams/crates/core-client/gen/gen_fakecore.py`.
mod common;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn flows_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../bridge-contract/examples/flows")
}

#[test]
fn golden_flows_json_is_in_sync_with_the_bridge_goldens() {
    let mut names: Vec<_> = std::fs::read_dir(flows_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    assert!(!names.is_empty());
    let mut h = Sha256::new();
    for n in &names {
        h.update(n.as_bytes());
        h.update([0u8]);
        let bytes = std::fs::read(flows_dir().join(n)).unwrap();
        h.update(String::from_utf8_lossy(&bytes).replace("\r\n", "\n").as_bytes());
        h.update([0u8]);
    }
    let now: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        now,
        common::golden::source_sha256(),
        "bridge-contract/examples/flows changed: run `python seams/crates/core-client/gen/gen_fakecore.py`"
    );
}

#[test]
fn every_golden_step_is_available_to_the_fake() {
    for (flow, case) in [
        ("invoke_scout", "invoke_scout_ok"),
        ("arms", "arm_run_native"),
        ("writer_evaluation", "admission_created"),
        ("authoring", "dry_run_valid"),
        ("authoring", "alias_read"),
        ("credentials", "issue_core_task"),
        ("auth_and_envelope", "version_ok"),
    ] {
        let s = common::golden::step(flow, case);
        assert_eq!(s.case, case);
        assert!((200..=422).contains(&s.status));
    }
}
