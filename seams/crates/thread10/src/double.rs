//! The OFFLINE Core double behind `engine::live::CorePort`. It is NOT the Core: every answer is derived locally and the
//! report labels it (`ports[].provenance = offline-double`). The arms complete identically on both sides, so the
//! structural gate reports no improvement (the same finding as the live windows); the double never encodes an
//! "improvement" (that would be circular, authored by us).
use core_client::canon::sha256_hex;
use core_client::dto::ArmReport;
use engine::live::{ArmCall, CorePort, FrozenInfo, PublishInfo, SuiteInfo};
use serde_json::json;

pub const BASE_RELEASE: &str = "rel-base-double";
pub const ACTOR: &str = "local-supervisor";

/// `ledger`: every `publish` invocation appends `key release_id` (the observable side effect of the double).
/// `kill_in_publish`: after the effect (ledger line) create the file and block, so the test kills the process
/// between the effect and its commit.
#[derive(Default)]
pub struct DoublePort {
    pub ledger: Option<std::path::PathBuf>,
    pub kill_in_publish: Option<std::path::PathBuf>,
}

fn hash_of(ops: &[String]) -> String {
    sha256_hex(ops.join("\n").as_bytes())
}

impl CorePort for DoublePort {
    fn dry_run(&self, ops: &[String]) -> Result<String, String> {
        Ok(format!("sha256:{}", hash_of(ops)))
    }
    fn freeze(&self, ops: &[String], job_id: &str) -> Result<FrozenInfo, String> {
        let h = hash_of(ops);
        Ok(FrozenInfo {
            proposal_id: format!("prop-double-{job_id}"),
            candidate_hash: h,
            base_release_id: BASE_RELEASE.into(),
            task_binding_ref: "bind-double".into(),
            plan_ref: "plan-double".into(),
            title: format!("pulso-key:{job_id}"),
            rev: 1,
            version: "2.0.0".into(),
        })
    }
    fn suite(&self, _f: &FrozenInfo) -> Result<SuiteInfo, String> {
        Ok(SuiteInfo { digest: sha256_hex(b"thread10-double-suite"), cases: vec!["c1".into(), "c2".into()] })
    }
    fn run_arm(&self, _f: &FrozenInfo, call: &ArmCall) -> Result<ArmReport, String> {
        let eid = format!("arm-{}", &sha256_hex(call.key.as_bytes())[..32]);
        ArmReport::from_json(&json!({"execution_id": eid, "status": "completed", "case_ref": call.case_ref, "closed_early": false, "cost_known": true})).map_err(|e| format!("{e:?}"))
    }
    fn evaluate(&self, _f: &FrozenInfo, _job: &str) -> Result<String, String> {
        Ok("pass".into())
    }
    fn approve(&self, _f: &FrozenInfo) -> Result<String, String> {
        Ok(ACTOR.into())
    }
    fn publish(&self, f: &FrozenInfo, key: &str) -> Result<PublishInfo, String> {
        let id = format!("rel-{}", &f.candidate_hash[..16]);
        if let Some(l) = &self.ledger {
            use std::io::Write;
            let mut fh = std::fs::OpenOptions::new().create(true).append(true).open(l).map_err(|e| e.to_string())?;
            writeln!(fh, "{key} {id}").map_err(|e| e.to_string())?;
        }
        if let Some(m) = &self.kill_in_publish {
            std::fs::write(m, "x").map_err(|e| e.to_string())?;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
        Ok(PublishInfo { release_id: id.clone(), staging_release_id: id })
    }
}
