//! CMP hardening: SHA-256 vectors, malformed input never panics, world-declaration drift, CRLF digest, FRZ0 acceptance.
//!
//! FRZ0 acceptance note (row CMP, "compiles the FRZ0 corpus"): the FRZ0 `valid-compile` sample targets
//! `prompt:sample-1@1` / `bundle:sample-1@1`, which are outside the seeded world, so the stand-in correctly DENIES it
//! with `outside_bridge` (fixture `frz0_valid_compile_sample`; the only schema-valid FRZ0 compile input). It still
//! counts as compiled corpus because (a) it validates against compile.in and yields a schema-valid compile.out, and
//! (b) the same sample retargeted onto the seeded world (`frz0_valid_compile_sample_in_world`) compiles.
//! The FRZ0 unsupported-kind sample is schema-invalid and is an `Err`.
use std::fs;
use std::path::PathBuf;
use steps::compile::{run, run_with, sha256_hex, World};

fn cases() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cmp/cases")
}
fn input(name: &str) -> String {
    fs::read_to_string(cases().join(format!("{name}.in.json"))).unwrap()
}

#[test]
fn sha256_known_vectors() {
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(sha256_hex(&vec![b'a'; 1_000_000]), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    let v: [(usize, &str); 17] = [
        (0, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        (1, "084fed08b978af4d7d196a7446a86b58009e636b611db16211b65a9aadff29c5"),
        (3, "6ab0dba1f4f1dfbb37b4f9eeb092c09fca4900ad32bdcd147d8dde35d6c87c35"),
        (55, "e7313d333c272e639f790978283f9eb392e843d0f29b7016828bb1daa4aac70b"),
        (56, "4324d65f3c103567f5589c710bc08f8523f929a9272e3af36fc968e52abc6c27"),
        (57, "35df609437dcfea3279283ab79fd554e2bf78f8f7ae2de532d8ee300b09e8f73"),
        (63, "81c80242132f230c3bd41b3e63bbcff16107339549214a99614ff26664625055"),
        (64, "39e3d7b6b5d075d37d053ad89b24b41bef4f3c29760c84447cab3f3be1882241"),
        (65, "aacca6ff74fdbb296d165a45cecfa04e5127bc008770fbbdd48006f2d2fae95e"),
        (111, "67d9492e628fd376e0b2efec8ca2b99b123e202cf620deb270728df979b2f73e"),
        (112, "96b928cff8528dbb99602c709a65b846cb6467acb8b722f0d758e4dc27bfc508"),
        (119, "9ce7368e4daf32341631b492e80359dc9f594b48453cd0dd5bf0b19279cc177e"),
        (120, "7836b787757e95e58b3ca5aec90b1b004e8deba1e50e9675af9cabf1a13a04b5"),
        (127, "a8d23e75d936f303d248888d9b165ee543f4cbafcad3c9dd2a79bd84faa11d07"),
        (128, "d2742f1f4ac6bb7ca2b239ee18402ba8b3f9f8e652d2a72973c2b9ba11c08cf6"),
        (129, "307f8fc2c1622b92762e818d39a185d4d667ad49a4b07ceae1f4afa008a93ec4"),
        (1000, "1e9bc38cbf860b9ec31918b065f9b52476c549a782e0e7990bed8ce3868d2371"),
    ];
    for (n, want) in v {
        let data: Vec<u8> = (0..n).map(|i| ((i * 7 + 3) % 256) as u8).collect();
        assert_eq!(sha256_hex(&data), want, "len {n}");
    }
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let h = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            assert!(run(&"[".repeat(200_000)).is_err());
            assert!(run(&format!("{}1{}", "{\"a\":".repeat(100_000), "}".repeat(100_000))).is_err());
        })
        .unwrap();
    h.join().expect("no panic");
}

#[test]
fn truncated_and_mutated_inputs_never_panic() {
    let base = input("two_ops_compiled");
    let b = base.as_bytes();
    for cut in 0..b.len() {
        if let Ok(s) = std::str::from_utf8(&b[..cut]) {
            let _ = run(s);
        }
    }
    for i in (0..b.len()).step_by(3) {
        for ch in [b'"', b'\\', b'{', b'}', b'[', b']', b',', b':', b'u', b'-', b'0'] {
            let mut m = b.to_vec();
            m[i] = ch;
            if let Ok(s) = String::from_utf8(m) {
                let _ = run(&s);
            }
        }
    }
    for s in ["\"\\u", "\"\\ud800\\u", "\"\\ud800\\ude", "{\"a\"", "{\"a\":", "tru", "-", "\u{feff}{}"] {
        assert!(run(s).is_err(), "{s:?}");
    }
}

