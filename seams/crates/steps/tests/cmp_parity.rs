//! Byte-for-byte parity with the Python reference (e2e-core compile_step.py) on the shared fixtures.
//! Expected files come from fixtures/cmp/gen_expected.py (canonical JSON, label normalised to claude-standin).
use std::fs;
use std::path::PathBuf;
use steps::compile::run;

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
        if dir.join(format!("{stem}.error")).exists() {
            assert!(got.is_err(), "{stem}: reference rejects the input");
        } else {
            let want = fs::read_to_string(dir.join(format!("{stem}.expected.json"))).unwrap();
            assert_eq!(got.unwrap_or_else(|e| panic!("{stem}: {e:?}")), want, "{stem}");
        }
        n += 1;
    }
    assert!(n >= 18, "corpus too small: {n}");
}
