//! E3b: the `compile` and `gate` CLI arms read stdin, write stdout, exit 0/1/2.
use std::io::Write;
use std::process::{Command, Stdio};

const GATE_CASES: &str = include_str!("fixtures/gsi/cases.json");
const COMPILE_IN: &str = include_str!("fixtures/cmp/cases/two_ops_compiled.in.json");

/// First case `input` object (brace-matched, string-aware) of the GSI fixture: the gate CLI envelope.
fn gate_input() -> String {
    let start = GATE_CASES.find("\"input\":").unwrap() + "\"input\":".len();
    let b = GATE_CASES.as_bytes();
    let (mut depth, mut in_str, mut esc, mut i) = (0i32, false, false, start);
    while b[i] != b'{' { i += 1 }
    let from = i;
    loop {
        let c = b[i];
        if in_str {
            if esc { esc = false } else if c == 92u8 { esc = true } else if c == b'"' { in_str = false }
        } else if c == b'"' { in_str = true } else if c == b'{' { depth += 1 } else if c == b'}' {
            depth -= 1;
            if depth == 0 { return GATE_CASES[from..=i].to_string() }
        }
        i += 1;
    }
}

fn cli(step: &str, stdin: &str) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_steps_cli"))
        .arg(step)
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
fn cli_compile_arm_matches_library() {
    let (code, out, err) = cli("compile", COMPILE_IN);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, steps::compile::run(COMPILE_IN).unwrap());
    assert!(out.contains("\"status\":\"compiled\""));
    assert!(err.contains("semantics: claude-standin"), "{err}");
}

#[test]
fn cli_compile_bad_input_exits_1_with_empty_stdout() {
    let (code, out, _) = cli("compile", "not json");
    assert_eq!(code, 1);
    assert!(out.is_empty());
}

#[test]
fn cli_gate_arm_matches_library() {
    let input = gate_input();
    let (code, out, err) = cli("gate", &input);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, steps::gate::run(&input).unwrap());
    assert!(out.contains("\"verdict\""), "{out}");
}

#[test]
fn cli_gate_bad_input_exits_1_and_unknown_step_exits_2() {
    let (code, out, _) = cli("gate", "not json");
    assert_eq!(code, 1);
    assert!(out.is_empty());
    let (code, _, err) = cli("bogus", "{}");
    assert_eq!(code, 2);
    assert!(err.contains("compile") && err.contains("gate"), "{err}");
}