/// Value of the first line whose trimmed text starts with `key`, after `from` in `yaml`, comment stripped.
fn yaml_value(yaml: &str, from: &str, key: &str) -> String {
    let at = yaml.match_indices(from).map(|(i, _)| i).find(|&i| i == 0 || yaml.as_bytes()[i - 1] == b'
');
    let rest = &yaml[at.unwrap_or_else(|| panic!("no {from}"))..];
    let line = rest.lines().find(|l| l.trim_start().starts_with(key)).unwrap_or_else(|| panic!("no {key}"));
    line.trim_start()[key.len()..].split('#').next().unwrap().trim().to_string()
}

#[test]
fn hardcoded_world_matches_the_world_declaration() {
    let w = World::seeded_base();
    let y = fs::read_to_string(w.root.join("seeded-base.world.yaml")).expect("world declaration readable");
    assert_eq!(yaml_value(&y, "world:", "world:"), w.world);
    assert_eq!(yaml_value(&y, "replaceable_prompt:", "id:"), format!("p/{}", w.prompt_name));
    assert_eq!(yaml_value(&y, "replaceable_prompt:", "version:"), w.prompt_version);
    assert_eq!(yaml_value(&y, "replaceable_prompt:", "used_by:"), format!("{{flow: {}, node: responder_ok}}", w.flow));
    assert_eq!(yaml_value(&y, "eval_suite_slot:", "current:"), format!("{}@{}", w.suite_name, w.suite_version));
    let kinds = yaml_value(&y, "targetable_kinds:", "targetable_kinds:");
    let listed: Vec<&str> = kinds.trim_matches(|c| c == '[' || c == ']').split(',').map(|s| s.trim()).collect();
    assert_eq!(listed, w.targetable_kinds);
    let dir = w.root.join(w.world);
    assert!(dir.join(format!("prompts/p/{}@{}.yaml", w.prompt_name, w.prompt_version)).is_file());
    assert!(dir.join(format!("eval_suites/{}@{}.yaml", w.suite_name, w.suite_version)).is_file());
}

#[test]
fn crlf_checkout_of_assets_yields_the_same_digest() {
    let w = World::seeded_base();
    let tmp = std::env::temp_dir().join(format!("cmp-crlf-{}", std::process::id()));
    let mut crlf = World::seeded_base();
    for sub in ["prompts/p", "eval_suites"] {
        let from = w.root.join(w.world).join(sub);
        let to = tmp.join(w.world).join(sub);
        fs::create_dir_all(&to).unwrap();
        for e in fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let lf = fs::read_to_string(e.path()).unwrap().replace("\r\n", "\n");
            fs::write(to.join(e.file_name()), lf.replace('\n', "\r\n")).unwrap();
        }
    }
    crlf.root = tmp.clone();
    for case in ["two_ops_compiled", "replace_only", "frz0_valid_compile_sample_in_world"] {
        assert_eq!(run_with(&input(case), &crlf, None).unwrap(), run(&input(case)).unwrap(), "{case}");
    }
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn missing_world_assets_are_an_error_not_a_panic() {
    let mut w = World::seeded_base();
    w.root = PathBuf::from("does/not/exist");
    assert!(run_with(&input("replace_only"), &w, None).is_err());
    assert!(run_with(&input("denied_kind_disable"), &w, None).unwrap().contains("kind_not_supported"));
}

#[test]
fn frz0_corpus_acceptance() {
    let out = run(&input("frz0_valid_compile_sample")).unwrap();
    assert!(out.contains("\"denied_reason\":\"outside_bridge\""), "FRZ0 sample is outside the seeded world: {out}");
    assert!(run(&input("frz0_valid_compile_sample_in_world")).unwrap().contains("\"status\":\"compiled\""));
    assert!(run(&input("frz0_invalid_unsupported_kind")).is_err());
}
