//! R1E: a run over N signals produces N proposals, each ending in a recorded viability verdict (ledger), still DEMO-0 honest.
use core_client::dto::ArmReport;
use debug_api::{NewEvent, RunEventSink};
use engine::FileStore;
use engine::ledger::{Ledger, Verdict};
use engine::live::{ArmCall, CorePort, FrozenInfo, PublishInfo, Side, SuiteInfo};
use engine::models::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest, Role, Scripted};
use serde_json::{Value, json};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use thread10::double::DoublePort;
use thread10::pipeline::{PipelineOpts, run_signals};
use thread10::SignalSeed;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("t10p-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn seed(id: &str, num: u64, count: u64) -> SignalSeed {
    SignalSeed { signal_id: id.into(), evidence_ref: format!("ev-{id}"), numerator: num, count }
}

fn four() -> Vec<SignalSeed> {
    vec![seed("sig-a", 120, 400), seed("sig-b", 30, 100), seed("sig-c", 5, 8), seed("sig-d", 80, 200)]
}

fn opts(name: &str, signals: Vec<SignalSeed>) -> PipelineOpts {
    PipelineOpts::new(tmp(name), env!("CARGO_BIN_EXE_synth_runner").into(), "run-p", signals)
}

#[derive(Default)]
struct Collect(Mutex<Vec<(String, NewEvent)>>);
impl RunEventSink for Collect {
    fn emit(&self, run_id: &str, ev: NewEvent) -> Result<Value, String> {
        self.0.lock().unwrap().push((run_id.into(), ev));
        Ok(json!({}))
    }
}

/// TEST FAKE (not the Core): like the offline double, except that the baseline arm does not complete case c2, so the real
/// structural gate code sees the candidate complete strictly more cases and answers `pass`.
struct ImprovingPort {
    inner: DoublePort,
    native: &'static str,
}
impl CorePort for ImprovingPort {
    fn dry_run(&self, ops: &[String]) -> Result<String, String> {
        self.inner.dry_run(ops)
    }
    fn freeze(&self, ops: &[String], job_id: &str) -> Result<FrozenInfo, String> {
        self.inner.freeze(ops, job_id)
    }
    fn suite(&self, f: &FrozenInfo) -> Result<SuiteInfo, String> {
        self.inner.suite(f)
    }
    fn run_arm(&self, f: &FrozenInfo, call: &ArmCall) -> Result<ArmReport, String> {
        if call.side == Side::Base && call.case_ref == "c2" {
            let eid = format!("arm-{}", &core_client::canon::sha256_hex(call.key.as_bytes())[..32]);
            return ArmReport::from_json(&json!({"execution_id": eid, "status": "candidate_failed", "case_ref": "c2", "closed_early": false, "cost_known": true})).map_err(|e| format!("{e:?}"));
        }
        self.inner.run_arm(f, call)
    }
    fn evaluate(&self, _f: &FrozenInfo, _job: &str) -> Result<String, String> {
        Ok(self.native.into())
    }
    fn approve(&self, f: &FrozenInfo) -> Result<String, String> {
        self.inner.approve(f)
    }
    fn publish(&self, f: &FrozenInfo, key: &str) -> Result<PublishInfo, String> {
        self.inner.publish(f, key)
    }
}

fn improving(native: &'static str) -> Rc<dyn CorePort> {
    Rc::new(ImprovingPort { inner: DoublePort::default(), native })
}

