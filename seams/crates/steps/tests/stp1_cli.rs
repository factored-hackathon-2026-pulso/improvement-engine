mod stp1_common;
use std::io::Write;
use std::process::{Command, Stdio};
use stp1_common::*;

fn cli(args: &[&str], stdin: &str, lab_dir: &std::path::Path) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_steps_cli"))
        .args(args)
        .env("STEPS_LAB_DIR", lab_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

#[test]
fn cli_recompute_reads_stdin_writes_stdout_and_labels_on_stderr() {
    let dir = std::env::temp_dir().join(format!("stp1-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sample-1.json"), read_fixture("lab-sample-1.json")).unwrap();
    let (code, out, err) = cli(&["recompute"], &read_fixture("recompute.in.json"), &dir);
    assert_eq!(code, 0, "{err}");
    assert_valid("recompute.out", &out);
    assert!(err.contains("semantics: claude-standin"), "{err}");
}

#[test]
fn cli_unknown_step_and_step_error_exit_nonzero() {
    let dir = std::env::temp_dir();
    let (code, _, err) = cli(&["bogus"], "{}", &dir);
    assert_eq!(code, 2, "{err}");
    let (code, out, err) = cli(&["intent"], "not json", &dir);
    assert_eq!(code, 1, "{err}");
    assert!(out.is_empty());
}
