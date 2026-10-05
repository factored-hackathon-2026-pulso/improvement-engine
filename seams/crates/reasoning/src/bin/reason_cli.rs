//! `reason_cli`: run the three roles over the corroborated signals of a cells report and print the run report (JSON) to stdout.
//!
//! ```text
//! reason_cli (--report <cells-report.json> | --ndjson <cells.ndjson> | --synthetic-demo)
//!            [--source synthetic|bank|e0] [--allow-derived-aggregates] [--catalog <base_artifacts.json>] [--mode live|scripted]
//! ```
//! `--ndjson` runs the L1 sensor first (`steps::cells`). `--mode live` (default) needs the llm-gateway environment
//! (`PULSO_LLM_GATEWAY=enabled`, `PULSO_LLM_GATEWAY_ADDR`, `PULSO_LLM_GATEWAY_KEY`, optional `PULSO_LLM_GATEWAY_MODEL`,
//! `PULSO_LLM_GATEWAY_VERIFIER_MODEL`); `--mode scripted` only exercises the plumbing with fixed answers and labels it so.
use reasoning::catalog::Catalog;
use reasoning::finding::Source;
use reasoning::pipeline::{Opts, reason_report};
use reasoning::testkit::synthetic_cells_ndjson;
use serde_json::Value;

fn main() {
    if let Err(e) = run() {
        eprintln!("reason_cli: {e}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let val = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let has = |k: &str| args.iter().any(|a| a == k);
    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
    let report: Value = if let Some(p) = val("--report") {
        serde_json::from_str(&read(&p)?).map_err(|e| format!("report: {e}"))?
    } else if let Some(p) = val("--ndjson") {
        serde_json::from_str(&steps::cells::run(&read(&p)?).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?
    } else if has("--synthetic-demo") {
        serde_json::from_str(&steps::cells::run(&synthetic_cells_ndjson()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?
    } else {
        return Err("give --report, --ndjson or --synthetic-demo".into());
    };
    let source = match val("--source") {
        Some(s) => Source::parse(&s).ok_or_else(|| format!("--source {s:?} is not synthetic|bank|e0"))?,
        None if has("--synthetic-demo") => Source::Synthetic,
        None => return Err("--source is required for a real report (synthetic|bank|e0): the data class decides what a hosted model may see".into()),
    };
    let catalog = match val("--catalog") {
        Some(p) => Catalog::from_json(&serde_json::from_str(&read(&p)?).map_err(|e| e.to_string())?)?,
        None => Catalog::bundled(),
    };
    let opts = Opts { allow_derived_aggregates: has("--allow-derived-aggregates") };
    let ports = match val("--mode").as_deref().unwrap_or("live") {
        "live" => reasoning::live::ports_from_env(&|k| std::env::var(k).ok())?,
        "scripted" => return Err("scripted mode is exercised by the test-suite (cargo test -p reasoning); the CLI is for live runs".into()),
        o => return Err(format!("--mode {o:?} is not live|scripted")),
    };
    let out = reason_report(&catalog, &report, source, &ports, &opts)?;
    println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
    Ok(())
}