#[test]
fn n_signals_make_n_proposals_each_with_a_verdict_and_nothing_is_viable_on_the_double() {
    let mut o = opts("four", four());
    o.human_override = true;
    let r = run_signals(&o).unwrap();
    assert_eq!(r.entries.len(), 4);
    assert_eq!(r.entries.iter().map(|e| e.ordinal).collect::<Vec<_>>(), [0, 1, 2, 3]);
    let got: Vec<(String, Verdict, String)> = r.entries.iter().map(|e| (e.signal_id.clone(), e.verdict, e.reason.clone())).collect();
    assert_eq!(
        got,
        vec![
            ("sig-a".into(), Verdict::NotViable, "gate_failed".into()),
            ("sig-b".into(), Verdict::NotViable, "gate_failed".into()),
            ("sig-c".into(), Verdict::NotEvaluable, "evidence_insufficient".into()),
            ("sig-d".into(), Verdict::NotViable, "claim_not_corroborated".into()),
        ]
    );
    let ids: std::collections::BTreeSet<&str> = r.entries.iter().map(|e| e.proposal_id.as_str()).collect();
    assert_eq!(ids.len(), 4, "one distinct proposal per signal");
    assert!(r.entries.iter().all(|e| e.label == "DEMO-0" && e.validate().is_ok()));
    assert_eq!(r.report["summary"], json!({"signals": 4, "proposals": 4, "viable": 0, "not_viable": 3, "not_evaluable": 1}));
    assert_eq!(r.report["label"], "DEMO-0");
    assert_eq!(r.report["quality_claims"], "forbidden");
    // the trail: hypotheses with the recompute, evidence refs, gate results, the labelled simulated override
    let a = &r.entries[0];
    assert_eq!(a.stage, "gate");
    assert_eq!(a.hypotheses[0]["claimed_rate"], 0.3);
    assert_eq!(a.hypotheses[0]["recomputed_rate"], 0.3);
    assert_eq!(a.hypotheses[0]["match"], true);
    assert!(a.evidence_refs.contains(&"ev-sig-a".to_string()));
    assert_eq!(a.gate_verdict.as_deref(), Some("fail"));
    assert!(a.gates.iter().any(|g| g.gate == "improvement" && g.status == "fail"), "{:?}", a.gates);
    assert_eq!(a.overrides.len(), 1);
    assert_eq!((a.overrides[0]["label"].as_str(), a.overrides[0]["simulated"].as_bool()), (Some("human_override"), Some(true)));
    assert_eq!((a.kind.as_deref(), a.op.as_deref()), (Some("prompt"), Some("replace")));
    assert!(a.models.iter().all(|m| m["label"] == "scripted" && m["real"] == false) && a.models.len() == 3);
    let (c, d) = (&r.entries[2], &r.entries[3]);
    assert!(c.models.is_empty() && c.overrides.is_empty() && c.stage == "evidence", "{c:?}");
    assert_eq!(d.hypotheses[0]["match"], false);
    assert!(d.overrides.is_empty(), "the job stopped at validation");
}

#[test]
fn a_failing_gate_without_a_labelled_override_stays_blocked_and_not_viable() {
    let r = run_signals(&opts("nooverride", vec![seed("sig-a", 120, 400)])).unwrap();
    let e = &r.entries[0];
    assert_eq!((e.verdict, e.reason.as_str()), (Verdict::NotViable, "gate_failed"));
    assert!(e.overrides.is_empty());
    assert!(r.runs[0].error.as_deref().is_some_and(|x| x.contains("blocked(gate)")), "{:?}", r.runs[0].error);
}

#[test]
fn viable_only_when_the_structural_gate_passes_on_its_own() {
    for over in [false, true] {
        let mut o = opts(&format!("viable-{over}"), vec![seed("sig-a", 120, 400)]);
        o.human_override = over;
        o.core = Some(improving("pass"));
        let r = run_signals(&o).unwrap();
        let e = &r.entries[0];
        assert_eq!((e.verdict, e.reason.as_str()), (Verdict::Viable, "structural_gate_passed"), "override={over}");
        assert_eq!(e.gate_verdict.as_deref(), Some("pass"));
        assert!(e.gates.iter().all(|g| g.status == "pass") && e.overrides.is_empty(), "{e:?}");
        assert!(r.runs[0].error.is_none(), "{:?}", r.runs[0].error);
        assert_eq!(r.report["summary"]["viable"], 1);
    }
}

#[test]
fn a_gate_pass_with_a_failed_native_evaluation_is_not_viable() {
    let mut o = opts("nativefail", vec![seed("sig-a", 120, 400)]);
    o.core = Some(improving("fail"));
    let r = run_signals(&o).unwrap();
    let e = &r.entries[0];
    assert_eq!((e.verdict, e.reason.as_str()), (Verdict::NotViable, "native_eval_failed"));
    assert_eq!(e.gate_verdict.as_deref(), Some("pass"));
}

/// Scripted for scout and verifier; the builder proposes `kind` for `signal` and a prompt replace for the others.
struct PerSignal {
    kind_for: &'static str,
    kind: &'static str,
    refuse_builder_for: Option<&'static str>,
}
impl ModelPort for PerSignal {
    fn label(&self) -> Label {
        Label::Scripted
    }
    fn model_id(&self) -> String {
        "scripted-per-signal".into()
    }
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        let sid = req.payload["inputs"]["signal_id"].as_str().unwrap_or("");
        if req.role == Role::Builder && self.refuse_builder_for == Some(sid) {
            return Err(ModelError::Refused("data_class original never reaches a hosted model".into()));
        }
        let mut a = Scripted::new().call(req)?;
        if req.role == Role::Builder && sid == self.kind_for {
            a.content["proposal"]["kind"] = json!(self.kind);
        }
        Ok(a)
    }
}

