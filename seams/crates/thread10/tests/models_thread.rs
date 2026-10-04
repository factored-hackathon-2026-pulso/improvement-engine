//! R1E: the scripted pieces of the ten-step thread (scout claim, verifier check, builder change spec) are now calls through a
//! `ModelPort`. The default is `Scripted` (the old behaviour); labels in the report come from what actually answered.
use engine::models::roleplay::{Roleplay, replay_key};
use engine::models::tps::{DEFAULT_K, scan_payload};
use engine::models::{DataClass, Label, ModelAnswer, ModelError, ModelPort, ModelRequest, Role, Scripted};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use thread10::requests::{builder_request, scout_request, verifier_request};
use thread10::{Opts, SignalSeed, run};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("t10m-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn opts(name: &str) -> Opts {
    Opts { human_override: true, ..Opts::new(tmp(name), env!("CARGO_BIN_EXE_synth_runner").into()) }
}

fn step<'a>(r: &'a Value, id: &str) -> &'a Value {
    r["steps"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap_or_else(|| panic!("step {id}"))
}

/// A port that answers like Scripted but is labelled `label`, counting calls per role (and optionally failing one role).
struct Probe {
    label: Label,
    id: &'static str,
    inner: Scripted,
    calls: RefCell<Vec<Role>>,
    fail: Option<(Role, ModelError)>,
    verdict: &'static str,
}

impl Probe {
    fn new(label: Label, id: &'static str) -> Probe {
        Probe { label, id, inner: Scripted::new(), calls: RefCell::new(vec![]), fail: None, verdict: "agree" }
    }
}

impl ModelPort for Probe {
    fn label(&self) -> Label {
        self.label
    }
    fn model_id(&self) -> String {
        self.id.into()
    }
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        self.calls.borrow_mut().push(req.role);
        if let Some((r, e)) = &self.fail
            && *r == req.role
        {
            return Err(e.clone());
        }
        let mut a = self.inner.call(req)?;
        if req.role == Role::Verifier {
            a.content = json!({"verdict": self.verdict});
        }
        Ok(ModelAnswer { model_id: self.id.into(), label: self.label, ..a })
    }
}

#[test]
fn the_requests_the_thread_builds_pass_the_treated_payload_scan() {
    let seed = SignalSeed::lab_default();
    for r in [scout_request(&seed), verifier_request(&seed, 0.3), builder_request(&seed)] {
        let s = scan_payload(&r.payload, DEFAULT_K, &r.registry);
        assert!(s.ok, "{:?}: {:?}", r.role, s.violations);
        assert_eq!(r.data_class, DataClass::Synthetic);
    }
}

#[test]
fn the_default_port_is_scripted_and_the_report_says_so_for_every_call() {
    let r = run(&opts("default")).unwrap();
    assert_eq!(r.error, None, "{:?}", r.events);
    let models = r.report["models"].as_array().unwrap();
    let roles: Vec<&str> = models.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["scout", "verifier", "builder"]);
    assert!(models.iter().all(|m| m["label"] == "scripted" && m["real"] == false && m["outcome"] == "answered"), "{models:?}");
    assert_eq!(step(&r.report, "scout")["receipt"]["provider"], "scripted");
    assert_eq!(step(&r.report, "scout")["status"], "stand-in");
    assert_eq!(step(&r.report, "opportunity")["receipt"]["provider"], "scripted");
    assert!(r.report["steps"].as_array().unwrap().iter().all(|s| s["status"] != "real"));
}

#[test]
fn an_answered_gateway_call_makes_exactly_its_steps_real_and_nothing_else() {
    let mut o = opts("gateway");
    o.model = Some(Rc::new(Probe::new(Label::Gateway, "gw-model-1")));
    let r = run(&o).unwrap();
    assert_eq!(r.error, None);
    assert_eq!((step(&r.report, "scout")["status"].as_str(), step(&r.report, "scout")["receipt"]["provider"].as_str()), (Some("real"), Some("gateway:gw-model-1")));
    assert_eq!(step(&r.report, "opportunity")["status"], "real");
    for id in ["signals", "compile", "gate", "approval", "publish", "observation"] {
        assert_ne!(step(&r.report, id)["status"], "real", "{id}");
    }
    assert_eq!(r.report["quality_claims"], "forbidden");
    let gw = r.report["ports"].as_array().unwrap().iter().find(|p| p["port"] == "llm_gateway").unwrap();
    assert!(gw["provenance"].as_str().unwrap().contains("gw-model-1"), "{gw}");
}

#[test]
fn roleplay_and_local_model_answers_are_never_real() {
    for (label, id, provider) in [(Label::Roleplay, "agent_roleplay:r1", "agent-roleplay"), (Label::LocalModel, "llama-local", "local-model")] {
        let mut o = opts(provider);
        o.model = Some(Rc::new(Probe::new(label, id)));
        let r = run(&o).unwrap();
        assert_eq!(step(&r.report, "scout")["status"], "stand-in", "{label:?}");
        assert_eq!(step(&r.report, "scout")["receipt"]["provider"], provider);
        assert!(r.report["models"].as_array().unwrap().iter().all(|m| m["real"] == false));
    }
}

