//! Drift guard: the generated pin constants, routes and contract-derived tables must match
//! `bridge-contract/` and ADR 0012. Fails when the pin moves without regenerating `src/pins.rs`.
use core_client::{pins, routes};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").join(p)
}
fn read(p: &str) -> String {
    std::fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn pin_constants_match_contract_json() {
    let c: Value = serde_json::from_str(&read("bridge-contract/contract.json")).unwrap();
    assert_eq!(c["pin"]["agent_core_sha"], pins::AGENT_CORE_SHA);
    assert_eq!(c["pin"]["contracts_version"], pins::CONTRACTS_VERSION);
    assert_eq!(c["base_path"], pins::BASE_PATH);
    assert_eq!(c["contract_revision"], pins::CONTRACT_REVISION);
    assert_eq!(c["auth"]["token"]["max_ttl_seconds"].as_u64().unwrap_or(300), 300);
    assert_eq!(c["auth"]["token"]["iss"], pins::JWT_ISS);
    assert_eq!(c["auth"]["token"]["aud"], pins::JWT_AUD);
}

#[test]
fn pin_constants_match_runtime_source() {
    let init = read("core-bridge/src/pulso_core_runtime/__init__.py");
    assert!(init.contains(&format!("PIN_SHA = \"{}\"", pins::AGENT_CORE_SHA)));
    assert!(init.contains(&format!("CONTRACTS_VERSION = \"{}\"", pins::CONTRACTS_VERSION)));
}

#[test]
fn manifest_digest_matches_adr_0012_and_the_manifest_file() {
    let adr = read("core-bridge/docs/adr/0012-agent-core-pin-c814c2b.md");
    assert!(adr.contains(pins::MANIFEST_SHA256), "ADR 0012 does not record the MANIFEST digest");
    let bytes = std::fs::read(repo("core-bridge/wire/agent_core@c814c2b/MANIFEST.json")).unwrap();
    let lf: Vec<u8> = String::from_utf8_lossy(&bytes).replace("\r\n", "\n").into_bytes();
    let hex = |b: &[u8]| Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert!(
        hex(&bytes) == pins::MANIFEST_SHA256 || hex(&lf) == pins::MANIFEST_SHA256,
        "MANIFEST.json drifted from pins::MANIFEST_SHA256"
    );
    assert!(pins::AGENT_CORE_SHA.starts_with(pins::AGENT_CORE_SHORT));
}

#[test]
fn route_table_equals_contract_routes() {
    let c: Value = serde_json::from_str(&read("bridge-contract/contract.json")).unwrap();
    let want = c["routes"].as_array().unwrap();
    assert_eq!(routes::ALL.len(), want.len());
    for w in want {
        let found = routes::ALL
            .iter()
            .find(|r| w["method"] == r.method && w["path"] == r.path)
            .unwrap_or_else(|| panic!("route {w} missing"));
        assert_eq!(w["id"], found.id);
        assert_eq!(w["purposes"][0], found.purpose);
        assert_eq!(w["tenant_required"], found.tenant_required);
        assert_eq!(w["purposes"].as_array().unwrap().len(), 1);
    }
}
