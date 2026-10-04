//! R1E pipeline: a run over N signals. Each signal becomes one proposal (the ten-step thread over its lab row, scout/verifier/
//! builder through the `ModelPort`, the Core through the `CorePort`), and each proposal ends in a viability verdict recorded in
//! the ledger (`engine::ledger`, persisted through the engine job store) and streamed as a `proposal_verdict` run event.
//!
//! The verdict is derived from what the job COMMITTED, never from what was configured:
//! - a model stage that stopped the job: `evidence_insufficient`, `model_refused|unavailable|invalid` (not_evaluable),
//!   `verifier_refuted` (not_viable);
//! - the recompute did not corroborate the claim: `claim_not_corroborated` (not_viable);
//! - compile denied the change: the BK0 reason (`kind_not_supported`, `release_settings_not_allowed`) or `precondition_missing` /
//!   `compile_denied` (not_evaluable); BK0 and the compile step must agree, else the pipeline fails loudly;
//! - the structural gate: `fail` is `gate_failed`, `not_evaluable` is `gate_not_evaluable`, a missing gate after a compiled change
//!   is `core_unavailable`; a gate `pass` followed by a failed native evaluation is `native_eval_failed`;
//! - `viable` only when the gate passed on its own and the native evaluation passed. An override (a labelled SIMULATED human
//!   decision on the approval step) is kept on the trail but can never turn a failed gate into `viable`.
use crate::{JOB, Opts, Run, SignalSeed, run};
use debug_api::{NewEvent, RunEventSink};
use engine::ledger::{GateResult, Ledger, LedgerEntry, Verdict, bk0_check};
use engine::live::CorePort;
use engine::models::ModelPort;
use engine::{FileStore, JobStore};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

pub struct PipelineOpts {
    pub work: PathBuf,
    pub runner: PathBuf,
    pub run_id: String,
    pub signals: Vec<SignalSeed>,
    /// A complete labelled SIMULATED human override of a failed gate (DEMO-0); never makes a proposal viable.
    pub human_override: bool,
    pub model: Option<Rc<dyn ModelPort>>,
    pub core: Option<Rc<dyn CorePort>>,
    pub sink: Option<Arc<dyn RunEventSink>>,
    /// Where the ledger lives. `None` = an engine `FileStore` under `<work>/ledger`; pass a `PgJobStore` for Postgres.
    pub store: Option<Rc<dyn JobStore>>,
    pub sha: String,
    pub now: u64,
    /// Called after each committed handler of each signal's thread with `(signal index, handler index, committed {spec,out} payload)`.
    /// A read-only hook (a console projection); it never alters the run.
    pub on_payload: Option<PayloadHook>,
}

/// `(signal index, handler index, committed payload)`.
pub type PayloadHook = Rc<dyn Fn(usize, usize, &Value)>;

impl PipelineOpts {
    pub fn new(work: PathBuf, runner: PathBuf, run_id: &str, signals: Vec<SignalSeed>) -> PipelineOpts {
        PipelineOpts { work, runner, run_id: run_id.into(), signals, human_override: false, model: None, core: None, sink: None, store: None, sha: "0".repeat(40), now: 1000, on_payload: None }
    }
}

pub struct PipelineRun {
    pub entries: Vec<LedgerEntry>,
    pub runs: Vec<Run>,
    pub report: Value,
    /// Sink failures (the ledger entry is stored regardless); empty when every new verdict reached the sink.
    pub emit_errors: Vec<String>,
}

