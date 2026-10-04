//! E2 live wiring, offline: the arms / native_eval / authority / publish handlers and the compile dry-run hook over a FAKE
//! `CorePort` (the real one is `engine::live_core::LiveCore`, exercised by the ignored `live_thread.rs`).
//! What is proven here: the job payload carries the arm reports into the V2 gate, arm idempotency keys derive from
//! job + step + fence + attempt, `publish` is effectful, a gate that is not `pass` blocks WITHOUT an explicit labelled
//! simulated human override, and no Core effect happens before the decision.
use abi::{EffectState, Fence, HandlerError, InputEnvelope, JobHandler as _};
use authority::Override;
use core_client::dto::ArmReport;
use engine::adapters::thread_handlers;
use engine::executor::{ExecError, ExecOptions, execute};
use engine::live::{self, ArmCall, CorePort, FrozenInfo, LiveConfig, PublishInfo, Side, SuiteInfo};
use engine::{FileStore, JobStore, event_log, synth};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("e2live-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn runner() -> std::path::PathBuf {
    env!("CARGO_BIN_EXE_synth_runner").into()
}

const CAND: &str = "aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11";

#[derive(Default)]
struct Log(RefCell<Vec<String>>);

struct FakePort {
    log: Rc<Log>,
    digest: String,
    arm_status: &'static str,
    verdict: &'static str,
    readback: &'static str,
}

impl FakePort {
    fn new(log: &Rc<Log>) -> FakePort {
        FakePort { log: log.clone(), digest: format!("sha256:{CAND}"), arm_status: "completed", verdict: "pass", readback: "rel-new" }
    }
    fn note(&self, s: String) {
        self.log.0.borrow_mut().push(s);
    }
}

impl CorePort for FakePort {
    fn dry_run(&self, ops: &[String]) -> Result<String, String> {
        self.note(format!("dry_run:{}", ops.len()));
        Ok(self.digest.clone())
    }
    fn freeze(&self, ops: &[String], job_id: &str) -> Result<FrozenInfo, String> {
        self.note(format!("freeze:{}:{job_id}", ops.len()));
        Ok(FrozenInfo { proposal_id: "prop-1".into(), candidate_hash: CAND.into(), base_release_id: "rel-base".into(), task_binding_ref: "bind-1".into(), plan_ref: "plan-1".into(), title: "pulso-key:t".into(), rev: 1, version: "2.0.0".into() })
    }
    fn suite(&self, _f: &FrozenInfo) -> Result<SuiteInfo, String> {
        Ok(SuiteInfo { digest: "ab".repeat(32), cases: vec!["c1".into(), "c2".into()] })
    }
    fn run_arm(&self, _f: &FrozenInfo, call: &ArmCall) -> Result<ArmReport, String> {
        self.note(format!("arm:{}:{}:{}", if call.side == Side::Base { "base" } else { "cand" }, call.case_ref, call.key));
        Ok(ArmReport::from_json(&json!({"execution_id": "arm-00000000000000000000000000000000", "status": self.arm_status, "case_ref": call.case_ref, "closed_early": false, "cost_known": true})).unwrap())
    }
    fn evaluate(&self, _f: &FrozenInfo, _job: &str) -> Result<String, String> {
        self.note("evaluate".into());
        Ok(self.verdict.into())
    }
    fn approve(&self, _f: &FrozenInfo) -> Result<String, String> {
        self.note("approve".into());
        Ok("local-supervisor".into())
    }
    fn publish(&self, _f: &FrozenInfo, key: &str) -> Result<PublishInfo, String> {
        self.note(format!("publish:{key}"));
        Ok(PublishInfo { release_id: "rel-new".into(), staging_release_id: self.readback.into() })
    }
}

fn cfg(over: Option<Override>) -> LiveConfig {
    LiveConfig { human_actor: "local-supervisor".into(), human_override: over, decision_ttl_seconds: 600 }
}

fn override_() -> Override {
    Override { by: "human".into(), actor: "local-supervisor".into(), reason: "exercise approve/publish; no quality claim".into() }
}

/// sensors, recompute, validation, compile(dry-run hook), arms, gate, native_eval, authority, publish
fn job(port: FakePort, over: Option<Override>, name: &str) -> (Result<String, ExecError>, FileStore, Vec<String>) {
    let work = tmp(name);
    let (env, spec) = synth::build(&work, &runner(), None).unwrap();
    let store = FileStore::open(work.join("store")).unwrap();
    let port: Rc<dyn CorePort> = Rc::new(port);
    let hs = live::live_handlers(thread_handlers(env, Some(live::dry_run_hook(port.clone()))), port, cfg(over));
    assert_eq!(hs.len(), 9);
    let r = execute(&store, &hs, &spec, &ExecOptions::new("thread-live", "w1", 1000));
    let n = hs.len();
    let events = event_log(&store, n).unwrap();
    (r, store, events)
}

