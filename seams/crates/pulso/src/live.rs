//! The live demo: the ten-step offline thread (thread10: engine executor + step stand-ins + labelled Core double) streams
//! every step, double and gate into a `RunEventSink` WHILE it runs. Labels are never invented here: a step's label is the
//! status thread10's report derives from what the job committed; between those moments a step is `pending` or `running`.
use crate::doubles::generate;
use debug_api::ingest::{double_item, gates_data, step_node};
use debug_api::panels::project;
use debug_api::store::now_iso;
use debug_api::{NewEvent, RunEventSink};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// The report steps in order (the split steps 3 and 4 appear twice in the ten-step list: scout/recompute, opportunity/validation).
pub const STEP_IDS: [&str; 12] = ["trigger", "signals", "scout", "recompute", "opportunity", "validation", "compile", "gate", "revision", "approval", "publish", "observation"];
/// `revision` (V3r) is a library hook not wired into the job: it is never marked `running`.
const NEVER_RUNNING: [&str; 1] = ["revision"];

pub struct DemoOpts {
    pub run_id: String,
    pub work: PathBuf,
    pub runner: PathBuf,
    pub sha: String,
    pub pace: Duration,
    pub human_override: bool,
    pub denied_kind: bool,
}

impl DemoOpts {
    pub fn new(run_id: String, work: PathBuf, runner: PathBuf) -> DemoOpts {
        DemoOpts { run_id, work, runner, sha: "0".repeat(40), pace: Duration::ZERO, human_override: true, denied_kind: false }
    }
}

struct Live {
    sink: Arc<dyn RunEventSink>,
    run: String,
    pace: Duration,
    labels: HashMap<String, String>,
    declared: HashSet<String>,
    /// Last panel snapshot sent per event kind + entity, with its timestamps blanked, so an unchanged panel is not re-sent.
    panels: HashMap<String, Value>,
    error: Option<String>,
}

impl Live {
    fn emit(&mut self, kind: &str, ek: &str, eid: &str, data: Value) {
        if self.error.is_none() {
            if let Err(e) = self.sink.emit(&self.run, NewEvent::new(kind, ek, eid, data)) {
                self.error = Some(e);
            }
        }
    }

    fn node(&mut self, id: &str, status: &str) {
        let idx = STEP_IDS.iter().position(|s| *s == id).unwrap_or(0);
        let prev = idx.checked_sub(1).map(|i| STEP_IDS[i]);
        let node = step_node(id, status, prev);
        self.labels.insert(id.to_string(), node["label"].as_str().unwrap_or_default().to_string());
        self.emit("node_status_changed", "node", id, json!({"node": node}));
    }

    fn doubles(&mut self, report: &Value, include_not_exercised: bool) {
        let mut fresh = Vec::new();
        for d in generate(report) {
            if !include_not_exercised && d["status"] == "not_exercised" {
                continue;
            }
            if let Some(item) = double_item(&d) {
                if self.declared.insert(item["id"].as_str().unwrap_or_default().to_string()) {
                    fresh.push(item);
                }
            }
        }
        if !fresh.is_empty() {
            let run = self.run.clone();
            self.emit("doubles_declared", "run", &run, json!({"doubles": fresh}));
        }
    }

    /// Emit the panel events (investigation, alternatives, diff, gates, decision) of what is committed so far. Each is a full
    /// snapshot derived by the shared `debug_api::panels::project`; one is sent only when it changed since the last send.
    fn panels(&mut self, committed: &Value, report: Option<&Value>) {
        let blank = project(committed, report, "");
        let stamped = project(committed, report, &now_iso());
        for (b, e) in blank.into_iter().zip(stamped) {
            let key = format!("{}|{}", b.kind, b.entity_id);
            if self.panels.get(&key) != Some(&b.data) {
                self.panels.insert(key, b.data);
                if self.error.is_none() {
                    if let Err(err) = self.sink.emit(&self.run, e) {
                        self.error = Some(err);
                    }
                }
            }
        }
    }

    /// Complete every step the partial report says has run, then mark the next one `running`.
    fn advance(&mut self, partial: &Value) {
        let steps = partial["steps"].as_array().cloned().unwrap_or_default();
        for st in &steps {
            let (id, status) = (st["id"].as_str().unwrap_or_default(), st["status"].as_str().unwrap_or("unknown"));
            if status != "not_exercised" && self.labels.get(id).is_none_or(|l| l.ends_with("[pending]") || l.ends_with("[running]")) {
                self.node(id, status);
                std::thread::sleep(self.pace);
            }
        }
        let next = STEP_IDS.iter().find(|id| self.labels.get(**id).is_some_and(|l| l.ends_with("[pending]")) && !NEVER_RUNNING.contains(id));
        if let Some(id) = next.map(|s| s.to_string()) {
            self.node(&id, "running");
        }
        self.doubles(partial, false);
    }
}

pub fn demo(sink: Arc<dyn RunEventSink>, o: &DemoOpts) -> Result<thread10::Run, String> {
    let live = Rc::new(RefCell::new(Live { sink, run: o.run_id.clone(), pace: o.pace, labels: HashMap::new(), declared: HashSet::new(), panels: HashMap::new(), error: None }));
    let empty = thread10::report::build(&thread10::report::Input { sha: &o.sha, payload: None, events: &[], error: None });
    {
        let mut l = live.borrow_mut();
        let title = "DEMO-0 live run (host rust, target local): offline Core double, no real model, simulated human, no quality claim";
        let run = l.run.clone();
        l.emit("run_started", "run", &run, json!({"title": title, "state": "running", "origin": "manual"}));
        l.doubles(&empty, false);
        for id in STEP_IDS {
            l.node(id, "pending");
        }
        std::thread::sleep(o.pace);
    }
    let hook = live.clone();
    let mut t = thread10::Opts::new(o.work.clone(), o.runner.clone());
    t.human_override = o.human_override;
    t.denied_kind = o.denied_kind;
    t.sha = o.sha.clone();
    t.on_commit = Some(Rc::new(move |_, partial| hook.borrow_mut().advance(partial)));
    let panel_hook = live.clone();
    t.on_payload = Some(Rc::new(move |_, committed| panel_hook.borrow_mut().panels(committed, None)));
    let result = thread10::run(&t);
    let mut l = live.borrow_mut();
    let run = l.run.clone();
    let state = match &result {
        Ok(r) if r.error.is_none() => "completed",
        _ => "failed",
    };
    if let Ok(r) = &result {
        for st in r.report["steps"].as_array().into_iter().flatten() {
            let (id, status) = (st["id"].as_str().unwrap_or_default(), st["status"].as_str().unwrap_or("unknown"));
            if l.labels.get(id).map(String::as_str) != Some(format!("{id} [{status}]").as_str()) {
                l.node(id, status);
            }
        }
        l.doubles(&r.report, true);
        // The final snapshot knows the end of the run (e.g. an approval that was blocked); gates not derivable from the committed
        // payload fall back to the report's own verdict.
        if let Some(p) = &r.payload {
            l.panels(p, Some(&r.report));
        }
        if !l.panels.keys().any(|k| k.starts_with("gates_set|")) {
            l.emit("gates_set", "run", &run, gates_data(&r.report));
        }
    }
    l.emit("run_state_changed", "run", &run, json!({"state": state}));
    if let Some(e) = l.error.take() {
        return Err(format!("streaming into the debug-api failed: {e}"));
    }
    result
}
