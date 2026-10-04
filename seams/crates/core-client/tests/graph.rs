//! Dependency-graph test: `abi` and `core-client` must never depend on
//! Codex's `crates/core`. Std-only manifest scan (no cargo-metadata needed).
use std::fs;
use std::path::Path;

/// Returns offending dependency lines in a manifest that point at Codex core.
fn forbidden_core_deps(manifest: &str) -> Vec<String> {
    let mut in_deps = false;
    let mut bad = Vec::new();
    for raw in manifest.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_deps = line.contains("dependencies");
            continue;
        }
        if !in_deps || line.starts_with('#') {
            continue;
        }
        let key = line.split(['=', ' ']).next().unwrap_or("");
        if key == "core" || line.contains("crates/core") || line.contains("package = \"core\"") {
            bad.push(line.to_string());
        }
    }
    bad
}

#[test]
fn detects_abi_depending_on_codex_core() {
    let bad = "[package]\nname = \"abi\"\n[dependencies]\ncore = { path = \"../../../crates/core\" }\n";
    assert_eq!(forbidden_core_deps(bad).len(), 1);
    assert!(forbidden_core_deps("[dependencies]\nserde = \"1\"\n").is_empty());
}

#[test]
fn abi_and_core_client_are_free_of_crates_core() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for name in ["abi", "core-client"] {
        let m = crates.join(name).join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&m) {
            assert!(forbidden_core_deps(&text).is_empty(), "{name} depends on crates/core");
        }
    }
}

#[test]
fn workspace_is_nested_and_separate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let text = fs::read_to_string(root).unwrap();
    assert!(text.contains("[workspace]") && text.contains("crates/*"));
}
