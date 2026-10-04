//! E2 adapters: the `steps` stand-ins (sensor, recompute, intent/validation, compile, gate) behind the abi.
//! One `JobHandler` per step; a `thread` job is `thread_handlers(..)` run by the executor.
//!
//! Payload (single-line JSON): `{"spec":{<step key>: <step input doc>...},"out":{<step key>: <step output>...}}`.
//! Each handler reads `spec[<key>]`, runs the step, and appends `out[<key>]`. Events are
//! `thread:<key>:<summary>:semantics=claude-standin` (deterministic, no timestamps).
//!
//! Env contract (as steps_host.py): STEPS_RUNNER_EXE, STEPS_SNAPSHOT_ROOT, STEPS_ARRANQUE, STEPS_MIN_SUPPORT (sensor),
//! STEPS_LAB_DIR (recompute), STEPS_RECOMPUTE_DIR (validation). The steps read them from the process
//! environment, so calls are serialised by a lock and the variables are set per call.
use abi::*;
use std::path::PathBuf;
use std::sync::Mutex;
use steps::sensor::json::{self, Json};

/// Dry-run hook for compile: gets the canonical JSON of each operation, returns the plan digest of the real
/// Core dry-run (live wiring point; see `live`). `None` = local canonical digest.
pub type DryRun = Box<dyn Fn(&[String]) -> String>;

#[derive(Debug, Clone)]
pub struct StepEnv {
    pub runner_exe: PathBuf,
    pub snapshot_root: PathBuf,
    pub lab_dir: PathBuf,
    pub recompute_dir: PathBuf,
    pub arranque: u32,
    pub min_support: u32,
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Sensors,
    Recompute,
    Validation,
    Compile,
    Gate,
}

impl Step {
    fn key(self) -> &'static str {
        match self {
            Step::Sensors => "sensors",
            Step::Recompute => "recompute",
            Step::Validation => "validation",
            Step::Compile => "compile",
            Step::Gate => "gate",
        }
    }
}

pub struct StepHandler {
    step: Step,
    env: StepEnv,
    dry_run: Option<DryRun>,
}

/// The five handlers in pipeline order: signals -> recompute -> validation -> compile -> gate.
/// `dry_run` goes to the compile handler.
pub fn thread_handlers(env: StepEnv, dry_run: Option<DryRun>) -> Vec<Box<dyn JobHandler>> {
    let mut dry = dry_run;
    [Step::Sensors, Step::Recompute, Step::Validation, Step::Compile, Step::Gate]
        .into_iter()
        .map(|step| {
            let d = if step == Step::Compile { dry.take() } else { None };
            Box::new(StepHandler { step, env: env.clone(), dry_run: d }) as Box<dyn JobHandler>
        })
        .collect()
}

fn bad(m: impl Into<String>) -> HandlerError {
    HandlerError::Invalid(m.into())
}

fn step_err(e: steps::StepError) -> HandlerError {
    match e {
        steps::StepError::Invalid(m) => HandlerError::Invalid(m),
        other => HandlerError::Failed(other.to_string()),
    }
}

impl StepHandler {
    fn call(&self, input: &str) -> Result<String, HandlerError> {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let e = &self.env;
        let vars: [(&str, String); 6] = [
            ("STEPS_RUNNER_EXE", e.runner_exe.display().to_string()),
            ("STEPS_SNAPSHOT_ROOT", e.snapshot_root.display().to_string()),
            ("STEPS_LAB_DIR", e.lab_dir.display().to_string()),
            ("STEPS_RECOMPUTE_DIR", e.recompute_dir.display().to_string()),
            ("STEPS_ARRANQUE", e.arranque.to_string()),
            ("STEPS_MIN_SUPPORT", e.min_support.to_string()),
        ];
        for (k, v) in &vars {
            // SAFETY: every reader of STEPS_* is a step called below while ENV_LOCK is held; no other
            // thread of this crate reads or writes these variables.
            unsafe { std::env::set_var(k, v) };
        }
        match self.step {
            Step::Sensors => steps::sensor::run(input).map_err(step_err),
            Step::Recompute => steps::recompute::run(input).map_err(step_err),
            Step::Validation => steps::intent::run(input).map_err(step_err),
            Step::Compile => {
                let w = steps::compile::World::seeded_base();
                steps::compile::run_with(input, &w, self.dry_run.as_deref()).map_err(|e| bad(format!("compile: {}", e.0)))
            }
            Step::Gate => steps::gate::run(input).map_err(|e| bad(e.to_string())),
        }
    }
}