fn safe_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub fn run_signals(o: &PipelineOpts) -> Result<PipelineRun, String> {
    if !safe_id(&o.run_id) {
        return Err(format!("run_id {:?} is not an opaque id", o.run_id));
    }
    let mut seen = std::collections::BTreeSet::new();
    for s in &o.signals {
        if !safe_id(&s.signal_id) || !seen.insert(s.signal_id.as_str()) {
            return Err(format!("signal id {:?} is empty, not an opaque id or repeated", s.signal_id));
        }
    }
    std::fs::create_dir_all(&o.work).map_err(|e| e.to_string())?;
    let store: Rc<dyn JobStore> = match &o.store {
        Some(s) => s.clone(),
        None => Rc::new(FileStore::open(o.work.join("ledger"))?),
    };
    let ledger = Ledger::new(&*store);
    let (mut entries, mut runs, mut emit_errors, mut proposals) = (vec![], vec![], vec![], vec![]);
    for (i, seed) in o.signals.iter().enumerate() {
        let mut t = Opts::new(o.work.join(format!("p{i:04}")), o.runner.clone());
        t.human_override = o.human_override;
        t.sha = o.sha.clone();
        t.now = o.now;
        t.model = o.model.clone();
        t.core = o.core.clone();
        t.seed = seed.clone();
        if let Some(hook) = o.on_payload.clone() {
            t.on_payload = Some(Rc::new(move |handler, payload| hook(i, handler, payload)));
        }
        t.job = JOB.into();
        let r = run(&t)?;
        let ordinal = u32::try_from(i).map_err(|_| "too many signals".to_string())?;
        let e = verdict_of(ordinal, &o.run_id, seed, &r)?;
        ledger.record(&e)?;
        // The event is emitted once per verdict, also across a crash between the ledger write and the emit: a delivery marker is
        // written only after the sink accepted it, so a resume re-emits exactly the verdicts whose event never got out.
        let marker = format!("ledger_emitted/e{ordinal:04}");
        if let Some(sink) = &o.sink
            && store.get(&marker)?.is_none()
        {
            match sink.emit(&o.run_id, NewEvent::new("proposal_verdict", "proposal", &e.proposal_id, e.to_json())) {
                Ok(_) => {
                    let _ = store.cas(&marker, 0, "emitted");
                }
                Err(err) => emit_errors.push(format!("{}: {err}", e.proposal_id)),
            }
        }
        proposals.push(json!({"proposal_id": e.proposal_id, "signal_id": e.signal_id, "verdict": e.verdict.as_str(), "reason": e.reason, "stage": e.stage, "report": r.report}));
        entries.push(e);
        runs.push(r);
    }
    let count = |v: Verdict| entries.iter().filter(|e| e.verdict == v).count();
    let report = json!({
        "contract_revision": "pipeline-run/r1e-0", "label": "DEMO-0", "quality_claims": "forbidden", "run_id": o.run_id,
        "summary": {"signals": o.signals.len(), "proposals": entries.len(), "viable": count(Verdict::Viable), "not_viable": count(Verdict::NotViable), "not_evaluable": count(Verdict::NotEvaluable)},
        "proposals": proposals,
        "ledger": entries.iter().map(LedgerEntry::to_json).collect::<Vec<_>>(),
        "emit_errors": emit_errors,
    });
    Ok(PipelineRun { entries, runs, report, emit_errors })
}

fn last_stage(events: &[String]) -> String {
    events.last().and_then(|e| e.strip_prefix("thread:")).and_then(|r| r.split(':').next()).unwrap_or("").to_string()
}

