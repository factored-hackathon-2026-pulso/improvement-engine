//! Byte-for-byte parity with the Python reference (e2e-core compile_step.py) on the shared fixtures.
//! Expected files come from fixtures/cmp/gen_expected.py (canonical JSON, label normalised to claude-standin).
use std::fs;
use std::path::PathBuf;
use steps::compile::run;

/// Inputs the Python reference accepts only because Python's `$` matches before a trailing newline.
const STRICTER: [&str; 1] = ["raw_trailing_newline_ref"];

#[test]
fn matches_python_reference_on_all_cases() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cmp/cases");
    let mut n = 0;
    let mut ins: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    ins.sort();
    for p in ins {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".in.json") else { continue };
        let input = fs::read_to_string(&p).unwrap();
        let got = run(&input);
        if STRICTER.contains(&stem) {
            // Documented divergence: the reference regex `$` also matches before a trailing newline; we follow the schema.
            assert!(got.is_err(), "{stem}: stricter than the reference by design");
            n += 1;
            continue;
        }
        if dir.join(format!("{stem}.error")).exists() {
            assert!(got.is_err(), "{stem}: reference rejects the input");
        } else {
            let want = fs::read_to_string(dir.join(format!("{stem}.expected.json"))).unwrap();
            assert_eq!(got.unwrap_or_else(|e| panic!("{stem}: {e:?}")), want, "{stem}");
        }
        n += 1;
    }
    assert!(n >= 34, "corpus too small: {n}");
}
