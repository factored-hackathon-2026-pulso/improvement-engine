//! Synthetic data for the `thread` job: package dir, lab file and the job spec. No real E0 rows.
//! The sensor runner is `synth_runner` (a stand-in binary) passed in by the caller.
use crate::adapters::StepEnv;
use std::path::Path;

const COMPILE_IN: &str = r#"{"base_bundle_ref":"bundle:attention-task@1","change_spec":{"affected_routes":["disputa-tarea"],"base_bundle_ref":"bundle:attention-task@1","expected_mechanism":"shorter closing reply","operations":[{"new_ref":"prompt:resumen_radicado@2","op":"replace","precondition_digest":"sha256:727341c7f278859de2b2a8e997305012d9094e02ac60d4af7a722101fa9eb089","target_kind":"prompt","target_ref":"prompt:resumen_radicado@1"},{"new_ref":"eval_suite:disputas-tarea-suite@2","op":"add","precondition_digest":"sha256:4136704fa37d584b4de65be3a56b725b41f3bbc95f59f95b6273649994d4e216","target_kind":"eval_suite","target_ref":"eval_suite:disputas-tarea-suite@1"}],"opportunity_ref":"opportunity:o1@1","rollback_ref":"bundle:attention-task@1","workflow_bridge_ref":"bridge:disputa-tarea@1"},"contract_version":"engine-steps/0","data_class":"synthetic","run_id":"run-thread01-0001","step":"compile"}"#;

const GATE_IN: &str = r#"{"gate_in":{"contract_version":"engine-steps/0","step":"gate","run_id":"run-thread01-0001","data_class":"synthetic","base_arm_report_ref":"arm_report:base@1","candidate_arm_report_ref":"arm_report:cand@1","suite_ref":"eval_suite:disputas-tarea-suite@1","judge_actor":"claude-gsipy","author_actors":["claude-wrld0"]},"reports":{"arm_report:base@1":{"runs":[{"arm":"x","case_ref":"c1","status":"failed","closed_early":false,"cost_known":true,"oracle_ref":"oracle:o@1","final_state_ref":"state:s@1","effect_receipts":[],"reason":null},{"arm":"x","case_ref":"c2","status":"completed","closed_early":false,"cost_known":true,"oracle_ref":"oracle:o@1","final_state_ref":"state:s@1","effect_receipts":[],"reason":null}]},"arm_report:cand@1":{"runs":[{"arm":"x","case_ref":"c1","status":"completed","closed_early":false,"cost_known":true,"oracle_ref":"oracle:o@1","final_state_ref":"state:s@1","effect_receipts":[],"reason":null},{"arm":"x","case_ref":"c2","status":"completed","closed_early":false,"cost_known":true,"oracle_ref":"oracle:o@1","final_state_ref":"state:s@1","effect_receipts":[],"reason":null}]}},"world_authors":{"world":"claude-wrld0","suite":"agent-core-registry-demo@c814c2b"}}"#;

/// Builds `<work>/snapshots/sample-1`, `<work>/lab/sample-1.json`, `<work>/recompute/` and returns the step env plus
/// the initial job payload. The lab holds 120/400 = 0.30; `claimed_rate` (default 0.3) is what the scout claims.
/// The lab row of one signal: what the verifier recompute reads (aggregate only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabRow {
    pub evidence_ref: String,
    pub numerator: u64,
    pub count: u64,
}

impl LabRow {
    /// The lab value of the default thread: 120/400 = 0.30.
    pub fn default_row() -> LabRow {
        LabRow { evidence_ref: "ev-0001".into(), numerator: 120, count: 400 }
    }
}

pub fn build(work: &Path, runner_exe: &Path, claimed_rate: Option<f64>) -> Result<(StepEnv, String), String> {
    build_row(work, runner_exe, &LabRow::default_row(), claimed_rate)
}

/// As `build`, with the lab row of the signal under test. The stand-in sensor names its only signal `sig-0001`, so the
/// row is always filed under that thread-local id; the caller maps it to its own signal id.
pub fn build_row(work: &Path, runner_exe: &Path, row: &LabRow, claimed_rate: Option<f64>) -> Result<(StepEnv, String), String> {
    let io = |e: std::io::Error| e.to_string();
    if row.evidence_ref.is_empty() || !row.evidence_ref.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(format!("lab evidence_ref {:?} is not an opaque id", row.evidence_ref));
    }
    if row.count == 0 || row.numerator > row.count {
        return Err("lab row needs 0 <= numerator <= count and count >= 1".into());
    }
    let (snap, lab, rec) = (work.join("snapshots"), work.join("lab"), work.join("recompute"));
    std::fs::create_dir_all(snap.join("sample-1")).map_err(io)?;
    std::fs::create_dir_all(&lab).map_err(io)?;
    std::fs::create_dir_all(&rec).map_err(io)?;
    std::fs::write(snap.join("sample-1").join("README.txt"), "synthetic package placeholder\n").map_err(io)?;
    std::fs::write(
        lab.join("sample-1.json"),
        format!(r#"{{"rows":[{{"signal_id":"sig-0001","evidence_ref":"{}","numerator":{},"count":{}}}]}}"#, row.evidence_ref, row.numerator, row.count),
    )
    .map_err(io)?;
    let head = |step: &str| format!(r#""contract_version":"engine-steps/0","step":"{step}","run_id":"run-thread01-0001","data_class":"synthetic""#);
    let spec = format!(
        concat!(
            r#"{{"spec":{{"sensors":{{{h1},"source_snapshot_ref":"source_snapshot:sample-1@1","discovery_config_ref":"discovery_config:sample-1@1","#,
            r#""window":{{"start":"2026-04-18","end":"2026-06-17"}},"metric_spec_refs":["metric_spec:sample-1@1"]}},"#,
            r#""recompute":{{{h2},"lab_ref":"lab:sample-1@1","signal_ids":["sig-0001"],"scout_claims":[{{"signal_id":"sig-0001","claimed_rate":{rate}}}]}},"#,
            r#""validation":{{{h3},"signal_id":"sig-0001","recompute_ref":"recompute:sample-1@1","checks":["denominator","temporality"],"#,
            r#""scout_actor":"actor-scout","verifier_actor":"actor-verifier"}},"#,
            r#""compile":{compile},"gate":{gate}}},"out":{{}}}}"#
        ),
        h1 = head("sensors"),
        h2 = head("recompute"),
        h3 = head("validation"),
        rate = claimed_rate.unwrap_or(0.3),
        compile = COMPILE_IN,
        gate = GATE_IN,
    );
    let env = StepEnv {
        runner_exe: runner_exe.to_path_buf(),
        snapshot_root: snap,
        lab_dir: lab,
        recompute_dir: rec,
        arranque: 30,
        min_support: 5,
    };
    Ok((env, spec))
}
