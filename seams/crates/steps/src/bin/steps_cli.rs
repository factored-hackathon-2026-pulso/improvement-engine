//! `steps_cli <step>`: reads the step input JSON on stdin, writes the output JSON on stdout.
//! The semantics label goes to stderr so the stdout JSON stays schema-exact.
//! Exit codes: 0 ok, 1 step error (message on stderr), 2 usage error.
//! Env: STEPS_RUNNER_EXE, STEPS_SNAPSHOT_ROOT (sensor); STEPS_LAB_DIR (recompute);
//! STEPS_RECOMPUTE_DIR (intent).
use std::io::{Read, Write};

use steps::{SEMANTICS, StepError};

type StepFn = fn(&str) -> Result<String, StepError>;

/// Dispatch table: CLI name -> step function. Each lane adds its own arm.
const TABLE: &[(&str, StepFn)] = &[
    // --- STP1 ---
    ("sensor", steps::sensor::run),
    ("recompute", steps::recompute::run),
    ("intent", steps::intent::run),
    // --- CMP ---
    ("compile", compile_step),
    // --- L1 ---
    ("cells", steps::cells::run),
    ("cells_agent_runs", steps::cells::run_agent_runs),
    // --- GSI ---
    ("gate", gate_step),
];

// compile/gate have their own error types; the CLI maps them to StepError (exit 1).
fn compile_step(input: &str) -> Result<String, StepError> {
    steps::compile::run(input).map_err(|e| StepError::Invalid(format!("compile: {}", e.0)))
}

fn gate_step(input: &str) -> Result<String, StepError> {
    steps::gate::run(input).map_err(|e| StepError::Invalid(e.to_string()))
}

fn main() {
    let name = std::env::args().nth(1).unwrap_or_default();
    let Some((_, f)) = TABLE.iter().find(|(n, _)| *n == name) else {
        let known: Vec<&str> = TABLE.iter().map(|(n, _)| *n).collect();
        eprintln!("usage: steps_cli <{}> < input.json > output.json", known.join("|"));
        std::process::exit(2);
    };
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("cannot read stdin: {e}");
        std::process::exit(2);
    }
    match f(&input) {
        Ok(out) => {
            eprintln!("semantics: {SEMANTICS}");
            let _ = std::io::stdout().write_all(out.as_bytes());
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
