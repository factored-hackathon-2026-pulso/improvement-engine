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
fn no_seams_crate_depends_on_crates_core() {
    // Every crate of the workspace (engine now depends on core-client/eval/authority): K0 graph rule, not a fixed list.
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut seen = 0;
    for e in fs::read_dir(crates).unwrap().filter_map(Result::ok) {
        if let Ok(text) = fs::read_to_string(e.path().join("Cargo.toml")) {
            seen += 1;
            assert!(forbidden_core_deps(&text).is_empty(), "{:?} depends on crates/core", e.file_name());
        }
    }
    assert!(seen >= 6, "scanned only {seen} manifests");
}

#[test]
fn workspace_is_nested_and_separate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let text = fs::read_to_string(root).unwrap();
    assert!(text.contains("[workspace]") && text.contains("crates/*"));
}