#[test]
fn arm_keys_derive_from_job_step_fence_and_attempt() {
    let k = |job: &str, step: &str, fence: u64, attempt: u64, side: &str, case: &str| live::arm_key(job, step, fence, attempt, side, case).unwrap();
    let base = k("thread-1", "arms", 1, 1, "base", "c1");
    assert_eq!(base, k("thread-1", "arms", 1, 1, "base", "c1"), "deterministic");
    for other in [k("thread-2", "arms", 1, 1, "base", "c1"), k("thread-1", "publish", 1, 1, "base", "c1"), k("thread-1", "arms", 2, 1, "base", "c1"), k("thread-1", "arms", 1, 2, "base", "c1"), k("thread-1", "arms", 1, 1, "cand", "c1"), k("thread-1", "arms", 1, 1, "base", "c2")] {
        assert_ne!(base, other);
    }
    assert!(base.len() <= 200 && base.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b)), "{base}");
    assert!(live::arm_key("a|b", "arms", 1, 1, "base", "c1").is_err(), "a `|` would make the derivation ambiguous");
}

#[test]
fn publish_is_effectful_and_nothing_before_it_is() {
    let log = Rc::new(Log::default());
    let port: Rc<dyn CorePort> = Rc::new(FakePort::new(&log));
    let env = synth::build(&tmp("eff"), &runner(), None).unwrap().0;
    let hs = live::live_handlers(thread_handlers(env, None), port, cfg(None));
    let flags: Vec<(String, bool)> = hs.iter().map(|h| (h.id().0, h.effectful())).collect();
    assert_eq!(flags.iter().filter(|(_, e)| *e).map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["publish"], "{flags:?}");
    assert_eq!(flags.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["sensors", "recompute", "validation", "compile", "arms", "gate", "native_eval", "authority", "publish"]);
}

#[test]
fn the_compile_dry_run_hook_makes_the_core_digest_the_plan_digest() {
    let log = Rc::new(Log::default());
    let (r, store, events) = job(FakePort::new(&log), Some(override_()), "dry");
    assert!(r.is_ok(), "{r:?}");
    assert!(log.0.borrow()[0].starts_with("dry_run:2"), "{:?}", log.0.borrow());
    let (payload, _) = store.get("out/3").unwrap().map(|(_, rec)| (rec, ())).unwrap();
    assert!(payload.contains(&format!("sha256:{CAND}")), "{payload}");
    assert_eq!(events[3], "thread:compile:status=compiled:semantics=claude-standin");
}

#[test]
fn arms_feed_the_gate_with_the_core_reports_and_keys_carry_fence_and_attempt() {
    let log = Rc::new(Log::default());
    let (r, _store, events) = job(FakePort::new(&log), Some(override_()), "arms");
    assert!(r.is_ok(), "{r:?}");
    let l = log.0.borrow().clone();
    let arms: Vec<&String> = l.iter().filter(|s| s.starts_with("arm:")).collect();
    assert_eq!(arms.len(), 4, "2 cases x base/candidate: {l:?}");
    let expect = live::arm_key("thread-live", "arms", 1, 1, "base", "c1").unwrap();
    assert!(arms.iter().any(|a| a.ends_with(&expect)), "{arms:?} vs {expect}");
    assert!(events[4].starts_with("thread:arms:runs=4,completed=4:"), "{}", events[4]);
    assert!(events[4].contains("oracle=suite-expect"), "{}", events[4]);
    // the gate ran on the REAL arm reports: both arms completed the same cases, so the stand-in gate does not pass
    assert!(events[5].starts_with("thread:gate:verdict="), "{}", events[5]);
    assert!(!events[5].contains("verdict=pass"), "{}", events[5]);
}

#[test]
fn a_gate_that_did_not_pass_blocks_publication_without_an_explicit_override_and_nothing_hit_the_registry() {
    let log = Rc::new(Log::default());
    let (r, _s, events) = job(FakePort::new(&log), None, "blocked");
    match r {
        Err(ExecError::Handler(HandlerError::Failed(m))) => assert!(m.contains("blocked(gate)"), "{m}"),
        other => panic!("{other:?}"),
    }
    let l = log.0.borrow();
    assert!(l.iter().any(|s| s == "evaluate"), "the native evaluation ran: {l:?}");
    assert!(!l.iter().any(|s| s == "approve" || s.starts_with("publish")), "no approve/publish without the decision: {l:?}");
    assert!(events.iter().all(|e| !e.starts_with("thread:publish")), "{events:?}");
}

