//! LIVE tests against the local stack (llm-gateway on 127.0.0.1:8080, model deepseek/deepseek-v4.1-flash). Opt-in: every test is
//! `#[ignore]`d; run with
//!
//! ```text
//! PULSO_LLM_GATEWAY=enabled PULSO_LLM_GATEWAY_ADDR=127.0.0.1:8080 PULSO_LLM_GATEWAY_KEY=<consumer token> \
//!   cargo test -j 1 -p reasoning --test live -- --ignored --nocapture
//! ```
//!
//! SYNTHETIC aggregates only (invented numbers), so no opt-in flag is needed; the derived-data opt-in is covered offline. The key
//! comes from the environment and is never printed. A real model is not deterministic, so each scenario is tried up to three
//! times (separately recorded, no prompt change between attempts) and the test requires at least one compiled proposal; the
//! outcome of every attempt is printed as a reason code, never as free text.
mod common;
use common::*;
use reasoning::catalog::Catalog;
use reasoning::finding::{Finding, Source};
use reasoning::pipeline::{Opts, Ports, reason};
use serde_json::{Value, json};

fn live_ports() -> Ports {
    reasoning::live::ports_from_env(&|k| std::env::var(k).ok()).expect("live env (see the module docs)")
}

fn attempt(f: &Finding, expect_kind: &str) {
    let catalog = Catalog::bundled();
    let mut outcomes: Vec<String> = vec![];
    let mut proposed: Option<reasoning::pipeline::Reasoned> = None;
    for _ in 0..3 {
        let r = reason(&catalog, f, &live_ports(), &Opts::default());
        outcomes.push(format!("{}:{}:{}", r.status, r.reason, r.stage));
        if r.status == "proposed" {
            proposed = Some(r);
            break;
        }
        if r.status == "blocked" && r.stage != "compile" && r.stage != "scout" && r.stage != "builder" {
            // a transport or policy failure is not a model-quality outcome: surface it
            assert!(!["model_unavailable", "model_refused"].contains(&r.reason.as_str()), "the live stack is not reachable or refused the payload: {outcomes:?} {}", r.detail);
        }
    }
    println!("{} outcomes over attempts: {outcomes:?}", f.id);
    let r = proposed.unwrap_or_else(|| panic!("no attempt produced a compiled proposal: {outcomes:?}"));
    assert!(r.calls.iter().all(|c| c["real"] == true), "every call was answered by the real gateway");
    assert!(r.doubles.is_empty(), "a live run has no doubles");
    let p = r.compiled.as_ref().unwrap();
    assert_eq!(p["kind"], expect_kind);
    let rub = r.rubric.as_ref().unwrap();
    println!("rubric {} total {} band {} models {:?}", f.id, rub["total"], rub["band"], r.calls.iter().map(|c| c["model_id"].clone()).collect::<Vec<Value>>());
    println!("proposal {}: {}", f.id, json!({"target": p["target_ref"], "diff_entries": p["diff"].as_array().map(Vec::len), "cascade": p["cascade"], "edit_chars": p["edit_chars"]}));
    for hg in ["R4", "R5", "R7", "R11"] {
        assert_eq!(rub["hard_gates"][hg], true, "hard gate {hg}");
    }
}

#[test]
#[ignore = "live: needs the local llm-gateway and PULSO_LLM_GATEWAY_KEY"]
fn live_uncovered_reason_becomes_a_new_agent_proposal() {
    attempt(&tecnico_finding(), "new_agent");
}

#[test]
#[ignore = "live: needs the local llm-gateway and PULSO_LLM_GATEWAY_KEY"]
fn live_status_inquiry_gap_becomes_a_template_patch_of_t_estado_pqr() {
    attempt(&pqr_finding(), "patch");
}

#[test]
#[ignore = "live: needs the local llm-gateway and PULSO_LLM_GATEWAY_KEY"]
fn live_repeated_lookup_becomes_a_prompt_patch_of_p_copiloto() {
    attempt(&copilot_finding(Source::Synthetic), "patch");
}
