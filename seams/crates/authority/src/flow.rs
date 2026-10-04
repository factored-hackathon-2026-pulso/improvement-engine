//! H1: human authority flow on top of the AUS machine. `DecisionCard` (diff, both gates, candidate hash, alternatives,
//! cost) is digested and the digest is bound into the decision: a decision quoting another card digest is refused.
//! Default level (label `authority=claude-standin`): the issuer is simulated. `H1r` (real approver) plugs a real
//! [`HumanIssuer`] and a [`CardRecorder`] that persists `(decision_id, card_digest)`; neither is implemented here.
use crate::issuer::{HumanIssuer, Target};
use crate::machine::{Applied, Approval, Authority, AuthorityError, Event, GateVerdict, Override, State};
use core_client::canon::sha256_hex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionCard {
    pub diff: String,
    pub safety: GateVerdict,
    pub improvement: GateVerdict,
    pub candidate_hash: String,
    pub alternatives: Vec<String>,
    pub cost_micro_usd: u64,
}

fn verdict_name(v: GateVerdict) -> &'static str {
    match v {
        GateVerdict::Pass => "pass",
        GateVerdict::Fail => "fail",
        GateVerdict::NotEvaluable => "not_evaluable",
    }
}

/// 64-hex sha256 of the length-prefixed fields of the card (injective field framing).
pub fn card_digest(c: &DecisionCard) -> String {
    let mut s = String::from("decision-card/1");
    let mut put = |x: &str| s.push_str(&format!("|{}:{x}", x.len()));
    put(&c.diff);
    put(verdict_name(c.safety));
    put(verdict_name(c.improvement));
    put(&c.candidate_hash);
    for a in &c.alternatives {
        put(a);
    }
    put(&c.alternatives.len().to_string());
    put(&c.cost_micro_usd.to_string());
    sha256_hex(s.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowError {
    Authority(AuthorityError),
    CardDigestMismatch,
    CardCandidateMismatch,
    NoCard,
}

impl From<AuthorityError> for FlowError {
    fn from(e: AuthorityError) -> Self {
        FlowError::Authority(e)
    }
}

/// Hook for H1r: record the card digest at request and at each decision. `event` is `request` | `approve` | `reject`.
pub trait CardRecorder {
    fn record(&mut self, decision_id: &str, digest: &str, event: &'static str);
}

pub struct NoRecorder;
impl CardRecorder for NoRecorder {
    fn record(&mut self, _: &str, _: &str, _: &'static str) {}
}

pub struct HumanFlow {
    auth: Authority,
    target: Target,
    card: Option<(DecisionCard, String)>,
}

impl HumanFlow {
    pub fn new(target: Target) -> Self {
        Self { auth: Authority::new(target.clone()), target, card: None }
    }
    pub fn state(&self) -> State {
        self.auth.state()
    }
    pub fn approval(&self) -> Option<&Approval> {
        self.auth.approval()
    }
    pub fn card_digest(&self) -> Option<&str> {
        self.card.as_ref().map(|c| c.1.as_str())
    }

    /// Draft -> evaluating -> gated (gates taken from the card) -> waiting_human. Returns the card digest.
    pub fn request_decision(&mut self, card: DecisionCard, decision_id: &str, expires_at: u64, now: u64, issuer: &dyn HumanIssuer) -> Result<String, FlowError> {
        self.request_decision_recorded(card, decision_id, expires_at, now, issuer, &mut NoRecorder)
    }

    pub fn request_decision_recorded(&mut self, card: DecisionCard, decision_id: &str, expires_at: u64, now: u64, issuer: &dyn HumanIssuer, rec: &mut dyn CardRecorder) -> Result<String, FlowError> {
        if card.candidate_hash != self.target.candidate_hash {
            return Err(FlowError::CardCandidateMismatch);
        }
        // validate on a clone so a refused request leaves no half-moved machine
        let mut next = self.auth.clone();
        next.apply(Event::StartEvaluation, now, issuer)?;
        next.apply(Event::RecordGates { safety: card.safety, improvement: card.improvement }, now, issuer)?;
        next.apply(Event::RequestDecision { decision_id: decision_id.into(), expires_at }, now, issuer)?;
        self.auth = next;
        let d = card_digest(&card);
        rec.record(decision_id, &d, "request");
        self.card = Some((card, d.clone()));
        Ok(d)
    }

    fn check(&self, digest: &str) -> Result<(), FlowError> {
        match &self.card {
            None => Err(FlowError::NoCard),
            Some((_, d)) if d == digest => Ok(()),
            Some(_) => Err(FlowError::CardDigestMismatch),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn approve(&mut self, decision_id: &str, digest: &str, target: Target, ticket: &str, override_: Option<Override>, now: u64, issuer: &dyn HumanIssuer) -> Result<Applied, FlowError> {
        self.approve_recorded(decision_id, digest, target, ticket, override_, now, issuer, &mut NoRecorder)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn approve_recorded(&mut self, decision_id: &str, digest: &str, target: Target, ticket: &str, override_: Option<Override>, now: u64, issuer: &dyn HumanIssuer, rec: &mut dyn CardRecorder) -> Result<Applied, FlowError> {
        self.check(digest)?;
        let r = self.auth.apply(Event::Approve { decision_id: decision_id.into(), target, ticket: ticket.into(), override_ }, now, issuer)?;
        rec.record(decision_id, digest, "approve");
        Ok(r)
    }

    pub fn reject(&mut self, decision_id: &str, digest: &str, target: Target, ticket: &str, now: u64, issuer: &dyn HumanIssuer) -> Result<Applied, FlowError> {
        self.check(digest)?;
        Ok(self.auth.apply(Event::Reject { decision_id: decision_id.into(), target, ticket: ticket.into() }, now, issuer)?)
    }

    pub fn publish(&mut self, target: Target, ticket: &str, now: u64, issuer: &dyn HumanIssuer) -> Result<Applied, FlowError> {
        Ok(self.auth.apply(Event::Publish { target, ticket: ticket.into() }, now, issuer)?)
    }

    /// Alias read: the candidate hash the alias points at; only a published candidate has one.
    pub fn alias_read(&self) -> Option<&str> {
        (self.auth.state() == State::Published).then_some(self.target.candidate_hash.as_str())
    }
}