fn verdict_of(ordinal: u32, run_id: &str, seed: &SignalSeed, r: &Run) -> Result<LedgerEntry, String> {
    let proposal_id = format!("prop-{run_id}-{ordinal:04}");
    let mut e = LedgerEntry::new(ordinal, run_id, &proposal_id, &seed.signal_id, Verdict::NotEvaluable, "core_unavailable");
    e.models = r.report["models"].as_array().cloned().unwrap_or_default();
    e.evidence_refs = vec![seed.evidence_ref.clone()];
    let out = r.payload.as_ref().and_then(|p| p.get("out"));
    let op0 = r.payload.as_ref().and_then(|p| p.pointer("/spec/compile/change_spec/operations/0"));
    let prop = r.proposal.as_ref();
    e.kind = op0.and_then(|o| o["target_kind"].as_str()).or_else(|| prop.and_then(|p| p["kind"].as_str())).map(str::to_string);
    e.op = op0.and_then(|o| o["op"].as_str()).or_else(|| prop.and_then(|p| p["op"].as_str())).map(str::to_string);
    for rc in out.and_then(|o| o.pointer("/recompute/recomputes")).and_then(Value::as_array).into_iter().flatten() {
        e.hypotheses.push(json!({"id": "h_1", "signal_id": seed.signal_id, "claimed_rate": rc["claimed_rate"], "recomputed_rate": rc["recomputed_rate"], "match": rc["match"]}));
        if let Some(ev) = rc["evidence_ref"].as_str().filter(|x| !e.evidence_refs.iter().any(|y| y == x)) {
            e.evidence_refs.push(ev.to_string());
        }
    }
    e.gates = out
        .and_then(|o| o.pointer("/gate/gates"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| Some(GateResult { gate: g["gate"].as_str()?.to_string(), status: g["status"].as_str()?.to_string() }))
        .collect();
    e.gate_verdict = out.and_then(|o| o.pointer("/gate/verdict")).and_then(Value::as_str).map(str::to_string);
    e.overrides = r.report["overrides"].as_array().cloned().unwrap_or_default();
    let set = |e: &mut LedgerEntry, v: Verdict, reason: &str, stage: &str, detail: &str| {
        e.verdict = v;
        e.reason = reason.into();
        e.stage = stage.into();
        e.detail = detail.chars().take(300).collect();
    };

    if let Some((role, code, why)) = &r.stop {
        let (v, stage) = match code.as_str() {
            "evidence_insufficient" => (Verdict::NotEvaluable, "evidence"),
            "verifier_refuted" => (Verdict::NotViable, "verifier"),
            "kind_not_supported" | "release_settings_not_allowed" => (Verdict::NotEvaluable, "compile"),
            _ => (Verdict::NotEvaluable, role.as_str()),
        };
        set(&mut e, v, code, stage, why);
        e.validate()?;
        return Ok(e);
    }
    let validation = out.and_then(|o| o.pointer("/validation/verdict")).and_then(Value::as_str);
    if validation.is_some_and(|v| v != "corroborated") {
        set(&mut e, Verdict::NotViable, "claim_not_corroborated", "validation", &format!("validation verdict {}", validation.unwrap_or("")));
        e.validate()?;
        return Ok(e);
    }
    let compile = out.and_then(|o| o.get("compile"));
    let compiled = compile.and_then(|c| c["status"].as_str()) == Some("compiled");
    let (kind, op) = (e.kind.clone().unwrap_or_default(), e.op.clone().unwrap_or_default());
    if let Some(c) = compile.filter(|_| !compiled) {
        let denied = c["denied_reason"].as_str().unwrap_or("unknown");
        let reason = match denied {
            "kind_not_supported" | "release_settings_not_allowed" => {
                if bk0_check(&kind, &op).is_ok() {
                    return Err(format!("the compile step denied ({kind},{op}) as {denied} but BK0 supports it: the engine and the matrix disagree"));
                }
                denied
            }
            "missing_precondition" => "precondition_missing",
            _ => "compile_denied",
        };
        set(&mut e, Verdict::NotEvaluable, reason, "compile", denied);
        e.validate()?;
        return Ok(e);
    }
    if compiled && let Err(why) = bk0_check(&kind, &op) {
        return Err(format!("the compile step compiled ({kind},{op}) which BK0 refuses ({why}): the engine and the matrix disagree"));
    }
    match e.gate_verdict.clone().as_deref() {
        None => {
            let stage = last_stage(&r.events);
            set(&mut e, Verdict::NotEvaluable, "core_unavailable", &stage, r.error.as_deref().unwrap_or("the job ended before the gate"));
        }
        Some("fail") => set(&mut e, Verdict::NotViable, "gate_failed", "gate", "the structural gate did not pass"),
        Some("not_evaluable") => set(&mut e, Verdict::NotEvaluable, "gate_not_evaluable", "gate", "the structural gate could not evaluate"),
        Some("pass") => {
            let native = out.and_then(|o| o.pointer("/native_eval/verdict")).and_then(Value::as_str);
            match native {
                Some("pass") => set(&mut e, Verdict::Viable, "structural_gate_passed", "gate", "the structural gate and the native evaluation passed"),
                _ if r.error.as_deref().is_some_and(|x| x.contains("native evaluation")) => set(&mut e, Verdict::NotViable, "native_eval_failed", "native_eval", r.error.as_deref().unwrap_or("")),
                _ => set(&mut e, Verdict::NotEvaluable, "core_unavailable", "native_eval", r.error.as_deref().unwrap_or("native evaluation did not run")),
            }
        }
        Some(other) => return Err(format!("unknown gate verdict {other:?}")),
    }
    e.validate()?;
    Ok(e)
}
