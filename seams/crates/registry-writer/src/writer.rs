//! The delivery flows. Every request passes `guard::allowed` first; every failure becomes a closed `Reason`.
use crate::baseline::{self, Refreshed};
use crate::guard;
use crate::store::{Receipt, ReceiptStore};
use crate::transport::{Reply, Request, Transport, TransportError};
use crate::{Labels, Outcome, PROPOSALS_PER_DAY, Reason, Submission, TITLE_PREFIX};
use core_client::authorizer::Jws;
use reasoning::catalog::{Artifact, Catalog};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// The engine's byte-exact changes through the registry API.
    RegistryApi,
    /// The `pulso-builder` agent run authors and writes the draft.
    BuilderRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// Our own non-shared agent-core instance (`scripts/dev-stack`).
    LocalStack,
    /// The shared agent-core.
    SharedCore,
}

pub struct Config {
    pub via: Via,
    pub environment: Environment,
    /// A principal the REGISTRY API accepts as `builder` (reads and, for `RegistryApi`, writes).
    pub registry_token: Jws,
    /// The engine-signed `builder` principal for `POST /v1/runs` (`BuilderRun` only).
    pub run_token: Option<Jws>,
    /// Compare the live text with the text the patch was compiled against before opening a proposal.
    pub check_base: bool,
    /// Agent of the run (`pulso-builder`).
    pub run_agent: String,
    /// Label of the registry credential (`builder principal` unless a stand-in was used).
    pub credential: &'static str,
}

impl Config {
    pub fn new(via: Via, environment: Environment, registry_token: Jws) -> Config {
        Config { via, environment, registry_token, run_token: None, check_base: true, run_agent: "pulso-builder".into(), credential: "engine builder principal" }
    }
}

type Fail = (Reason, String);

pub struct Writer<'a> {
    cfg: Config,
    transport: &'a dyn Transport,
    store: &'a dyn ReceiptStore,
    clock: Box<dyn Fn() -> u64 + 'a>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Problem code and rule ids of an error body: closed vocabulary, never free text.
