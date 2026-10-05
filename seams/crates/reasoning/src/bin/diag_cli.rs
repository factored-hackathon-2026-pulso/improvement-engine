//! `diag_cli`: capture the RAW Scout and Builder answers of a real model for every linked finding of a cells report, and classify
//! what is wrong with each (BLD1 diagnosis). Output is JSONL on stdout (one object per call) with the raw text, so the caller must
//! keep it OUTSIDE git. Needs the live gateway environment (see `reason_cli`).
//!
//! ```text
//! diag_cli --report <cells-report.json> --source bank [--catalog <base_artifacts.json>] [--mode text|prompted|native] [--reps N]
//! ```
//! The Builder is called with the Scout's opportunity when the Scout answer parsed, else with a stand-in opportunity (first target,
//! first mechanism, the discovery rate), so Builder failures are visible even when the Scout fails.
use engine::models::extract::extract_json_object;
use engine::models::llm_gateway::{LlmGateway, Structured};
use engine::models::{DataClass, ModelPort, ModelRequest};
use reasoning::catalog::Catalog;
use reasoning::finding::{Finding, Source, round2};
use reasoning::mapping::map_finding;
use reasoning::roles::{self, Opportunity};
use serde_json::{Value, json};

fn classify(raw: &str) -> (&'static str, Option<Value>) {
    let low = raw.to_lowercase();
    match extract_json_object(raw) {
        Ok(v) => {
            let packaging = if raw.trim_start().starts_with('{') && raw.trim_end().ends_with('}') {
                "bare"
            } else if raw.contains("```") {
                "fenced"
            } else if low.contains("<think>") {
                "reasoning_block"
            } else {
                "prose_around"
            };
            (packaging, Some(v))
        }
        Err(e) => (e.code(), None),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("diag_cli: {e}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let val = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
    let report: Value = serde_json::from_str(&read(&val("--report").ok_or("--report")?)?).map_err(|e| e.to_string())?;
    let source = Source::parse(&val("--source").ok_or("--source")?).ok_or("--source")?;
    let catalog = match val("--catalog") {
        Some(p) => Catalog::from_json(&serde_json::from_str(&read(&p)?).map_err(|e| e.to_string())?)?,
        None => Catalog::bundled(),
    };
    let mode = match val("--mode").as_deref().unwrap_or("text") {
        "text" => Structured::Text,
        "prompted" => Structured::Prompted,
        "native" => Structured::Native,
        o => return Err(format!("--mode {o:?}")),
    };
    let reps: usize = val("--reps").and_then(|v| v.parse().ok()).unwrap_or(1);
    let gw = LlmGateway::from_env(&|k| std::env::var(k).ok())?;
    if !gw.is_enabled() {
        return Err("gateway not enabled".into());
    }
    let (findings, _) = Finding::from_report(&report, source)?;
    let emit = |f: &Finding, role: &str, rep: usize, req: &ModelRequest, mode: Structured, gw: &LlmGateway| -> Option<Value> {
        let r = gw.send(req, mode);
        let usage = gw.last_usage();
        let mut o = json!({"finding": f.id, "metric": f.metric, "role": role, "rep": rep, "model": gw.model_id(), "mode": mode.as_str(),
                           "tokens_in": usage.as_ref().map(|u| u.tokens_in), "tokens_out": usage.as_ref().map(|u| u.tokens_out), "cost_usd": usage.as_ref().map(|u| u.cost_usd.clone()), "latency_ms": usage.as_ref().map(|u| u.latency_ms)});
        let parsed = match r {
            Err(e) => {
                o["gateway_error"] = json!(format!("{e:?}"));
                None
            }
            Ok(s) => {
                let (raw, v) = match &s.output {
                    Value::String(t) => (t.clone(), classify(t)),
                    other => (other.to_string(), ("gateway_parsed", Some(other.clone()))),
                };
                o["raw"] = json!(raw);
                o["packaging"] = json!(v.0);
                v.1
            }
        };
        println!("{}", o);
        parsed
    };
    for f in findings.iter().filter(|f| f.direction == "up") {
        let Some(row) = map_finding(f) else { continue };
        let dc = if f.source == Source::Synthetic { DataClass::Synthetic } else { DataClass::Treated };
        for rep in 0..reps {
            let sreq = roles::scout_request(f, &row, dc);
            let sv = emit(f, "scout", rep, &sreq, mode, &gw);
            let mut opp = sv.as_ref().and_then(|v| roles::parse_scout(&row, v).ok());
            if let Some(v) = &sv {
                if let Err(e) = roles::parse_scout(&row, v) {
                    println!("{}", json!({"finding": f.id, "role": "scout", "rep": rep, "validation_error": e}));
                }
            }
            if opp.is_none() {
                let t = &row.targets[0];
                opp = Some(Opportunity { id: "h_1".into(), target_ref: t.target_ref.clone(), mechanism_class: t.mechanisms[0].into(), hypothesis: "stand-in".into(), claimed_rate: round2(f.discovery.rate), falsifiers: vec![], alternatives: vec![] });
            }
            let opp = opp.expect("set above");
            match roles::builder_request(f, &opp, &row, &catalog, dc) {
                Err(e) => println!("{}", json!({"finding": f.id, "role": "builder", "rep": rep, "request_error": e})),
                Ok(breq) => {
                    let bv = emit(f, "builder", rep, &breq, mode, &gw);
                    if let Some(v) = bv {
                        match roles::parse_builder(&v) {
                            Err(e) => println!("{}", json!({"finding": f.id, "role": "builder", "rep": rep, "validation_error": e})),
                            Ok(p) => {
                                let c = reasoning::patch::compile(&catalog, f, &row, &opp, &p);
                                println!("{}", json!({"finding": f.id, "role": "builder", "rep": rep, "compile": match &c { Ok(c) => format!("ok:{}", c.kind), Err(d) => format!("denied:{}:{}", d.code, d.why) }}));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
