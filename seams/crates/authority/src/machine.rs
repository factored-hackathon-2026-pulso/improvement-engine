use crate::issuer::{HumanIssuer, IssuerError, Op, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Draft,
    Evaluating,
    Gated,
    WaitingHuman,
    Approved,
    Published,
    Rejected,
    Expired,
    Revoked,
}

impl State {
    pub const ALL: [State; 9] = [
        State::Draft,
        State::Evaluating,
        State::Gated,
        State::WaitingHuman,
        State::Approved,
        State::Published,
        State::Rejected,
        State::Expired,
        State::Revoked,
    ];
    pub fn name(self) -> &'static str {
        match self {
            State::Draft => "draft",
            State::Evaluating => "evaluating",
            State::Gated => "gated",
            State::WaitingHuman => "waiting_human",
            State::Approved => "approved",
            State::Published => "published",
            State::Rejected => "rejected",
            State::Expired => "expired",
            State::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateVerdict {
    Pass,
    Fail,
    NotEvaluable,
}

/// Explicit labelled human override of a non-pass gate (DEMO-0 rule, G1): the "human" is a config value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override {
    pub by: String,
    pub actor: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub decision_id: String,
    pub actor_ref: String,
    pub override_label: Option<&'static str>,
    pub simulated: bool,
    pub override_actor: Option<String>,
    pub override_reason: Option<String>,
    /// G1: a human override of a non-pass gate forbids quality claims.
    pub quality_claims_forbidden: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Event {
    StartEvaluation,
    RecordGates { safety: GateVerdict, improvement: GateVerdict },
    /// Creates the DecisionRequest bound to the target; it expires at `expires_at`.
    RequestDecision { decision_id: String, expires_at: u64 },
    Approve { decision_id: String, target: Target, ticket: String, override_: Option<Override> },
    Reject { decision_id: String, target: Target, ticket: String },
    Publish { target: Target, ticket: String },
    Revoke { target: Target, ticket: String },
    Expire,
}

impl std::fmt::Debug for Event {
    /// Tickets are credentials-adjacent: never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Approve { decision_id, target, override_, .. } => f
                .debug_struct("Approve")
                .field("decision_id", decision_id)
                .field("target", target)
                .field("ticket", &"<redacted>")
                .field("override_", override_)
                .finish(),
            Event::Reject { decision_id, target, .. } => f
                .debug_struct("Reject")
                .field("decision_id", decision_id)
                .field("target", target)
                .field("ticket", &"<redacted>")
                .finish(),
            Event::Publish { target, .. } => {
                f.debug_struct("Publish").field("target", target).field("ticket", &"<redacted>").finish()
            }
            Event::Revoke { target, .. } => {
                f.debug_struct("Revoke").field("target", target).field("ticket", &"<redacted>").finish()
            }
            Event::StartEvaluation => f.write_str("StartEvaluation"),
            Event::Expire => f.write_str("Expire"),
            Event::RecordGates { safety, improvement } => {
                f.debug_struct("RecordGates").field("safety", safety).field("improvement", improvement).finish()
            }
            Event::RequestDecision { decision_id, expires_at } => f
                .debug_struct("RequestDecision")
                .field("decision_id", decision_id)
                .field("expires_at", expires_at)
                .finish(),
        }
    }
}

impl Event {
    pub fn kind(&self) -> &'static str {
        match self {
            Event::StartEvaluation => "start_evaluation",
            Event::RecordGates { .. } => "record_gates",
            Event::RequestDecision { .. } => "request_decision",
            Event::Approve { .. } => "approve",
            Event::Reject { .. } => "reject",
            Event::Publish { .. } => "publish",
            Event::Revoke { .. } => "revoke",
            Event::Expire => "expire",
        }
    }
}

/// Named errors, mirroring the Core registry codes where one exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityError {
    IllegalTransition { state: State, event: &'static str },
    GateFailed,
    CandidateChanged,
    ProposalStale,
    StaleDecision,
    IdempotencyConflict,
    DecisionExpired,
    NotYetExpired,
    ApprovalExpired,
    ForbiddenRole,
    StepUpRequired,
    WrongIssuer,
    TicketExpired,
    UnknownTicket,
    TicketNotBound,
    OverrideOnPassingGate,
    EmptyDecisionId,
}

