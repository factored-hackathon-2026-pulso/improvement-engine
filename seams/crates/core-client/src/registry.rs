//! Rust client for Core's `/v1/registry` (NOT `/internal/v1`): proposal read, human-authorized approve and publish,
//! alias and release reads. Shapes and refusal semantics are those of the reviewed Python `RealCore`
//! (`e2e-core/src/claude_standin/core_hooks.py`) and of the pinned agent-core registry (`registry/http.py`,
//! `service.py`); no route here is invented.
//!
//! HONEST LABEL: with `LocalSimAuthorizer` every approval is a claude-standin for the human (simulated issuer).
//! Core verifies only signature, `auth.level`, role and `exp` of the principal and IGNORES the binding attrs, so THIS
//! client enforces them before sending: the JWS must be for this operation, this proposal, this candidate hash and this
//! revision, and a JWS is single use (a second use is refused locally). Core still refuses the same things server side
//! (`candidate_changed`, `illegal_transition`); the `probe_*` methods send deliberately-bad requests (no client guard) to
//! PROVE that, as the Python `approve_wrong_hash` / `approve_replay` calls do.
//! Tokens are `Jws` values (redacting `Debug`); no error carries one.
use crate::authorizer::{AuthError, Authorizer, Jws, ProposalTarget};
use crate::client::{path_param_ok, pct};
use crate::http::{self, HttpError};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

/// The client refused to send: nothing reached Core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The body hash is not the frozen candidate hash.
    CandidateHashMismatch,
    /// The JWS is bound to another operation / proposal / hash / revision (or is not a readable principal).
    JwsBindingMismatch(&'static str),
    /// This JWS was already used for a request.
    ReplayedJws,
    InvalidPathParam(String),
    InvalidIdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    Refused(Refusal),
    Auth(AuthError),
    /// `sent=false`: connect failed. `sent=true`: outcome unknown.
    Transport { sent: bool, message: String },
    /// Core answered non-2xx: `code` is the registry problem code (`candidate_changed`, `illegal_transition`, ...).
    Http { status: u16, code: Option<String> },
    Malformed(String),
    /// A 2xx answer that breaks an invariant (publish answered the base release, alias mismatch, ...).
    Invariant(String),
    /// Sequencing: nothing to publish, alias read before publish, ...
    Flow(String),
}

impl From<AuthError> for RegistryError {
    fn from(e: AuthError) -> Self {
        RegistryError::Auth(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalView {
    pub rev: u64,
    pub state: String,
    pub candidate_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub actor: String,
    pub decision: String,
    pub candidate_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegAliasState {
    pub agent_id: String,
    pub alias: String,
    pub release_id: String,
    pub status: String,
}

/// Raw outcome of a probe (a request the client would have refused).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub status: u16,
    pub code: Option<String>,
}

pub struct RegistryClient {
    addr: String,
    timeout: Duration,
    used: Mutex<HashSet<String>>,
}

fn problem_code(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body).ok()?.get("code")?.as_str().map(str::to_string)
}

fn attr<'a>(claims: &'a Value, k: &str) -> Option<&'a str> {
    claims.get("attrs")?.get(k)?.as_str()
}

impl RegistryClient {
    /// `addr` is `host:port` of Core (plain HTTP, local sandbox).
    pub fn new(addr: &str, timeout: Duration) -> RegistryClient {
        RegistryClient { addr: addr.into(), timeout, used: Mutex::new(HashSet::new()) }
    }

    fn send(&self, method: &str, path: &str, bearer: &Jws, extra: &[(&str, String)], body: Option<&Value>) -> Result<(u16, Vec<u8>), RegistryError> {
        let mut headers = vec![("Authorization", format!("Bearer {}", bearer.reveal()))];
        headers.extend(extra.iter().map(|(k, v)| (*k, v.clone())));
        let bytes = body.map(|b| serde_json::to_vec(b).expect("json"));
        let full = format!("/v1/registry{path}");
        match http::request(&self.addr, method, &full, &headers, bytes.as_deref(), self.timeout) {
            Ok(r) => Ok((r.status, r.body)),
            Err(HttpError::Connect(m)) => Err(RegistryError::Transport { sent: false, message: m }),
            Err(HttpError::Io(m) | HttpError::Protocol(m)) => Err(RegistryError::Transport { sent: true, message: m }),
        }
    }