fn arr_len(v: Option<&Json>) -> usize {
    v.and_then(Json::as_arr).map_or(0, <[Json]>::len)
}

fn summary(step: Step, out: &Json) -> String {
    match step {
        Step::Sensors => format!("signals={},discards={}", arr_len(out.get("signals")), arr_len(out.get("discards"))),
        Step::Recompute => {
            let rows = out.get("recomputes").and_then(Json::as_arr).unwrap_or(&[]);
            let m = rows.iter().filter(|r| r.get("match") == Some(&Json::Bool(true))).count();
            format!("matches={m}/{}", rows.len())
        }
        Step::Validation => format!("verdict={}", out.get("verdict").and_then(Json::as_str).unwrap_or("?")),
        Step::Compile => format!("status={}", out.get("status").and_then(Json::as_str).unwrap_or("?")),
        Step::Gate => format!("verdict={}", out.get("verdict").and_then(Json::as_str).unwrap_or("?")),
    }
}

/// `recompute:<id>@<rev>` -> `<id>`
fn ref_name(r: &str) -> Option<&str> {
    let (_, rest) = r.split_once(':')?;
    let (id, _) = rest.split_once('@')?;
    (!id.is_empty()).then_some(id)
}

impl JobHandler for StepHandler {
    fn id(&self) -> HandlerId {
        HandlerId(self.step.key().to_string())
    }

    fn run(&self, _fence: &Fence, input: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        let key = self.step.key();
        let payload = json::parse(&input.payload).map_err(|e| bad(format!("payload: {e}")))?;
        let spec = payload.get("spec").and_then(|s| s.get(key)).ok_or_else(|| bad(format!("spec.{key} missing")))?;
        let out_so_far = match payload.get("out") {
            Some(Json::Obj(kv)) => kv.clone(),
            _ => vec![],
        };
        let prior = |k: &str| out_so_far.iter().find(|(n, _)| n == k).map(|(_, v)| v);

        match self.step {
            Step::Recompute => {
                let sensed: Vec<&str> = prior("sensors")
                    .and_then(|s| s.get("signals"))
                    .and_then(Json::as_arr)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|s| s.get("signal_id").and_then(Json::as_str))
                    .collect();
                for id in spec.get("signal_ids").and_then(Json::as_arr).unwrap_or(&[]) {
                    if !id.as_str().is_some_and(|i| sensed.contains(&i)) {
                        return Err(bad("recompute signal_id was not produced by the sensor step"));
                    }
                }
            }
            Step::Compile => {
                let verdict = prior("validation").and_then(|v| v.get("verdict")).and_then(Json::as_str).unwrap_or("missing");
                if verdict != "corroborated" {
                    return Err(HandlerError::Failed(format!("blocked: validation verdict {verdict}, compile not attempted")));
                }
            }
            _ => {}
        }

        let out_text = self.call(&spec.write())?;
        let out = json::parse(&out_text).map_err(|e| bad(format!("step output: {e}")))?;
        if self.step == Step::Recompute {
            // the validation step resolves its recompute_ref to a file holding this output
            let r = payload.get("spec").and_then(|s| s.get("validation")).and_then(|v| v.get("recompute_ref")).and_then(Json::as_str);
            let id = r.and_then(ref_name).ok_or_else(|| bad("validation.recompute_ref"))?;
            std::fs::create_dir_all(&self.env.recompute_dir).map_err(|e| HandlerError::Failed(e.to_string()))?;
            std::fs::write(self.env.recompute_dir.join(format!("{id}.json")), &out_text).map_err(|e| HandlerError::Failed(e.to_string()))?;
        }
        let event = format!("thread:{key}:{}:semantics={}", summary(self.step, &out), steps::SEMANTICS);
        let mut out_obj = out_so_far;
        out_obj.push((key.to_string(), out));
        let spec_all = payload.get("spec").cloned().unwrap_or(Json::Null);
        let next = Json::obj(vec![("spec", spec_all), ("out", Json::Obj(out_obj))]).write();
        Ok(OutputEnvelope { payload: next, events: vec![event], effect: EffectState::NoEffect })
    }
}