fn problem(body: &Value) -> (String, String) {
    let code = body["code"].as_str().filter(|c| c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')).unwrap_or("").to_string();
    let v = body.get("violations").or_else(|| body.get("payload"));
    let rules: Vec<&str> = v.and_then(Value::as_array).into_iter().flatten().filter_map(|x| x["rule"].as_str()).filter(|r| rule_id(r)).take(3).collect();
    (code, rules.join(","))
}

/// A rule id (`REG-SCHEMA`, `limits.size`): short, no spaces, so registry prose never reaches an outcome.
fn rule_id(r: &str) -> bool {
    !r.is_empty() && r.len() <= 40 && r.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn reject(r: &Reply) -> Fail {
    let (code, rules) = problem(&r.body);
    let tail = if rules.is_empty() { String::new() } else { format!(" rules {rules}") };
    let detail = format!("http {} {}{tail}", r.status, if code.is_empty() { "-" } else { &code });
    let reason = match (r.status, code.as_str()) {
        (401, _) => Reason::Unauthorized,
        (_, "step_up_required") => Reason::StepUpRequired,
        (403, _) => Reason::ForbiddenRole,
        (429, _) | (_, "quota_exceeded") => Reason::QuotaExceeded,
        (_, "proposal_stale") => Reason::ProposalStale,
        (422, _) | (_, "validation_failed") => Reason::ValidationFailed,
        _ => Reason::RegistryError,
    };
    (reason, detail)
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Evidence text that survives agent-core's PII tokeniser: digit runs of 6 or more are shortened, nothing else is sent.
fn compact(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 6 {
            out.push_str("#n");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in s.chars() {
        if c.is_ascii_digit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    clip(out.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), max)
}

impl<'a> Writer<'a> {
    pub fn new(cfg: Config, transport: &'a dyn Transport, store: &'a dyn ReceiptStore) -> Writer<'a> {
        Writer { cfg, transport, store, clock: Box::new(now_secs) }
    }

    pub fn with_clock(mut self, clock: impl Fn() -> u64 + 'a) -> Writer<'a> {
        self.clock = Box::new(clock);
        self
    }

    fn labels(&self) -> Labels {
        Labels {
            environment: match self.cfg.environment {
                Environment::LocalStack => "local-stack: our own agent-core instance, not the shared Core",
                Environment::SharedCore => "shared-core",
            },
            via: match self.cfg.via {
                Via::RegistryApi => "registry-api (create_proposal, put_draft, validate)",
                Via::BuilderRun => "builder-run: POST /v1/runs of the pulso-builder agent",
            },
            authored_by: match self.cfg.via {
                Via::RegistryApi => "engine compiler (byte-exact anchored patch or closure copy)",
                Via::BuilderRun => "pulso-builder agent (a model re-authors the draft from the goal and evidence)",
            },
            credential: self.cfg.credential,
        }
    }

    fn outcome(&self, key: &str) -> Outcome {
        Outcome { status: "denied", reason: None, detail: String::new(), proposal_id: None, valid: None, changes: 0, state: None, replayed: false, key: key.into(), labels: self.labels() }
    }

    fn denied(&self, key: &str, (reason, detail): Fail, proposal_id: Option<&str>) -> Outcome {
        let mut o = self.outcome(key);
        o.reason = Some(reason);
        o.detail = clip(&detail, 300);
        o.proposal_id = proposal_id.map(str::to_string);
        o
    }

    /// One guarded request. Non-2xx answers are returned as `Reply` for the caller to interpret (a 404 can mean "not there").
    fn call(&self, method: &str, path: String, bearer: &Jws, idem: Option<&str>, body: Option<Value>) -> Result<Reply, Fail> {
        if !guard::allowed(method, &path) {
            return Err((Reason::ForbiddenOperation, format!("{method} {} is not an operation of the engine", clip(&path, 80))));
        }
        self.transport.send(&Request { method, path, bearer, idempotency_key: idem, body }).map_err(|e| match e {
            TransportError::NotSent(m) => (Reason::RegistryUnreachable, clip(&m, 120)),
            TransportError::Unknown(m) => (Reason::OutcomeUnknown, clip(&m, 120)),
        })
    }

    fn ok_call(&self, method: &str, path: String, bearer: &Jws, idem: Option<&str>, body: Option<Value>) -> Result<Value, Fail> {
        let r = self.call(method, path, bearer, idem, body)?;
        if (200..300).contains(&r.status) { Ok(r.body) } else { Err(reject(&r)) }
    }

    /// Live artifact of a `kind:id` reference.
    pub fn fetch_artifact(&self, target_ref: &str) -> Result<Artifact, Fail> {
        let (kind, id) = target_ref.split_once(':').ok_or((Reason::InvalidSubmission, "target reference is not kind:id".into()))?;
        let path = baseline::entity_path(kind, id).ok_or((Reason::InvalidSubmission, "target reference is not a safe id".into()))?;
        let r = self.call("GET", path, &self.cfg.registry_token, None, None)?;
        if r.status == 404 {
            return Err((Reason::BaseMissing, format!("{target_ref} is not in the live registry")));
        }
        if !(200..300).contains(&r.status) {
            return Err(reject(&r));
        }
        baseline::parse_entity(&r.body, vec![]).ok_or((Reason::RegistryError, "the entity answer has no readable locales".into()))
    }

    /// The catalogue with every artifact the live registry can serve replaced by its live text.
    pub fn refresh_catalog(&self, catalog: &Catalog) -> Refreshed {
        baseline::refresh_with(catalog, &catalog.target_refs(), &|r| self.fetch_artifact(r))
    }

    /// Delivers one compiled proposal. Never panics, never approves.
    pub fn deliver(&self, s: &Submission) -> Outcome {
        let key = s.key();
        let changes = match self.prepare(s, &key) {
            Ok(c) => c,
            Err(f) => return self.denied(&key, f, None),
        };
        // Same key, same proposal: a retry never opens a second one.
        if let Some(rec) = self.store.get(&key) {
            match self.resume(s, &key, &rec, &changes) {
                Ok(Some(o)) => return o,
                Ok(None) => {
                    let _ = self.store.forget(&key); // the registry no longer has it (reset): a fresh delivery
                }
                Err(f) => return self.denied(&key, f, Some(&rec.proposal_id)),
            }
        }
        let since = (self.clock)().saturating_sub(24 * 3600);
        if self.store.created_since(since) >= PROPOSALS_PER_DAY {
            return self.denied(&key, (Reason::QuotaExceeded, format!("local guard: {PROPOSALS_PER_DAY} proposals in the last 24 h")), None);
        }
        match self.cfg.via {
            Via::RegistryApi => self.direct(s, &key, &changes),
            Via::BuilderRun => self.run(s, &key, &changes),
        }
    }

    /// Shape checks and docs normalisation: what the registry would refuse with a 422 is refused here without a byte sent.
    fn prepare(&self, s: &Submission, key: &str) -> Result<Vec<Value>, Fail> {
        if s.kind == "no_change" || s.changes.is_empty() {
            return Err((Reason::NothingToPropose, "the compiled proposal has no changes".into()));
        }
        if !guard::ok_seg(&s.agent_id) {
            return Err((Reason::InvalidSubmission, "agent id is blank or not a safe id".into()));
        }
        let mut out = vec![];
        for (i, c) in s.changes.iter().enumerate() {
            let bad = |m: &str| (Reason::InvalidSubmission, format!("change {i}: {m}"));
            let kind = c["kind"].as_str().filter(|k| !k.is_empty()).ok_or_else(|| bad("no kind"))?;
            if kind == "release_settings" {
                return Err(bad("release_settings is human-owned; the engine never proposes it"));
            }
            let content = c["content"].as_object().ok_or_else(|| bad("content is not an object"))?;
            if !content.get("id").is_some_and(Value::is_string) || !content.get("version").is_some_and(Value::is_string) {
                return Err(bad("content needs id and version as text"));
            }
            let d = &c["docs"];
            let description = d["description"].as_str().filter(|t| !t.is_empty()).map(|t| clip(t, 4000)).unwrap_or_else(|| format!("{TITLE_PREFIX} {kind} change for {}", s.target_ref));
            let rationale = clip(d["rationale"].as_str().unwrap_or(""), 4000);
            let changelog = clip(d["changelog"].as_str().filter(|t| !t.is_empty()).unwrap_or(&format!("{TITLE_PREFIX} proposal key {key}")), 8000);
            out.push(json!({"kind": kind, "content": c["content"], "docs": {"description": description, "rationale": rationale, "changelog": changelog}}));
        }
        Ok(out)
    }

    fn proposal_path(id: &str) -> Result<String, Fail> {
        if guard::ok_seg(id) { Ok(format!("/v1/registry/proposals/{id}")) } else { Err((Reason::RegistryError, "the registry answered an unsafe proposal id".into())) }
    }

    fn read_proposal(&self, id: &str) -> Result<Option<Value>, Fail> {
        let r = self.call("GET", Self::proposal_path(id)?, &self.cfg.registry_token, None, None)?;
        match r.status {
            404 => Ok(None),
            200..=299 => Ok(Some(r.body)),
            401 | 403 => Err((Reason::ReadbackUnavailable, format!("http {}: this credential cannot read the proposal back", r.status))),
            _ => Err(reject(&r)),
        }
    }

    /// A receipt exists: confirm it in the registry and finish what is missing. `Ok(None)` = the registry does not know it.
    fn resume(&self, s: &Submission, key: &str, rec: &Receipt, changes: &[Value]) -> Result<Option<Outcome>, Fail> {
        let Some(detail) = self.read_proposal(&rec.proposal_id)? else { return Ok(None) };
        let stored = detail["changes"].as_array().map_or(0, Vec::len);
        let state = detail["proposal"]["state"].as_str().unwrap_or("").to_string();
        if stored == 0 && self.cfg.via == Via::RegistryApi && state == "draft" {
            let rev = detail["proposal"]["rev"].as_u64().ok_or((Reason::RegistryError, "the proposal has no rev".into()))?;
            return self.write_and_check(s, key, &rec.proposal_id, rev, changes, true).map(Some);
        }
        let mut o = self.check(key, &rec.proposal_id, if self.cfg.via == Via::RegistryApi { changes.len() } else { 0 }, true)?;
        o.replayed = true;
        Ok(Some(o))
    }

    /// The listed proposal of this agent whose title is the deterministic title of the submission (`GET /v1/registry/proposals`).
    fn lookup(&self, s: &Submission, _key: &str) -> Result<Option<String>, Fail> {
        let body = self.ok_call("GET", format!("/v1/registry/proposals?agent_id={}&limit=200", s.agent_id), &self.cfg.registry_token, None, None)?;
        let title = s.title();
        Ok(body["items"].as_array().into_iter().flatten().find(|p| p["title"].as_str() == Some(title.as_str()) && p["state"].as_str() == Some("draft")).and_then(|p| p["proposal_id"].as_str()).filter(|id| guard::ok_seg(id)).map(str::to_string))
    }

    // ---- RegistryApi ------------------------------------------------------------------------------------------------------

    fn direct(&self, s: &Submission, key: &str, changes: &[Value]) -> Outcome {
        if self.cfg.check_base && s.kind == "patch" {
            match self.fetch_artifact(&s.target_ref) {
                Err(f) => return self.denied(key, f, None),
                Ok(a) if a.digest() != s.base_digest => {
                    return self.denied(key, (Reason::BaseChanged, format!("the live {} is at version {} and differs from the text the patch was compiled against", s.target_ref, a.version)), None);
                }
                Ok(_) => {}
            }
        }
        // Secondary lookup: a proposal with this deterministic title may exist although the local receipt was lost (GET /proposals).
        match self.lookup(s, key) {
            Ok(Some(pid)) => {
                let rec = Receipt { proposal_id: pid, agent_id: s.agent_id.clone(), created_at: (self.clock)() };
                let _ = self.store.put(key, rec.clone());
                return match self.resume(s, key, &rec, changes) {
                    Ok(Some(o)) => o,
                    Ok(None) => self.denied(key, (Reason::RegistryError, "the listed proposal vanished".into()), Some(&rec.proposal_id)),
                    Err(f) => self.denied(key, f, Some(&rec.proposal_id)),
                };
            }
            Ok(None) | Err(_) => {} // not found, or a registry without the listing route: the idempotency key still protects the create
        }
        let created = match self.ok_call("POST", "/v1/registry/proposals".into(), &self.cfg.registry_token, Some(key), Some(json!({"agent_id": s.agent_id, "origin": "auto_detect", "title": s.title()}))) {
            Ok(v) => v,
            Err(f) => return self.denied(key, f, None),
        };
        let (Some(pid), Some(rev)) = (created["proposal_id"].as_str(), created["rev"].as_u64()) else {
            return self.denied(key, (Reason::RegistryError, "create answered without proposal_id and rev".into()), None);
        };
        if let Err(e) = self.store.put(key, Receipt { proposal_id: pid.into(), agent_id: s.agent_id.clone(), created_at: (self.clock)() }) {
            return self.denied(key, (Reason::RegistryError, format!("receipt not stored: {}", clip(&e, 80))), Some(pid));
        }
        match self.write_and_check(s, key, pid, rev, changes, false) {
            Ok(o) => o,
            Err(f) => self.denied(key, f, Some(pid)),
        }
    }

    fn write_and_check(&self, _s: &Submission, key: &str, pid: &str, rev: u64, changes: &[Value], replayed: bool) -> Result<Outcome, Fail> {
        let path = format!("{}/draft", Self::proposal_path(pid)?);
        let draft_key = format!("{key}-draft"); // server-side replay of a lost answer (agent-core Idempotency-Key on put_draft)
        self.ok_call("PUT", path, &self.cfg.registry_token, Some(&draft_key), Some(json!({"expected_rev": rev, "changes": changes})))?;
        let mut o = self.check(key, pid, changes.len(), replayed)?;
        o.replayed = replayed;
        Ok(o)
    }

    /// Validate and read back: success = proposal id + valid + non-empty stored changes.
    fn check(&self, key: &str, pid: &str, expected: usize, replayed: bool) -> Result<Outcome, Fail> {
        let v = self.ok_call("POST", format!("{}/validate", Self::proposal_path(pid)?), &self.cfg.registry_token, None, None)?;
        let viol = v["violations"].as_array().ok_or((Reason::RegistryError, "validate answered without violations".into()))?;
        if !viol.is_empty() {
            let rules: Vec<&str> = viol.iter().filter_map(|x| x["rule"].as_str()).filter(|r| rule_id(r)).take(3).collect();
            return Err((Reason::DraftInvalid, format!("{} violations, rules {}", viol.len(), rules.join(","))));
        }
        let detail = self.read_proposal(pid)?.ok_or((Reason::RegistryError, "the proposal vanished after the write".into()))?;
        let stored = detail["changes"].as_array().map_or(0, Vec::len);
        if stored == 0 {
            return Err((Reason::EmptyDraft, "the registry holds a draft without changes".into()));
        }
        if expected > 0 && stored != expected {
            return Err((Reason::EmptyDraft, format!("the registry holds {stored} changes, {expected} were sent")));
        }
        let mut o = self.outcome(key);
        o.status = "delivered";
        o.proposal_id = Some(pid.into());
        o.valid = Some(true);
        o.changes = stored;
        o.state = detail["proposal"]["state"].as_str().map(str::to_string);
        o.replayed = replayed;
        Ok(o)
    }

    // ---- BuilderRun -------------------------------------------------------------------------------------------------------

    fn run(&self, s: &Submission, key: &str, changes: &[Value]) -> Outcome {
        let Some(token) = &self.cfg.run_token else {
            return self.denied(key, (Reason::InvalidSubmission, "the builder-run delivery needs a run token".into()), None);
        };
        if self.cfg.check_base && s.kind == "patch" {
            match self.fetch_artifact(&s.target_ref) {
                Err(f) => return self.denied(key, f, None),
                Ok(a) if a.digest() != s.base_digest => return self.denied(key, (Reason::BaseChanged, format!("the live {} is at version {}", s.target_ref, a.version)), None),
                Ok(_) => {}
            }
        }
        let goal = clip(&format!("{TITLE_PREFIX} {} {}: {}", s.target_ref, &key[6..14], compact(&s.rationale, 120)), 200);
        let evidence = compact(&format!("finding evidence {} target {} kind {} compiled changes {}: {}", s.evidence_ref, s.target_ref, s.kind, changes.len(), s.rationale), 1500);
        let body = json!({"agent": self.cfg.run_agent, "lang": "es", "input": {"agente": s.agent_id, "objetivo": goal, "evidencia": evidence}});
        let reply = match self.call("POST", "/v1/runs".into(), token, Some(key), Some(body)) {
            Ok(r) => r,
            Err(f) => return self.denied(key, f, None),
        };
        if !(200..300).contains(&reply.status) {
            return self.denied(key, reject(&reply), None);
        }
        let outcome = reply.body["outcome"].as_str();
        if outcome.is_some_and(|o| o != "completed") {
            let why = reply.body["reason_code"].as_str().unwrap_or("-");
            return self.denied(key, (Reason::RunNotCompleted, format!("outcome {} reason {}", outcome.unwrap_or("-"), clip(why, 40))), None);
        }
        let out = reply.body.get("output_map").or_else(|| reply.body.get("output"));
        let Some(pid) = out.and_then(|o| o["proposal_id"].as_str()).filter(|p| guard::ok_seg(p)) else {
            return self.denied(key, (Reason::RunMalformed, "the run ended without a proposal_id slot".into()), None);
        };
        let _ = self.store.put(key, Receipt { proposal_id: pid.into(), agent_id: s.agent_id.clone(), created_at: (self.clock)() });
        if out.and_then(|o| o["valid"].as_bool()) != Some(true) {
            return self.denied(key, (Reason::DraftInvalid, "the run reports the draft is not valid".into()), Some(pid));
        }
        // The slots do not show the drafted changes: they are read back from the registry.
        match self.check(key, pid, 0, false) {
            Ok(o) => o,
            Err(f) => self.denied(key, f, Some(pid)),
        }
    }
}
