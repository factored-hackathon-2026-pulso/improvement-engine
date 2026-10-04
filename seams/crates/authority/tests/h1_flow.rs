use authority::flow::*;
use authority::*;

fn tgt() -> Target {
    Target { proposal_ref: "prop-1".into(), proposal_rev: 3, candidate_hash: "sha256:aaaa".into() }
}
fn card(safety: GateVerdict, improvement: GateVerdict) -> DecisionCard {
    DecisionCard {
        diff: "--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y\n".into(),
        safety,
        improvement,
        candidate_hash: "sha256:aaaa".into(),
        alternatives: vec!["do_nothing".into(), "catalogue:rename-guard@1+r1".into()],
        cost_micro_usd: 1234,
    }
}
fn issuer() -> SimulatedIssuer {
    SimulatedIssuer::default().with("ok", SimTicket::approver("sim-approver", 1000)).with("adm", SimTicket::admin("sim-admin", 1000))
}
fn waiting(c: DecisionCard) -> (HumanFlow, String) {
    let mut f = HumanFlow::new(tgt());
    let d = f.request_decision(c, "d1", 500, 0, &issuer()).unwrap();
    (f, d)
}

#[test]
fn request_moves_to_waiting_human_with_card_digest() {
    let (f, digest) = waiting(card(GateVerdict::Pass, GateVerdict::Pass));
    assert_eq!(f.state(), State::WaitingHuman);
    assert_eq!(digest.len(), 64);
    assert_eq!(f.card_digest(), Some(digest.as_str()));
}

#[test]
fn approve_publish_then_alias_read() {
    let (mut f, d) = waiting(card(GateVerdict::Pass, GateVerdict::Pass));
    assert_eq!(f.alias_read(), None);
    f.approve("d1", &d, tgt(), "ok", None, 1, &issuer()).unwrap();
    assert_eq!(f.alias_read(), None, "approved is not published");
    f.publish(tgt(), "ok", 2, &issuer()).unwrap();
    assert_eq!(f.state(), State::Published);
    assert_eq!(f.alias_read(), Some("sha256:aaaa"));
}

#[test]
fn approve_without_both_gates_is_refused() {
    let (mut f, d) = waiting(card(GateVerdict::Pass, GateVerdict::Fail));
    let e = f.approve("d1", &d, tgt(), "ok", None, 1, &issuer()).unwrap_err();
    assert_eq!(e, FlowError::Authority(AuthorityError::GateFailed));
    assert_eq!(f.state(), State::WaitingHuman);
}

#[test]
fn labelled_override_is_the_only_way_past_a_failed_gate() {
    let (mut f, d) = waiting(card(GateVerdict::Pass, GateVerdict::NotEvaluable));
    let ov = Override { by: "human".into(), actor: "sim-approver".into(), reason: "demo".into() };
    f.approve("d1", &d, tgt(), "ok", Some(ov), 1, &issuer()).unwrap();
    assert_eq!(f.approval().unwrap().override_label, Some("human_override"));
    assert!(f.approval().unwrap().quality_claims_forbidden);
}

#[test]
fn decision_for_a_different_card_digest_is_refused() {
    let (mut f, _) = waiting(card(GateVerdict::Pass, GateVerdict::Pass));
    let other = card_digest(&card(GateVerdict::Pass, GateVerdict::Fail));
    assert_eq!(f.approve("d1", &other, tgt(), "ok", None, 1, &issuer()), Err(FlowError::CardDigestMismatch));
    assert_eq!(f.reject("d1", &other, tgt(), "ok", 1, &issuer()), Err(FlowError::CardDigestMismatch));
    assert_eq!(f.state(), State::WaitingHuman);
}

#[test]
fn reject_is_terminal_and_has_no_alias() {
    let (mut f, d) = waiting(card(GateVerdict::Pass, GateVerdict::Pass));
    f.reject("d1", &d, tgt(), "ok", 1, &issuer()).unwrap();
    assert_eq!(f.state(), State::Rejected);
    assert_eq!(f.alias_read(), None);
}

#[test]
fn card_for_another_candidate_is_refused_and_digest_changes_with_content() {
    let mut f = HumanFlow::new(tgt());
    let mut c = card(GateVerdict::Pass, GateVerdict::Pass);
    c.candidate_hash = "sha256:bbbb".into();
    assert_eq!(f.request_decision(c, "d1", 500, 0, &issuer()), Err(FlowError::CardCandidateMismatch));
    assert_ne!(card_digest(&card(GateVerdict::Pass, GateVerdict::Pass)), card_digest(&card(GateVerdict::Fail, GateVerdict::Pass)));
}

#[test]
fn recorder_hook_receives_card_digest_on_request_and_decision() {
    #[derive(Default)]
    struct Rec(Vec<(String, String, &'static str)>);
    impl CardRecorder for Rec {
        fn record(&mut self, decision_id: &str, digest: &str, event: &'static str) {
            self.0.push((decision_id.into(), digest.into(), event));
        }
    }
    let mut r = Rec::default();
    let mut f = HumanFlow::new(tgt());
    let d = f.request_decision_recorded(card(GateVerdict::Pass, GateVerdict::Pass), "d1", 500, 0, &issuer(), &mut r).unwrap();
    f.approve_recorded("d1", &d, tgt(), "ok", None, 1, &issuer(), &mut r).unwrap();
    assert_eq!(r.0.iter().map(|x| x.2).collect::<Vec<_>>(), ["request", "approve"]);
    assert!(r.0.iter().all(|x| x.1 == d && x.0 == "d1"));
}
