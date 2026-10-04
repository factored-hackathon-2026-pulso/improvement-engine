use authority::*;
use std::fs;
use std::path::Path;

use AuthorityError as E;
use GateVerdict::*;
use State::*;

fn tgt() -> Target {
    Target { proposal_ref: "prop-1".into(), proposal_rev: 3, candidate_hash: "sha256:aaaa".into() }
}
fn issuer() -> SimulatedIssuer {
    SimulatedIssuer::default()
        .with("ok", SimTicket::approver("sim-approver", 1000))
        .with("adm", SimTicket::admin("sim-admin", 1000))
        .with("bot", SimTicket { human: false, ..SimTicket::approver("bot", 1000) })
        .with("weak", SimTicket { step_up: false, ..SimTicket::approver("h", 1000) })
        .with("foreign", SimTicket { issuer_ok: false, ..SimTicket::approver("h", 1000) })
        .with("short", SimTicket::approver("h", 10))
}
fn ev(k: &str) -> Event {
    let d = || "d1".to_string();
    match k {
        "start_evaluation" => Event::StartEvaluation,
        "record_gates" => Event::RecordGates { safety: Pass, improvement: Pass },
        "request_decision" => Event::RequestDecision { decision_id: d(), expires_at: 500 },
        "approve" => Event::Approve { decision_id: "fresh".into(), target: tgt(), ticket: "ok".into(), override_: None },
        "reject" => Event::Reject { decision_id: "fresh".into(), target: tgt(), ticket: "ok".into() },
        "publish" => Event::Publish { target: tgt(), ticket: "ok".into() },
        "revoke" => Event::Revoke { target: tgt(), ticket: "adm".into() },
        "expire" => Event::Expire,
        _ => unreachable!(),
    }
}
const KINDS: [&str; 8] =
    ["start_evaluation", "record_gates", "request_decision", "approve", "reject", "publish", "revoke", "expire"];

/// Drive a fresh machine into `s` through the legal path.
fn at(s: State) -> Authority {
    let i = issuer();
    let mut a = Authority::new(tgt());
    let step = |a: &mut Authority, e: Event, now: u64| a.apply(e, now, &i).unwrap();
    if s == Draft {
        return a;
    }
    step(&mut a, ev("start_evaluation"), 0);
    if s == Evaluating {
        return a;
    }
    step(&mut a, ev("record_gates"), 0);
    if s == Gated {
        return a;
    }
    step(&mut a, ev("request_decision"), 0);
    match s {
        WaitingHuman => {}
        Rejected => {
            step(&mut a, Event::Reject { decision_id: "d1".into(), target: tgt(), ticket: "ok".into() }, 1);
        }
        Expired => {
            step(&mut a, ev("expire"), 500);
        }
        _ => {
            step(&mut a, Event::Approve { decision_id: "d1".into(), target: tgt(), ticket: "ok".into(), override_: None }, 1);
            if matches!(s, Published | Revoked) {
                step(&mut a, ev("publish"), 2);
            }
            if s == Revoked {
                step(&mut a, ev("revoke"), 3);
            }
        }
    }
    assert_eq!(a.state(), s);
    a
}

/// Expected outcome of event `k` in state `s`: Ok(next state) or the named error.
fn expected(s: State, k: &str) -> Result<State, AuthorityError> {
    let ill = || Err(E::IllegalTransition { state: s, event: ev(k).kind() });
    match (s, k) {
        (Draft, "start_evaluation") => Ok(Evaluating),
        (Evaluating, "record_gates") => Ok(Gated),
        (Gated, "request_decision") => Ok(WaitingHuman),
        (WaitingHuman, "approve") => Err(E::StaleDecision), // "fresh" id is not the open request
        (WaitingHuman, "reject") => Err(E::StaleDecision),
        (Approved | Published | Revoked | Rejected | Expired, "approve" | "reject") => Err(E::StaleDecision),
        (Approved, "publish") => Ok(Published),
        (Published, "revoke") => Ok(Revoked),
        (WaitingHuman | Approved, "expire") => Err(E::NotYetExpired), // now=1 < expiry
        _ => ill(),
    }
}