    fn ok_json(&self, method: &str, path: &str, bearer: &Jws, extra: &[(&str, String)], body: Option<&Value>) -> Result<Value, RegistryError> {
        let (status, raw) = self.send(method, path, bearer, extra, body)?;
        if !(200..300).contains(&status) {
            return Err(RegistryError::Http { status, code: problem_code(&raw) });
        }
        serde_json::from_slice(&raw).map_err(|e| RegistryError::Malformed(e.to_string()))
    }

    fn seg(p: &str) -> Result<String, RegistryError> {
        if path_param_ok(p) { Ok(pct(p)) } else { Err(RegistryError::Refused(Refusal::InvalidPathParam(format!("{p:?}")))) }
    }

    /// Single-use bookkeeping: the first use of a JWS records it, a second one is refused locally.
    fn spend(&self, jws: &Jws) -> Result<(), RegistryError> {
        if !self.used.lock().expect("lock").insert(jws.fingerprint()) {
            return Err(RegistryError::Refused(Refusal::ReplayedJws));
        }
        Ok(())
    }

    /// The JWS must be exactly for `operation` on `expected` (Core ignores these attrs; we do not).
    fn check_binding(jws: &Jws, operation: &str, expected: &ProposalTarget) -> Result<(), RegistryError> {
        let c = jws.claims().ok_or(RegistryError::Refused(Refusal::JwsBindingMismatch("unreadable")))?;
        let bad = |f: &'static str| RegistryError::Refused(Refusal::JwsBindingMismatch(f));
        if attr(&c, "operation") != Some(operation) {
            return Err(bad("operation"));
        }
        if attr(&c, "proposal_id") != Some(expected.proposal_id.as_str()) {
            return Err(bad("proposal_id"));
        }
        if attr(&c, "candidate_hash") != Some(expected.candidate_hash.as_str()) {
            return Err(bad("candidate_hash"));
        }
        if attr(&c, "expected_revision") != Some(expected.expected_revision.to_string().as_str()) {
            return Err(bad("expected_revision"));
        }
        Ok(())
    }

    /// `GET /proposals/{pid}` (reads use the bot credential, never a human JWS).
    pub fn proposal(&self, proposal_id: &str, bearer: &Jws) -> Result<ProposalView, RegistryError> {
        let v = self.ok_json("GET", &format!("/proposals/{}", Self::seg(proposal_id)?), bearer, &[], None)?;
        let p = v.get("proposal").ok_or_else(|| RegistryError::Malformed("no proposal".into()))?;
        Ok(ProposalView {
            rev: p.get("rev").and_then(Value::as_u64).ok_or_else(|| RegistryError::Malformed("rev".into()))?,
            state: p.get("state").and_then(Value::as_str).unwrap_or_default().to_string(),
            candidate_hash: p.get("candidate_hash").and_then(Value::as_str).map(str::to_string),
        })
    }

    /// `POST /proposals/{pid}/approve` `{candidate_hash}`. Refused locally unless `body_hash` is the frozen candidate hash
    /// in `expected` and `jws` is a fresh single-use approve authorization bound to `expected`.
    pub fn approve(&self, expected: &ProposalTarget, body_hash: &str, jws: &Jws) -> Result<Approval, RegistryError> {
        if body_hash != expected.candidate_hash {
            return Err(RegistryError::Refused(Refusal::CandidateHashMismatch));
        }
        Self::check_binding(jws, "approve", expected)?;
        let path = format!("/proposals/{}/approve", Self::seg(&expected.proposal_id)?);
        self.spend(jws)?;
        let v = self.ok_json("POST", &path, jws, &[], Some(&json!({"candidate_hash": body_hash})))?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Ok(Approval {
            actor: s("actor").ok_or_else(|| RegistryError::Malformed("actor".into()))?,
            decision: s("decision").ok_or_else(|| RegistryError::Malformed("decision".into()))?,
            candidate_hash: s("candidate_hash").ok_or_else(|| RegistryError::Malformed("candidate_hash".into()))?,
        })
    }

