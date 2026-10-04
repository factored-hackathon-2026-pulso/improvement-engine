//! TEMPORARY exploration of the evaluate outcomes on a live stack (replaced by live_capture.rs).
#[path = "../../core-client/tests/live_common/mod.rs"]
mod live_common;
use core_client::authoring::{Alias, DryRunRequest};
use core_client::evaluate::EvaluationRun;
use live_common::*;

fn er(tg: &str, attempt: u32, run_release: &str) -> EvaluationRun {
    EvaluationRun {
        tenant_id: tenant(),
        job_id: format!("job-evalonly-{tg}"),
        logical_key: "evalonly".into(),
        attempt,
        pulso_run_ref: format!("pr-job-evalonly-{tg}"),
        lab_grant_ref: "grant-contract".into(),
        writer_release_id: run_release.into(),
        writer_agent_version: "1.0.0".into(),
        budget_ref: "bud-e2e".into(),
        deadline: deadline(1),
    }
}

#[test]
#[ignore = "live exploration"]
fn explore() {
    let (c, fx, ten) = (client("v1-explore"), Fx::from_env(), tenant());
    let base = c.read_alias(&ten, None, &agent(), Alias::Staging).unwrap().release_id.unwrap();
    let which = std::env::var("EXPLORE").unwrap_or_default();
    let scripted = which != "infra" && which != "quota";
    if scripted {
        fx.script_closing_reply().unwrap();
    } else {
        fx.config(&serde_json::json!({"llm_replace": true, "llm_rules": []})).unwrap();
    }
    let var = variant(if which == "fail" { "fail_suite" } else { "accept_alt" });
    let tg = tag("v1x");
    let plan = if which == "quota" { plan(&var, &tg).with_origin("auto_detect") } else { plan(&var, &tg) };
    let run = writer_run(&tg);
    let fz = c.freeze_draft(&fx, &plan, &run, Some(&base)).expect("freeze");
    let n = if which == "quota" { 23 } else { 1 };
    for attempt in 1..=n {
        let r = c.evaluate_frozen(&fx, &fz, &var.suite, &er(&tg, attempt, &run.writer_release_id));
        match r {
            Ok(e) => eprintln!("ATTEMPT {attempt} OK verdict={:?} native={:?} state={:?}", e.verdict(), e.native, e.receipt.state),
            Err(e) => eprintln!("ATTEMPT {attempt} ERR {e}"),
        }
    }
    let _ = DryRunRequest::new;
}