#[test]
fn a_kind_outside_bk0_is_not_evaluable_with_the_matrix_reason_never_a_forged_proposal() {
    let mut o = opts("kinds", vec![seed("sig-a", 120, 400), seed("sig-b", 30, 100), seed("sig-c", 60, 200)]);
    o.model = Some(Rc::new(PerSignal { kind_for: "sig-b", kind: "flow", refuse_builder_for: None }));
    o.human_override = true;
    let r = run_signals(&o).unwrap();
    let b = &r.entries[1];
    assert_eq!((b.verdict, b.reason.as_str(), b.kind.as_deref(), b.stage.as_str()), (Verdict::NotEvaluable, "kind_not_supported", Some("flow"), "compile"));
    assert!(b.overrides.is_empty(), "nothing was approved");
    assert_eq!(r.entries[0].reason, "gate_failed", "the other proposals still ran");
    assert_eq!(r.entries.len(), 3);
}

#[test]
fn a_refused_model_call_is_not_evaluable_and_its_record_says_refused() {
    let mut o = opts("refused", vec![seed("sig-a", 120, 400), seed("sig-b", 30, 100)]);
    o.model = Some(Rc::new(PerSignal { kind_for: "", kind: "", refuse_builder_for: Some("sig-b") }));
    let r = run_signals(&o).unwrap();
    let b = &r.entries[1];
    assert_eq!((b.verdict, b.reason.as_str(), b.stage.as_str()), (Verdict::NotEvaluable, "model_refused", "builder"));
    let last = b.models.last().unwrap();
    assert_eq!((last["outcome"].as_str(), last["real"].as_bool()), (Some("refused"), Some(false)));
    assert_eq!(r.entries[0].reason, "gate_failed");
}

#[test]
fn the_ledger_is_persisted_in_the_store_and_streamed_as_proposal_verdict_events() {
    let sink = Arc::new(Collect::default());
    let mut o = opts("persist", four());
    o.human_override = true;
    o.sink = Some(sink.clone());
    let work = o.work.clone();
    let r = run_signals(&o).unwrap();
    assert!(r.emit_errors.is_empty());
    let events = sink.0.lock().unwrap();
    assert_eq!(events.len(), 4);
    for ((run_id, ev), e) in events.iter().zip(&r.entries) {
        assert_eq!((run_id.as_str(), ev.kind.as_str(), ev.entity_kind.as_str(), ev.entity_id.as_str()), ("run-p", "proposal_verdict", "proposal", e.proposal_id.as_str()));
        assert_eq!(ev.data["verdict"], e.verdict.as_str());
        assert_eq!(ev.data["reason"], e.reason.as_str());
        assert_eq!(ev.data["signal_id"], e.signal_id.as_str());
        assert!(ev.data["hypotheses"].is_array() && ev.data["gates"].is_array() && ev.data["overrides"].is_array() && ev.data["models"].is_array());
    }
    drop(events);
    let again = FileStore::open(work.join("ledger")).unwrap();
    assert_eq!(Ledger::new(&again).entries().unwrap(), r.entries, "what the console saw is what the engine store holds");
    // a second run over the same work dir replays: same entries, no second event for the same proposal
    let mut o2 = PipelineOpts::new(work, env!("CARGO_BIN_EXE_synth_runner").into(), "run-p", four());
    o2.human_override = true;
    o2.sink = Some(sink.clone());
    o2.now = 9000;
    let r2 = run_signals(&o2).unwrap();
    assert_eq!(r2.entries, r.entries);
    assert_eq!(sink.0.lock().unwrap().len(), 4, "replayed verdicts are not re-emitted");
}

#[test]
fn a_gateway_answer_is_real_in_the_models_trail_but_never_in_the_verdict() {
    struct Gw;
    impl ModelPort for Gw {
        fn label(&self) -> Label {
            Label::Gateway
        }
        fn model_id(&self) -> String {
            "gw-1".into()
        }
        fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
            let mut a = Scripted::new().call(req)?;
            a.label = Label::Gateway;
            a.model_id = "gw-1".into();
            Ok(a)
        }
    }
    let mut o = opts("gw", vec![seed("sig-a", 120, 400)]);
    o.model = Some(Rc::new(Gw));
    let r = run_signals(&o).unwrap();
    let e = &r.entries[0];
    assert!(e.models.iter().all(|m| m["real"] == true && m["provider"] == "gateway:gw-1"));
    assert_eq!(e.verdict, Verdict::NotViable);
}

#[test]
fn zero_signals_make_zero_proposals() {
    let r = run_signals(&opts("none", vec![])).unwrap();
    assert!(r.entries.is_empty() && r.runs.is_empty());
    assert_eq!(r.report["summary"]["proposals"], 0);
}
