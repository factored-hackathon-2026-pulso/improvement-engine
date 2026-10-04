use authority::*;
use GateVerdict::*;

fn tgt() -> Target {
    Target { proposal_ref: "prop-1".into(), proposal_rev: 3, candidate_hash: "sha256:aaaa".into() }
}
fn issuer() -> SimulatedIssuer {
    SimulatedIssuer::default()
        .with("ok", SimTicket::approver("sim-approver", 1000))
        .with("other", SimTicket { bound_to: Some(Target { candidate_hash: "sha256:zzzz".into(), ..tgt() }), ..SimTicket::approver("h", 1000) })
}
fn waiting(s: GateVerdict, i: GateVerdict) -> Authority {
    let iss = issuer();
    let mut a = Authority::new(tgt());
    a.apply(Event::StartEvaluation, 0, &iss).unwrap();
    a.apply(Event::RecordGates { safety: s, improvement: i }, 0, &iss).unwrap();
    a.apply(Event::RequestDecision { decision_id: "d1".into(), expires_at: 500 }, 0, &iss).unwrap();
    a
}
fn approve(t: &str, ov: Option<Override>) -> Event {
    Event::Approve { decision_id: "d1".into(), target: tgt(), ticket: t.into(), override_: ov }
}
fn ov() -> Override {
    Override { by: "human".into(), actor: " sim-human ".into(), reason: " risk ".into() }
}

#[test]
fn stray_override_on_passing_gates_is_rejected() {
    let mut a = waiting(Pass, Pass);
    assert!(a.apply(approve("ok", Some(ov())), 1, &issuer()).is_err());
    assert_eq!(a.state(), State::WaitingHuman);
}

#[test]
fn override_records_actor_reason_and_forbids_quality_claims() {
    let mut a = waiting(Fail, Pass);
    a.apply(approve("ok", Some(ov())), 1, &issuer()).unwrap();
    let ap = a.approval().unwrap();
    assert!(ap.simulated && ap.quality_claims_forbidden);
    assert_eq!(ap.override_label, Some("human_override"));
    assert_eq!(ap.override_actor.as_deref(), Some("sim-human"));
    assert_eq!(ap.override_reason.as_deref(), Some("risk"));
}

#[test]
fn ticket_bound_to_another_candidate_is_refused() {
    let mut a = waiting(Pass, Pass);
    assert!(a.apply(approve("other", None), 1, &issuer()).is_err());
    assert_eq!(a.state(), State::WaitingHuman);
}

#[test]
fn replay_with_other_proposal_ref_is_not_replayed() {
    let mut a = waiting(Pass, Pass);
    a.apply(approve("ok", None), 1, &issuer()).unwrap();
    let t = Target { proposal_ref: "prop-2".into(), ..tgt() };
    let e = Event::Approve { decision_id: "d1".into(), target: t, ticket: "ok".into(), override_: None };
    assert!(a.apply(e, 2, &issuer()).is_err());
}

#[test]
fn empty_decision_id_refused() {
    let mut a = Authority::new(tgt());
    let i = issuer();
    a.apply(Event::StartEvaluation, 0, &i).unwrap();
    a.apply(Event::RecordGates { safety: Pass, improvement: Pass }, 0, &i).unwrap();
    assert!(a.apply(Event::RequestDecision { decision_id: " ".into(), expires_at: 9 }, 0, &i).is_err());
    assert_eq!(a.state(), State::Gated);
}

#[test]
fn debug_does_not_leak_tickets() {
    let e = approve("SECRET-TICKET", None);
    assert!(!format!("{e:?}").contains("SECRET-TICKET"));
    let e = Event::Publish { target: tgt(), ticket: "SECRET-TICKET".into() };
    assert!(!format!("{e:?}").contains("SECRET-TICKET"));
}
