//! Engine-run report (contracts/engine-run, C-2) -> run events, plus the contract seed.
use crate::event::RunEventSink;
use serde_json::Value;

/// Turns an engine-run report into a finished run in `sink`; returns the run id. Refuses a report that claims quality
/// or that was already ingested.
pub fn engine_run_report(sink: &dyn RunEventSink, exists: &dyn Fn(&str) -> bool, report: &Value) -> Result<String, String> {
    let _ = (sink, exists, report);
    todo!()
}

/// The data the console contract suite reads unconditionally (decision `dec-1`, proposal `prop-1`), under a run that says
/// plainly it is not an engine run.
pub fn contract_seed(sink: &dyn RunEventSink) -> Result<String, String> {
    let _ = sink;
    todo!()
}
