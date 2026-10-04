//! Synthetic stand-in for the sensor runner (`local-sim ... --output <dir> ...`): writes a fixed, aggregate-only
//! `<dir>/run-synth/result.json`. Reads no input data; exists so the `thread` job runs without real E0 rows.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let Some(out) = a.iter().position(|x| x == "--output").and_then(|i| a.get(i + 1)) else {
        eprintln!("synth_runner: --output <dir> required");
        std::process::exit(2);
    };
    let dir = std::path::Path::new(out).join("run-synth");
    std::fs::create_dir_all(&dir).expect("create run dir");
    let result = r#"{"signal":{"digest":"sha256:0000000000000000000000000000000000000000000000000000000000000001","metric_id":"e0_recurring_copilot_query_cases","numerator":22,"denominator":30,"minimum_support":5},"signals":[{"digest":"sha256:0000000000000000000000000000000000000000000000000000000000000001","metric_id":"e0_recurring_copilot_query_cases","numerator":22,"denominator":30,"minimum_support":5},{"digest":"sha256:0000000000000000000000000000000000000000000000000000000000000002","metric_id":"e0_technical_error_rate","numerator":0,"denominator":30,"minimum_support":1}],"e0_recurrence_holdout":{"status":"checked"}}"#;
    std::fs::write(dir.join("result.json"), result).expect("write result");
}