    /// Tamper / replay probe: sends `body_hash` with `jws` WITHOUT any client guard, to prove Core refuses it.
    /// Never use for the real approval.
    pub fn probe_approve(&self, proposal_id: &str, body_hash: &str, jws: &Jws) -> Result<ProbeOutcome, RegistryError> {
        let path = format!("/proposals/{}/approve", Self::seg(proposal_id)?);
        let (status, raw) = self.send("POST", &path, jws, &[], Some(&json!({"candidate_hash": body_hash})))?;
        Ok(ProbeOutcome { status, code: problem_code(&raw) })
    }

    /// `POST /proposals/{pid}/publish` with the mandatory `Idempotency-Key`; returns the release id. A 2xx without a
    /// release id is an error; comparing it with the base release is the caller's (`RegistryFlow::publish`).
    pub fn publish(&self, expected: &ProposalTarget, jws: &Jws, idempotency_key: &str) -> Result<String, RegistryError> {
        if !(1..=255).contains(&idempotency_key.len()) {
            return Err(RegistryError::Refused(Refusal::InvalidIdempotencyKey));
        }
        Self::check_binding(jws, "publish", expected)?;
        let path = format!("/proposals/{}/publish", Self::seg(&expected.proposal_id)?);
        self.spend(jws)?;
        let v = self.ok_json("POST", &path, jws, &[("Idempotency-Key", idempotency_key.to_string())], None)?;
        match v.get("release_id").and_then(Value::as_str) {
            Some(r) if !r.is_empty() => Ok(r.to_string()),
            _ => Err(RegistryError::Invariant("publish answered without a release_id".into())),
        }
    }

    /// `GET /aliases/{agent}/{alias}`; the answer must be for the requested alias and carry a release id.
    pub fn alias(&self, agent_id: &str, alias: &str, bearer: &Jws) -> Result<RegAliasState, RegistryError> {
        let v = self.ok_json("GET", &format!("/aliases/{}/{}", Self::seg(agent_id)?, Self::seg(alias)?), bearer, &[], None)?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let st = RegAliasState { agent_id: s("agent_id"), alias: s("alias"), release_id: s("release_id"), status: s("status") };
        if st.alias != alias || st.release_id.is_empty() {
            return Err(RegistryError::Invariant("alias answer does not match the requested alias or lacks release_id".into()));
        }
        Ok(st)
    }

    /// `GET /releases/{rid}` (raw `ReleaseDetail`).
    pub fn release(&self, release_id: &str, bearer: &Jws) -> Result<Value, RegistryError> {
        let v = self.ok_json("GET", &format!("/releases/{}", Self::seg(release_id)?), bearer, &[], None)?;
        if v.get("release_id").and_then(Value::as_str) != Some(release_id) {
            return Err(RegistryError::Invariant("release answer is not the requested release".into()));
        }
        Ok(v)
    }
}

/// What `RegistryFlow::approve` proved (the same fields as the Python `approve` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalReport {
    pub approver: String,
    pub decision: String,
    pub candidate_hash: String,
    /// Core refused an approval for a hash that is not the candidate (`409 candidate_changed`).
    pub tamper_refused: bool,
    /// Core refused a second use of the approving JWS (`409 illegal_transition`).
    pub replay_refused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub release_id: String,
    pub alias: &'static str,
}

/// Step 8/9 sequencing for one frozen proposal (the Rust `RealCore.approve/publish/alias_read`).
/// DOCUMENTED HOOK (not wired): the K3 writer flow calls this after `freeze_draft` returned a `FrozenProposal` and the
/// Core native evaluation passed (the registry approves only an `evaluated` proposal):
/// `RegistryFlow::new(&client, &authorizer, bot_jws, agent, frozen.proposal_id, frozen.candidate_hash, frozen.base_release_id)`.
pub struct RegistryFlow<'a, A: Authorizer> {
    client: &'a RegistryClient,
    authorizer: &'a A,
    bot: Jws,
    agent_id: String,
    proposal_id: String,
    candidate_hash: String,
    base_release_id: Option<String>,
    approved: bool,
    published: Option<String>,
}

