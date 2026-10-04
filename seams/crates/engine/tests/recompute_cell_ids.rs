//! W7: the recompute handler accepts the cell metric id the real `rust-events` sensor emits (`reassignment_rate.pt.web_chat`)
//! as a signal id, in addition to the sensor's own `sig-NNNN` ids. Additive: the old behaviour is unchanged.
use abi::{Fence, HandlerError, InputEnvelope, JobHandler};
use engine::adapters::{StepEnv, thread_handlers};
use serde_json::{Value, json};
use std::path::PathBuf;

const CELL: &str = "reassignment_rate.pt.web_chat";

fn env(tag: &str, row_id: &str) -> StepEnv {
    let d = std::env::temp_dir().join(format!("recomp-cell-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lab")).unwrap();
    std::fs::write(d.join("lab").join("sample-1.json"), json!({"rows": [{"signal_id": row_id, "evidence_ref": "ev-0001", "numerator": 54, "count": 321}]}).to_string()).unwrap();
    StepEnv { runner_exe: PathBuf::from("unused"), snapshot_root: d.join("snap"), lab_dir: d.join("lab"), recompute_dir: d.join("rec"), arranque: 30, min_support: 5 }
}

fn head(step: &str) -> Value {
    json!({"contract_version": "engine-steps/0", "step": step, "run_id": "run-cell-0001", "data_class": "treated"})
}

fn payload(signal_ids: Value, claim_id: &str) -> String {
    let mut rec = head("recompute");
    rec["lab_ref"] = json!("lab:sample-1@1");
    rec["signal_ids"] = signal_ids;
    rec["scout_claims"] = json!([{"signal_id": claim_id, "claimed_rate": 0.17}]);
    let mut val = head("validation");
    val["recompute_ref"] = json!("recompute:sample-1@1");
    let sensed = json!({"signals": [{"signal_id": "sig-0001", "metric_id": CELL, "population": "pt/web_chat", "numerator": 54, "denominator": 321, "holdout_checked": true, "evidence_ref": "ev-0001"}], "discards": []});
    json!({"spec": {"recompute": rec, "validation": val}, "out": {"sensors": sensed}}).to_string()
}

fn run_recompute(tag: &str, row_id: &str, p: String) -> Result<Value, HandlerError> {
    let hs = thread_handlers(env(tag, row_id), None);
    let recompute = hs.iter().find(|h| h.id().0 == "recompute").unwrap();
    let fence = Fence { worker_id: "w".into(), fence_token: 1, attempt: 1 };
    let out = recompute.run(&fence, &InputEnvelope { job_id: "j".into(), step_index: 1, payload: p })?;
    Ok(serde_json::from_str(&out.payload).unwrap())
}

#[test]
fn a_cell_metric_id_is_accepted_as_the_signal_id_and_recomputed_from_the_lab_row() {
    let out = run_recompute("ok", CELL, payload(json!([CELL]), CELL)).expect("the sensed cell metric id must be accepted");
    let row = &out["out"]["recompute"]["recomputes"][0];
    assert_eq!(row["signal_id"], CELL);
    assert_eq!(row["recomputed_rate"], 0.17);
    assert_eq!(row["match"], true);
}

#[test]
fn the_sensors_own_signal_id_still_works_and_an_unsensed_id_is_still_refused() {
    let out = run_recompute("old", "sig-0001", payload(json!(["sig-0001"]), "sig-0001")).expect("sig-NNNN ids keep working");
    assert_eq!(out["out"]["recompute"]["recomputes"][0]["signal_id"], "sig-0001");
    // neither a sensed signal_id nor a sensed metric_id
    let other = "reassignment_rate.es.app_chat";
    let err = run_recompute("bad", other, payload(json!([other]), other)).unwrap_err();
    assert!(format!("{err:?}").contains("not produced by the sensor step"), "{err:?}");
}
