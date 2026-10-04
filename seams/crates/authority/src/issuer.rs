//! Issuer port: the human authority (step-up, role, actor=human) is external; Pulso never signs for a human.
//! Mirrors HumanAuthorizationPort (spec 31.8.1). No credentials ever enter this crate: a ticket is an opaque ref.
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Approve,
    Reject,
    Publish,
    Revoke,
}

/// What a human authorization is bound to (spec: proposal_ref, proposal_rev, candidate_hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub proposal_ref: String,
    pub proposal_rev: u64,
    pub candidate_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub actor_ref: String,
    pub expiry: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuerError {
    /// Principal is a bot / not actor=human, or lacks the role for the op (Core: forbidden_role).
    ForbiddenRole,
    /// Session too weak (Core: step_up_required).
    StepUpRequired,
    /// Credential not from the staff issuer.
    WrongIssuer,
    /// Ticket expired at `now`.
    Expired,
    Unknown,
    /// Ticket was issued for a different proposal/revision/candidate.
    TargetMismatch,
}

pub trait HumanIssuer {
    fn verify(&self, ticket: &str, target: &Target, op: Op, now: u64) -> Result<Verified, IssuerError>;
}

/// SIMULATED human issuer for tests/demo (label authority=claude-standin, simulated=true).
#[derive(Default)]
pub struct SimulatedIssuer {
    tickets: BTreeMap<String, SimTicket>,
}

#[derive(Clone)]
pub struct SimTicket {
    pub actor: String,
    pub human: bool,
    pub step_up: bool,
    pub issuer_ok: bool,
    pub roles: Vec<&'static str>,
    pub expiry: u64,
    /// When set, the ticket is only valid for this exact target.
    pub bound_to: Option<Target>,
}

impl SimTicket {
    pub fn approver(actor: &str, expiry: u64) -> Self {
        Self { actor: actor.into(), human: true, step_up: true, issuer_ok: true, roles: vec!["aprobador"], expiry, bound_to: None }
    }
    pub fn admin(actor: &str, expiry: u64) -> Self {
        Self { actor: actor.into(), human: true, step_up: true, issuer_ok: true, roles: vec!["admin"], expiry, bound_to: None }
    }
}

impl SimulatedIssuer {
    pub fn with(mut self, ticket: &str, t: SimTicket) -> Self {
        self.tickets.insert(ticket.into(), t);
        self
    }
}

impl HumanIssuer for SimulatedIssuer {
    fn verify(&self, ticket: &str, target: &Target, op: Op, now: u64) -> Result<Verified, IssuerError> {
        let t = self.tickets.get(ticket).ok_or(IssuerError::Unknown)?;
        if t.bound_to.as_ref().is_some_and(|b| b != target) {
            return Err(IssuerError::TargetMismatch);
        }
        if !t.issuer_ok {
            return Err(IssuerError::WrongIssuer);
        }
        if !t.human {
            return Err(IssuerError::ForbiddenRole);
        }
        if !t.step_up {
            return Err(IssuerError::StepUpRequired);
        }
        let need = if op == Op::Revoke { "admin" } else { "aprobador" };
        if !t.roles.contains(&need) {
            return Err(IssuerError::ForbiddenRole);
        }
        if now >= t.expiry {
            return Err(IssuerError::Expired);
        }
        Ok(Verified { actor_ref: t.actor.clone(), expiry: t.expiry })
    }
}