impl From<IssuerError> for AuthorityError {
    fn from(e: IssuerError) -> Self {
        match e {
            IssuerError::ForbiddenRole => AuthorityError::ForbiddenRole,
            IssuerError::StepUpRequired => AuthorityError::StepUpRequired,
            IssuerError::WrongIssuer => AuthorityError::WrongIssuer,
            IssuerError::Expired => AuthorityError::TicketExpired,
            IssuerError::Unknown => AuthorityError::UnknownTicket,
            IssuerError::TargetMismatch => AuthorityError::TicketNotBound,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Moved(State),
    /// Same decision re-delivered: previous receipt, no new effect.
    Replayed(State),
}

#[derive(Debug, Clone)]
struct Request {
    id: String,
    expires_at: u64,
}

#[derive(Debug, Clone)]
struct Decided {
    id: String,
    approve: bool,
    target: Target,
}

#[derive(Debug, Clone)]
pub struct Authority {
    state: State,
    target: Target,
    gates: Option<(GateVerdict, GateVerdict)>,
    request: Option<Request>,
    decided: Option<Decided>,
    approval: Option<Approval>,
    approval_expiry: u64,
    history: Vec<(State, State, &'static str)>,
}

fn is_human_override(o: &Override) -> bool {
    o.by == "human" && !o.actor.trim().is_empty() && !o.reason.trim().is_empty()
}

impl Authority {
    pub fn new(target: Target) -> Self {
        Self {
            state: State::Draft,
            target,
            gates: None,
            request: None,
            decided: None,
            approval: None,
            approval_expiry: 0,
            history: vec![],
        }
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn approval(&self) -> Option<&Approval> {
        self.approval.as_ref()
    }
    pub fn history(&self) -> &[(State, State, &'static str)] {
        &self.history
    }

    fn illegal<T>(&self, ev: &Event) -> Result<T, AuthorityError> {
        Err(AuthorityError::IllegalTransition { state: self.state, event: ev.kind() })
    }

    fn bind(&self, t: &Target) -> Result<(), AuthorityError> {
        if t.proposal_ref != self.target.proposal_ref || t.proposal_rev != self.target.proposal_rev {
            return Err(AuthorityError::ProposalStale);
        }
        if t.candidate_hash != self.target.candidate_hash {
            return Err(AuthorityError::CandidateChanged);
        }
        Ok(())
    }

    fn go(&mut self, to: State, ev: &'static str) -> Result<Applied, AuthorityError> {
        self.history.push((self.state, to, ev));
        self.state = to;
        Ok(Applied::Moved(to))
    }

    fn check_request(&self, decision_id: &str, now: u64) -> Result<(), AuthorityError> {
        let req = self.request.as_ref().ok_or(AuthorityError::StaleDecision)?;
        if decision_id != req.id {
            return Err(AuthorityError::StaleDecision);
        }
        if now >= req.expires_at {
            return Err(AuthorityError::DecisionExpired);
        }
        Ok(())
    }

    /// Apply one event. On any error the machine is unchanged.
    pub fn apply(&mut self, ev: Event, now: u64, issuer: &dyn HumanIssuer) -> Result<Applied, AuthorityError> {
        use State::*;
        match (&ev, self.state) {
            (Event::StartEvaluation, Draft) => self.go(Evaluating, "start_evaluation"),
            (Event::RecordGates { safety, improvement }, Evaluating) => {
                self.gates = Some((*safety, *improvement));
                self.go(Gated, "record_gates")
            }
            (Event::RequestDecision { decision_id, expires_at }, Gated) => {
                if decision_id.trim().is_empty() {
                    return Err(AuthorityError::EmptyDecisionId);
                }
                if *expires_at <= now {
                    return Err(AuthorityError::DecisionExpired);
                }
                self.request = Some(Request { id: decision_id.clone(), expires_at: *expires_at });
                self.go(WaitingHuman, "request_decision")
            }
            (Event::Approve { decision_id, target, ticket, override_ }, WaitingHuman) => {
                self.check_request(decision_id, now)?;
                self.bind(target)?;
                let v = issuer.verify(ticket, target, Op::Approve, now)?;
                let mut label = None;
                let mut ov_info = None;
                let both_pass = self.gates == Some((GateVerdict::Pass, GateVerdict::Pass));
                match (both_pass, override_) {
                    (true, None) => {}
                    (true, Some(_)) => return Err(AuthorityError::OverrideOnPassingGate),
                    (false, Some(o)) if is_human_override(o) => {
                        label = Some("human_override");
                        ov_info = Some((o.actor.trim().to_string(), o.reason.trim().to_string()));
                    }
                    (false, _) => return Err(AuthorityError::GateFailed),
                }
                self.decided = Some(Decided {
                    id: decision_id.clone(),
                    approve: true,
                    target: target.clone(),
                });
                self.approval = Some(Approval {
                    decision_id: decision_id.clone(),
                    actor_ref: v.actor_ref,
                    override_label: label,
                    simulated: true,
                    quality_claims_forbidden: label.is_some(),
                    override_actor: ov_info.as_ref().map(|o| o.0.clone()),
                    override_reason: ov_info.map(|o| o.1),
                });
                self.approval_expiry = v.expiry;
                self.go(Approved, "approve")
            }
            (Event::Reject { decision_id, target, ticket }, WaitingHuman) => {
                self.check_request(decision_id, now)?;
                self.bind(target)?;
                issuer.verify(ticket, target, Op::Reject, now)?;
                self.decided = Some(Decided {
                    id: decision_id.clone(),
                    approve: false,
                    target: target.clone(),
                });
                self.go(Rejected, "reject")
            }
            // idempotent replay / stale or conflicting re-delivery of a settled decision
            (
                Event::Approve { decision_id, target, .. } | Event::Reject { decision_id, target, .. },
                Approved | Published | Revoked | Rejected | Expired,
            ) => {
                let is_approve = matches!(ev, Event::Approve { .. });
                match &self.decided {
                    Some(d) if d.id == *decision_id => {
                        if d.approve == is_approve && d.target == *target {
                            Ok(Applied::Replayed(self.state))
                        } else {
                            Err(AuthorityError::IdempotencyConflict)
                        }
                    }
                    _ => Err(AuthorityError::StaleDecision),
                }
            }
            (Event::Publish { target, ticket }, Approved) => {
                if now >= self.approval_expiry {
                    return Err(AuthorityError::ApprovalExpired);
                }
                self.bind(target)?;
                issuer.verify(ticket, target, Op::Publish, now)?;
                self.go(Published, "publish")
            }
            (Event::Revoke { target, ticket }, Published) => {
                self.bind(target)?;
                issuer.verify(ticket, target, Op::Revoke, now)?;
                self.go(Revoked, "revoke")
            }
            (Event::Expire, WaitingHuman) => {
                let exp = self.request.as_ref().map(|r| r.expires_at).unwrap_or(0);
                if now < exp {
                    return Err(AuthorityError::NotYetExpired);
                }
                self.go(Expired, "expire")
            }
            (Event::Expire, Approved) => {
                if now < self.approval_expiry {
                    return Err(AuthorityError::NotYetExpired);
                }
                self.go(Expired, "expire")
            }
            _ => self.illegal(&ev),
        }
    }
}