#[test]
fn every_state_x_event_is_allowed_or_a_named_error_and_never_panics() {
    let mut cells = 0;
    for s in State::ALL {
        for k in KINDS {
            let mut a = at(s);
            let before = a.history().len();
            let got = a.apply(ev(k), 1, &issuer()).map(|_| a.state());
            assert_eq!(got, expected(s, k), "{} x {k}", s.name());
            if got.is_err() {
                assert_eq!(a.state(), s, "error must not mutate");
                assert_eq!(a.history().len(), before);
            }
            cells += 1;
        }
    }
    assert_eq!(cells, 72);
}

#[test]
fn terminal_states_accept_nothing_new() {
    for s in [Rejected, Expired, Revoked] {
        for k in KINDS {
            assert!(matches!(at(s).apply(ev(k), 9999, &issuer()), Err(_)), "{} x {k}", s.name());
        }
    }
}

#[test]
fn happy_path_and_history() {
    let a = at(Revoked);
    let names: Vec<_> = a.history().iter().map(|h| h.2).collect();
    assert_eq!(
        names,
        ["start_evaluation", "record_gates", "request_decision", "approve", "publish", "revoke"]
    );
    assert_eq!(LABEL, "authority=claude-standin");
    assert!(at(Approved).approval().unwrap().simulated);
}

fn waiting(s: GateVerdict, i: GateVerdict) -> Authority {
    let iss = issuer();
    let mut a = Authority::new(tgt());
    a.apply(ev("start_evaluation"), 0, &iss).unwrap();
    a.apply(Event::RecordGates { safety: s, improvement: i }, 0, &iss).unwrap();
    a.apply(ev("request_decision"), 0, &iss).unwrap();
    a
}
fn approve_with(ov: Option<Override>) -> Event {
    Event::Approve { decision_id: "d1".into(), target: tgt(), ticket: "ok".into(), override_: ov }
}
fn ov(by: &str, actor: &str, reason: &str) -> Override {
    Override { by: by.into(), actor: actor.into(), reason: reason.into() }
}

#[test]
fn labelled_human_override_unblocks_a_failed_gate_and_is_recorded() {
    let mut a = waiting(Fail, NotEvaluable);
    a.apply(approve_with(Some(ov("human", "sim-human", "accepted risk"))), 1, &issuer()).unwrap();
    let ap = a.approval().unwrap();
    assert_eq!(ap.override_label, Some("human_override"));
    assert!(ap.simulated);
    // a clean pass carries no override label
    assert_eq!(at(Approved).approval().unwrap().override_label, None);
}

#[test]
fn malformed_override_does_not_unblock() {
    for o in [ov("bot", "x", "r"), ov("human", " ", "r"), ov("human", "x", ""), ov("agent", "", "")] {
        let mut a = waiting(Fail, Pass);
        assert_eq!(a.apply(approve_with(Some(o)), 1, &issuer()), Err(E::GateFailed));
        assert_eq!(a.state(), WaitingHuman);
    }
}

#[test]
fn reject_needs_no_gate_pass() {
    let mut a = waiting(Fail, Fail);
    let r = Event::Reject { decision_id: "d1".into(), target: tgt(), ticket: "ok".into() };
    assert_eq!(a.apply(r, 1, &issuer()), Ok(Applied::Moved(Rejected)));
}

#[test]
fn decision_bound_to_candidate_hash_and_revision() {
    let mut a = waiting(Pass, Pass);
    let mut t = tgt();
    t.candidate_hash = "sha256:bbbb".into();
    let e = Event::Approve { decision_id: "d1".into(), target: t, ticket: "ok".into(), override_: None };
    assert_eq!(a.apply(e, 1, &issuer()), Err(E::CandidateChanged));
    let mut t = tgt();
    t.proposal_rev = 4;
    let e = Event::Approve { decision_id: "d1".into(), target: t, ticket: "ok".into(), override_: None };
    assert_eq!(a.apply(e, 1, &issuer()), Err(E::ProposalStale));
    let mut p = at(Approved);
    let mut t = tgt();
    t.candidate_hash = "sha256:bbbb".into();
    assert_eq!(p.apply(Event::Publish { target: t, ticket: "ok".into() }, 2, &issuer()), Err(E::CandidateChanged));
    assert_eq!(p.state(), Approved);
}