#[test]
fn an_incomplete_override_is_not_an_override() {
    for bad in [Override { by: "bot".into(), actor: "a".into(), reason: "r".into() }, Override { by: "human".into(), actor: " ".into(), reason: "r".into() }, Override { by: "human".into(), actor: "a".into(), reason: "".into() }] {
        let log = Rc::new(Log::default());
        let (r, ..) = job(FakePort::new(&log), Some(bad), "badover");
        assert!(matches!(r, Err(ExecError::Handler(HandlerError::Failed(_)))), "{r:?}");
        assert!(!log.0.borrow().iter().any(|s| s == "approve"));
    }
}

#[test]
fn an_explicit_override_publishes_and_every_artifact_says_simulated() {
    let log = Rc::new(Log::default());
    let (r, _s, events) = job(FakePort::new(&log), Some(override_()), "over");
    let fin = r.expect("job completes");
    let l = log.0.borrow();
    let pos = |p: &str| l.iter().position(|s| s.starts_with(p)).unwrap_or_else(|| panic!("{p} in {l:?}"));
    assert!(pos("evaluate") < pos("approve") && pos("approve") < pos("publish:"));
    assert_eq!(events.len(), 9);
    assert!(events[7].contains("override=human_override") && events[7].contains("simulated=true") && events[7].contains("quality_claims=forbidden"), "{}", events[7]);
    assert!(events[8].starts_with("thread:publish:release=rel-new,alias=staging,readback=equal"), "{}", events[8]);
    assert!(events[8].contains("simulated_human=true"), "{}", events[8]);
    assert!(fin.contains("\"publish\":{") && fin.contains("\"override\":{") && fin.contains("\"simulated\":true"), "{fin}");
    assert!(!fin.contains('\n'));
}

#[test]
fn a_native_evaluation_that_does_not_pass_blocks_before_any_decision() {
    for verdict in ["failed_infra", "fail"] {
        let log = Rc::new(Log::default());
        let mut p = FakePort::new(&log);
        p.verdict = verdict;
        let (r, ..) = job(p, Some(override_()), "neval");
        match r {
            Err(ExecError::Handler(HandlerError::Failed(m))) => assert!(m.contains("native evaluation") && m.contains(verdict), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(!log.0.borrow().iter().any(|s| s == "approve"));
    }
}

#[test]
fn a_readback_that_is_not_the_published_release_is_an_error() {
    let log = Rc::new(Log::default());
    let mut p = FakePort::new(&log);
    p.readback = "rel-base";
    let (r, ..) = job(p, Some(override_()), "readback");
    assert!(matches!(&r, Err(ExecError::Handler(HandlerError::Failed(m))) if m.contains("readback")), "{r:?}");
}

#[test]
fn an_arm_that_ended_in_infra_failure_is_not_evidence() {
    for st in ["failed_infra", "unknown"] {
        let log = Rc::new(Log::default());
        let mut p = FakePort::new(&log);
        p.arm_status = st;
        let (r, ..) = job(p, Some(override_()), "armfail");
        assert!(matches!(&r, Err(ExecError::Handler(HandlerError::Failed(m))) if m.contains(st) && m.contains("not evidence")), "{r:?}");
        assert!(!log.0.borrow().iter().any(|s| s == "evaluate"), "no evaluation after a broken arm");
    }
}

#[test]
fn a_frozen_candidate_that_is_not_the_compiled_digest_stops_the_job_before_any_arm() {
    let log = Rc::new(Log::default());
    let mut p = FakePort::new(&log);
    p.digest = format!("sha256:{}", "bb".repeat(32)); // compile sees this; freeze returns CAND
    let (r, ..) = job(p, Some(override_()), "drift");
    assert!(matches!(&r, Err(ExecError::Handler(HandlerError::Failed(m))) if m.contains("candidate_hash")), "{r:?}");
    assert!(!log.0.borrow().iter().any(|s| s.starts_with("arm:")));
}

#[test]
fn handlers_reject_a_payload_without_the_previous_step_output() {
    let log = Rc::new(Log::default());
    let port: Rc<dyn CorePort> = Rc::new(FakePort::new(&log));
    let env = synth::build(&tmp("nopay"), &runner(), None).unwrap().0;
    let hs = live::live_handlers(thread_handlers(env, None), port, cfg(None));
    let f = Fence { worker_id: "w".into(), fence_token: 1, attempt: 1 };
    for i in [4usize, 6, 7, 8] {
        let r = hs[i].run(&f, &InputEnvelope { job_id: "j".into(), step_index: i, payload: "{\"spec\":{},\"out\":{}}".into() });
        assert!(matches!(r, Err(HandlerError::Invalid(_))), "{}: {r:?}", hs[i].id().0);
    }
    assert!(log.0.borrow().is_empty());
    let _ = EffectState::NoEffect;
}