impl<'a, A: Authorizer> RegistryFlow<'a, A> {
    pub fn new(client: &'a RegistryClient, authorizer: &'a A, bot: Jws, agent_id: &str, proposal_id: &str, candidate_hash: &str, base_release_id: Option<String>) -> Self {
        RegistryFlow {
            client,
            authorizer,
            bot,
            agent_id: agent_id.into(),
            proposal_id: proposal_id.into(),
            candidate_hash: candidate_hash.into(),
            base_release_id,
            approved: false,
            published: None,
        }
    }

    /// Rebuilds the flow for the publish half when the approval happened in an earlier call (a separate job handler, a
    /// restarted process) and the job recorded it. It trusts nothing: if the proposal is not approved, Core refuses the
    /// publish (`409 illegal_transition`).
    pub fn resumed_after_approval(client: &'a RegistryClient, authorizer: &'a A, bot: Jws, agent_id: &str, proposal_id: &str, candidate_hash: &str, base_release_id: Option<String>) -> Self {
        let mut f = Self::new(client, authorizer, bot, agent_id, proposal_id, candidate_hash, base_release_id);
        f.approved = true;
        f
    }

    fn target(&self) -> Result<ProposalTarget, RegistryError> {
        let rev = self.client.proposal(&self.proposal_id, &self.bot)?.rev;
        Ok(ProposalTarget { proposal_id: self.proposal_id.clone(), candidate_hash: self.candidate_hash.clone(), expected_revision: rev })
    }

    pub fn approve(&mut self) -> Result<ApprovalReport, RegistryError> {
        let bad = "0".repeat(64);
        let wrong = ProposalTarget { candidate_hash: bad.clone(), ..self.target()? };
        let bad_jws = self.authorizer.authorize("approve", &wrong)?;
        let tamper = self.client.probe_approve(&self.proposal_id, &bad, &bad_jws)?;
        let target = self.target()?;
        let jws = self.authorizer.authorize("approve", &target)?;
        let a = self.client.approve(&target, &self.candidate_hash, &jws)?;
        if a.decision != "approved" || a.candidate_hash != self.candidate_hash {
            return Err(RegistryError::Invariant("approval is not an approval of the frozen candidate".into()));
        }
        let again = self.client.probe_approve(&self.proposal_id, &self.candidate_hash, &jws)?;
        self.approved = true;
        Ok(ApprovalReport {
            approver: a.actor,
            decision: a.decision,
            candidate_hash: a.candidate_hash,
            tamper_refused: tamper.status == 409 && tamper.code.as_deref() == Some("candidate_changed"),
            replay_refused: again.status == 409 && again.code.as_deref() == Some("illegal_transition"),
        })
    }

    pub fn publish(&mut self, idempotency_key: &str) -> Result<Published, RegistryError> {
        if !self.approved {
            return Err(RegistryError::Flow("proposal not approved by Core: nothing to publish".into()));
        }
        let target = self.target()?;
        let jws = self.authorizer.authorize("publish", &target)?;
        let rel = self.client.publish(&target, &jws, idempotency_key)?;
        if self.base_release_id.as_deref() == Some(rel.as_str()) {
            return Err(RegistryError::Invariant("core publish answered the base release: the draft was not published".into()));
        }
        self.published = Some(rel.clone());
        Ok(Published { release_id: rel, alias: "staging" })
    }

    /// Alias read through the registry. `staging` before a publish is refused: it cannot show this proposal's release.
    pub fn alias_read(&self, alias: &str) -> Result<RegAliasState, RegistryError> {
        if alias.trim().eq_ignore_ascii_case("staging") && self.published.is_none() {
            return Err(RegistryError::Flow("alias read before publish: staging cannot show the proposal's release yet".into()));
        }
        self.client.alias(&self.agent_id, alias, &self.bot)
    }
}
