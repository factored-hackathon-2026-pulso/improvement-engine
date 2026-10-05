//! `reason_cli`: run the three roles over the corroborated signals of a cells report and print the run report (JSON) to stdout.
//!
//! ```text
//! reason_cli (--report <cells-report.json> | --ndjson <cells.ndjson> | --synthetic-demo)
//!            [--source synthetic|bank|e0] [--allow-derived-aggregates] [--catalog <base_artifacts.json>] [--mode live|scripted]
//! reason_cli dossier --proposal <proposal-or-run.json> --finding <signal-or-cells-report.json> [--verdict <reg1 story.json>]
//!            [--signal-index N] [--finding-id ID] [--runtime doubles|real|local-model] [--judge-family F] [--calibrated]
//!            [--lang es|pt|both] [--format json|md]
//! ```
//! `dossier` renders the decision dossier (W1-2) offline: no model call, deterministic, PII-shaped text refused. Without a
//! verdict the dossier says the proposal was not evaluated and `announce` is false.
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

fn load(p: &str) -> Result<Value, String> {
    serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?).map_err(|e| format!("{p}: {e}"))
}

fn dossier_cmd(args: &[String]) -> Result<(), String> {
    use reasoning::dossier::{Labels, Lang, Runtime, build, render_markdown};
    let val = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let proposal_file = load(&val("--proposal").ok_or("--proposal <file> is required")?)?;
    let finding_file = load(&val("--finding").ok_or("--finding <file> is required")?)?;
    // A run report holds many reasoning records: take the one named by --finding-id, else the first proposed one.
    let record = match proposal_file["findings"].as_array() {
        Some(list) => {
            let id = val("--finding-id");
            list.iter().find(|r| id.as_deref().map_or(r["status"] == "proposed", |i| r["finding_id"] == i)).cloned().ok_or("no matching proposed record in the run report")?
        }
        None => proposal_file,
    };
    let finding = match finding_file["signals"].as_array() {
        Some(list) => {
            let i: usize = val("--signal-index").map_or(Ok(0), |v| v.parse().map_err(|_| "--signal-index is not a number".to_string()))?;
            list.get(i).cloned().ok_or("--signal-index is out of range")?
        }
        None => finding_file,
    };
    let verdict = val("--verdict").map(|p| load(&p)).transpose()?;
    let labels = Labels {
        runtime: match val("--runtime") { Some(r) => Runtime::parse(&r).ok_or("--runtime is not doubles|real|local-model")?, None => Runtime::Doubles },
        rubric: None,
        judge_family: val("--judge-family"),
        calibrated: args.iter().any(|a| a == "--calibrated"),
    };
    let d = build(&finding, &record, verdict.as_ref(), &labels)?;
    let langs: Vec<Lang> = match val("--lang").as_deref().unwrap_or("both") {
        "es" => vec![Lang::Es],
        "pt" => vec![Lang::Pt],
        "both" => vec![Lang::Es, Lang::Pt],
        o => return Err(format!("--lang {o:?} is not es|pt|both")),
    };
    match val("--format").as_deref().unwrap_or("json") {
        "json" => println!("{}", serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?),
        "md" => langs.iter().for_each(|l| println!("{}", render_markdown(&d, *l))),
        o => return Err(format!("--format {o:?} is not json|md")),
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("dossier") {
        return dossier_cmd(&args[1..]);
    }
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
