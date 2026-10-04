use authority::*;

pub fn tgt() -> Target {
    Target { proposal_ref: "prop-1".into(), proposal_rev: 3, candidate_hash: "sha256:aaaa".into() }
}
pub fn issuer() -> SimulatedIssuer {
    SimulatedIssuer::default()
        .with("ok", SimTicket::approver("sim-approver", 1000))
        .with("adm", SimTicket::admin("sim-admin", 1000))
}
pub fn waiting(safety: GateVerdict, improvement: GateVerdict) -> Authority {
    let i = issuer();
    let mut a = Authority::new(tgt());
    a.apply(Event::StartEvaluation, 0, &i).unwrap();
    a.apply(Event::RecordGates { safety, improvement }, 0, &i).unwrap();
    a.apply(Event::RequestDecision { decision_id: "d1".into(), expires_at: 500 }, 0, &i).unwrap();
    a
}
pub fn approve(ov: Option<Override>) -> Event {
    Event::Approve { decision_id: "d1".into(), target: tgt(), ticket: "ok".into(), override_: ov }
}
pub fn human_ov() -> Override {
    Override { by: "human".into(), actor: "sim-human".into(), reason: "accepted risk".into() }
}

#[test]
fn state_test_allows_approve_without_both_gates() {
    use GateVerdict::*;
    for (s, i) in [(Fail, Pass), (Pass, Fail), (NotEvaluable, Pass), (Pass, NotEvaluable), (Fail, Fail)] {
        let mut a = waiting(s, i);
        assert_eq!(a.apply(approve(None), 1, &issuer()), Err(AuthorityError::GateFailed), "{s:?}/{i:?}");
        assert_eq!(a.state(), State::WaitingHuman);
    }
    let mut ok = waiting(Pass, Pass);
    assert_eq!(ok.apply(approve(None), 1, &issuer()), Ok(Applied::Moved(State::Approved)));
}