#[test]
fn issuer_port_rejections_map_to_named_errors() {
    for (ticket, want) in [
        ("bot", E::ForbiddenRole),
        ("weak", E::StepUpRequired),
        ("foreign", E::WrongIssuer),
        ("short", E::TicketExpired),
        ("nope", E::UnknownTicket),
        ("adm", E::ForbiddenRole), // admin is not approver
    ] {
        let mut a = waiting(Pass, Pass);
        let e = Event::Approve { decision_id: "d1".into(), target: tgt(), ticket: ticket.into(), override_: None };
        assert_eq!(a.apply(e, 20, &issuer()), Err(want), "{ticket}");
        assert_eq!(a.state(), WaitingHuman);
    }
    // revoke needs admin, not approver
    let mut p = at(Published);
    assert_eq!(p.apply(Event::Revoke { target: tgt(), ticket: "ok".into() }, 3, &issuer()), Err(E::ForbiddenRole));
}

#[test]
fn decisions_are_idempotent_and_stale_or_conflicting_replays_rejected() {
    let mut a = at(Approved);
    assert_eq!(a.apply(approve_with(None), 5, &issuer()), Ok(Applied::Replayed(Approved)));
    assert_eq!(a.history().len(), 4);
    // same id, different decision
    let rej = Event::Reject { decision_id: "d1".into(), target: tgt(), ticket: "ok".into() };
    assert_eq!(a.apply(rej, 5, &issuer()), Err(E::IdempotencyConflict));
    // same id, different hash
    let mut t = tgt();
    t.candidate_hash = "sha256:bbbb".into();
    let e = Event::Approve { decision_id: "d1".into(), target: t, ticket: "ok".into(), override_: None };
    assert_eq!(a.apply(e, 5, &issuer()), Err(E::IdempotencyConflict));
    // replay after publish still returns previous receipt
    let mut p = at(Published);
    assert_eq!(p.apply(approve_with(None), 9, &issuer()), Ok(Applied::Replayed(Published)));
    // unknown decision id is stale
    let mut w = waiting(Pass, Pass);
    let e = Event::Approve { decision_id: "old".into(), target: tgt(), ticket: "ok".into(), override_: None };
    assert_eq!(w.apply(e, 1, &issuer()), Err(E::StaleDecision));
    // replayed reject
    let mut r = at(Rejected);
    let rej = Event::Reject { decision_id: "d1".into(), target: tgt(), ticket: "ok".into() };
    assert_eq!(r.apply(rej, 5, &issuer()), Ok(Applied::Replayed(Rejected)));
}

#[test]
fn expiry_of_request_and_of_approval() {
    let mut w = waiting(Pass, Pass);
    assert_eq!(w.apply(approve_with(None), 500, &issuer()), Err(E::DecisionExpired));
    assert_eq!(w.apply(ev("expire"), 499, &issuer()), Err(E::NotYetExpired));
    assert_eq!(w.apply(ev("expire"), 500, &issuer()), Ok(Applied::Moved(Expired)));
    // approval expiry is the issuer ticket expiry (1000)
    let mut a = at(Approved);
    assert_eq!(a.apply(ev("publish"), 1000, &issuer()), Err(E::ApprovalExpired));
    assert_eq!(a.apply(ev("expire"), 1000, &issuer()), Ok(Applied::Moved(Expired)));
    // a request cannot be opened already expired
    let mut g = at(Gated);
    let e = Event::RequestDecision { decision_id: "d2".into(), expires_at: 0 };
    assert_eq!(g.apply(e, 0, &issuer()), Err(E::DecisionExpired));
}

#[test]
fn authority_crate_is_free_of_crates_core() {
    let m = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = fs::read_to_string(m).unwrap();
    let deps = text.split("[dependencies]").nth(1).unwrap_or("");
    assert!(!deps.contains("crates/core") && !deps.contains("core ="), "authority depends on crates/core");
}