#[test]
fn a_refused_model_call_blocks_the_job_before_it_runs_and_never_falls_back() {
    let mut o = opts("refused");
    let mut p = Probe::new(Label::Gateway, "gw");
    p.fail = Some((Role::Scout, ModelError::Refused("data_class e0 never reaches a hosted model".into())));
    let p = Rc::new(p);
    o.model = Some(p.clone());
    let r = run(&o).unwrap();
    assert!(r.error.as_deref().is_some_and(|e| e.contains("blocked(model_refused)")), "{:?}", r.error);
    assert_eq!(*p.calls.borrow(), vec![Role::Scout], "nothing after the refused call");
    assert!(r.events.is_empty(), "the job never ran: {:?}", r.events);
    assert_eq!(step(&r.report, "scout")["status"], "blocked(model_refused)");
    assert_eq!(step(&r.report, "recompute")["status"], "not_exercised");
    assert_eq!(r.report["models"][0]["outcome"], "refused");
}

#[test]
fn a_verifier_that_disagrees_stops_the_job_with_a_named_reason() {
    let mut o = opts("disagree");
    let mut p = Probe::new(Label::Roleplay, "agent_roleplay:v");
    p.verdict = "disagree";
    o.model = Some(Rc::new(p));
    let r = run(&o).unwrap();
    assert!(r.error.as_deref().is_some_and(|e| e.contains("blocked(verifier_refuted)")), "{:?}", r.error);
    assert!(r.events.is_empty());
    assert_eq!(step(&r.report, "validation")["status"], "not_exercised");
}

#[test]
fn a_builder_proposing_an_unsupported_kind_is_denied_by_the_compile_step() {
    let mut o = opts("flowkind");
    let mut p = Probe::new(Label::Scripted, "scripted-x");
    p.inner.op = "add".into(); // a prompt `add` is not a BK0 family
    o.model = Some(Rc::new(p));
    let r = run(&o).unwrap();
    assert_eq!(step(&r.report, "compile")["status"], "blocked(kind_not_supported)");
    assert_eq!(step(&r.report, "publish")["status"], "not_exercised");
}

#[test]
fn model_answers_are_persisted_so_a_resume_never_asks_the_model_again() {
    let mut o = opts("cache");
    let p = Rc::new(Probe::new(Label::Scripted, "scripted-c"));
    o.model = Some(p.clone());
    let first = run(&o).unwrap();
    assert_eq!(p.calls.borrow().len(), 3);
    let mut again = Opts { now: 5000, ..Opts::new(o.work.clone(), o.runner.clone()) };
    again.human_override = true;
    let p2 = Rc::new(Probe::new(Label::Scripted, "scripted-c"));
    again.model = Some(p2.clone());
    let second = run(&again).unwrap();
    assert!(p2.calls.borrow().is_empty(), "the persisted answers were reused: {:?}", p2.calls.borrow());
    assert_eq!(first.report["models"], second.report["models"], "same honest records after a resume");
}

#[test]
fn the_roleplay_port_drives_the_thread_from_recorded_answers() {
    let q = tmp("roleplay-queue");
    std::fs::create_dir_all(q.join("responses")).unwrap();
    let seed = SignalSeed::lab_default();
    let answers = [
        (scout_request(&seed), "scout", json!({"hypotheses": [{"id": "h_1", "signal_id": "sig-0001", "claimed_rate": 0.3}]})),
        (verifier_request(&seed, 0.3), "verifier", json!({"verdict": "agree"})),
        (builder_request(&seed), "builder", json!({"proposal": {"kind": "prompt", "op": "replace", "target_ref": "prompt:resumen_radicado@1", "new_ref": "prompt:resumen_radicado@2", "mechanism": "m"}})),
    ];
    for (i, (req, role, out)) in answers.into_iter().enumerate() {
        let key = replay_key(&req.system, &req.payload).unwrap();
        let doc = json!({"protocol": "roleplay-queue/1", "key": key, "provenance": "agent_roleplay", "quality_claims": "forbidden",
            "responder": {"id": format!("resp-{i}"), "role": role}, "content": {"kind": "final", "output": out}});
        std::fs::write(q.join("responses").join(format!("{key}.json")), doc.to_string()).unwrap();
    }
    let mut o = opts("roleplay");
    o.model = Some(Rc::new(Roleplay::new(&q)));
    let r = run(&o).unwrap();
    assert_eq!(r.error, None, "{:?}", r.events);
    assert_eq!(step(&r.report, "scout")["receipt"]["provider"], "agent-roleplay");
    let ids: Vec<&str> = r.report["models"].as_array().unwrap().iter().map(|m| m["model_id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["agent_roleplay:resp-0", "agent_roleplay:resp-1", "agent_roleplay:resp-2"]);
}

#[test]
fn a_persisted_answer_is_never_replayed_for_a_different_input() {
    let mut o = opts("stale");
    let p = Rc::new(Probe::new(Label::Scripted, "scripted-s"));
    o.model = Some(p.clone());
    run(&o).unwrap();
    assert_eq!(p.calls.borrow().len(), 3);
    // same work directory, a different signal row: the persisted answers belong to the old request
    let mut other = Opts { now: 5000, ..Opts::new(o.work.clone(), o.runner.clone()) };
    other.human_override = true;
    other.seed = SignalSeed { numerator: 200, ..SignalSeed::lab_default() };
    let p2 = Rc::new(Probe::new(Label::Scripted, "scripted-s"));
    other.model = Some(p2.clone());
    let second = run(&other);
    assert!(second.is_err() || !p2.calls.borrow().is_empty(), "a stale model answer was replayed for a different input");
}
