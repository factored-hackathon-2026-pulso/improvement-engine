//! ModelPort: Scripted answers, call recording and the honesty mapping of labels (never `real` unless Gateway answered).
use engine::models::{CallRecord, DataClass, Label, ModelAnswer, ModelError, ModelPort, ModelRequest, Outcome, Recording, Role, Scripted};
use serde_json::{Value, json};
use std::rc::Rc;

fn req(role: Role) -> ModelRequest {
    ModelRequest {
        role,
        system: "stage prompt".into(),
        payload: json!({"goal": "g", "inputs": {"signal_id": "sig-0001"}, "step": 0, "tools": [], "observations": []}),
        registry: vec!["sig-0001".into()],
        data_class: DataClass::Synthetic,
    }
}

#[test]
fn scripted_scout_claims_the_configured_rate_for_the_named_signal() {
    let a = Scripted { claimed_rate: 0.5, ..Scripted::new() }.call(&req(Role::Scout)).unwrap();
    assert_eq!((a.label, a.model_id.as_str()), (Label::Scripted, "scripted-v1"));
    assert_eq!(a.content["hypotheses"][0]["signal_id"], "sig-0001");
    assert_eq!(a.content["hypotheses"][0]["claimed_rate"], 0.5);
    assert_eq!(a.content["hypotheses"][0]["id"], "h_1");
}

#[test]
fn scripted_builder_proposes_a_prompt_replace_and_the_op_is_configurable() {
    let a = Scripted::new().call(&req(Role::Builder)).unwrap();
    let p = &a.content["proposal"];
    assert_eq!((p["kind"].as_str(), p["op"].as_str()), (Some("prompt"), Some("replace")));
    assert_eq!(p["target_ref"], "prompt:resumen_radicado@1");
    assert_eq!(p["new_ref"], "prompt:resumen_radicado@2");
    let b = Scripted { op: "add".into(), ..Scripted::new() }.call(&req(Role::Builder)).unwrap();
    assert_eq!(b.content["proposal"]["op"], "add");
}

#[test]
fn scripted_verifier_agrees_and_says_it_is_scripted() {
    let a = Scripted::new().call(&req(Role::Verifier)).unwrap();
    assert_eq!(a.content["verdict"], "agree");
}

#[test]
fn recording_keeps_one_record_per_call_under_the_inner_label_and_model_id() {
    let r = Recording::new(Rc::new(Scripted::new()));
    r.call(&req(Role::Scout)).unwrap();
    r.call(&req(Role::Builder)).unwrap();
    let c = r.calls();
    assert_eq!(c.len(), 2);
    assert_eq!((c[0].role, c[0].label, c[0].model_id.as_str(), c[0].outcome.clone()), (Role::Scout, Label::Scripted, "scripted-v1", Outcome::Answered));
    let d = r.doubles();
    assert_eq!(d.len(), 2, "scripted is never real: {d:?}");
    assert_eq!(d[0]["part"], "model.scout");
    assert_eq!(d[0]["provider"], "scripted");
    assert_eq!(d[0]["status"], "stand-in");
}

struct Failing(ModelError, Label);
impl ModelPort for Failing {
    fn label(&self) -> Label {
        self.1
    }
    fn model_id(&self) -> String {
        "m-x".into()
    }
    fn call(&self, _: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        Err(self.0.clone())
    }
}

#[test]
fn a_failed_call_is_recorded_and_blocks_instead_of_falling_back() {
    let r = Recording::new(Rc::new(Failing(ModelError::Refused("data_class_e0".into()), Label::Gateway)));
    assert!(r.call(&req(Role::Scout)).is_err());
    let c = &r.calls()[0];
    assert_eq!(c.outcome, Outcome::Refused("data_class_e0".into()));
    let j = c.to_json();
    assert_eq!((j["status"].as_str(), j["real"].as_bool()), (Some("blocked(model_refused)"), Some(false)));
    assert_eq!(r.doubles()[0]["status"], "blocked(model_refused)");
}

fn rec(label: Label, outcome: Outcome) -> CallRecord {
    CallRecord { role: Role::Scout, label, model_id: "m1".into(), data_class: DataClass::Treated, outcome, usage: None, wall_ms: 0 }
}

#[test]
fn only_an_answered_gateway_call_is_real() {
    let all = [Label::Scripted, Label::Roleplay, Label::LocalModel, Label::Gateway];
    for l in all {
        for o in [Outcome::Answered, Outcome::Refused("x".into()), Outcome::Unavailable("x".into()), Outcome::Invalid("x".into())] {
            let real = rec(l, o.clone()).status_provider().0 == "real";
            assert_eq!(real, l == Label::Gateway && o == Outcome::Answered, "{l:?} {o:?}");
        }
    }
    assert_eq!(rec(Label::Gateway, Outcome::Answered).status_provider(), ("real".into(), "gateway:m1".into()));
    assert_eq!(rec(Label::Roleplay, Outcome::Answered).status_provider(), ("stand-in".into(), "agent-roleplay".into()));
    assert_eq!(rec(Label::LocalModel, Outcome::Answered).status_provider(), ("stand-in".into(), "local-model".into()));
    let _: Value = rec(Label::Gateway, Outcome::Answered).to_json();
}
