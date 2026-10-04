//! E2 live wiring: the handlers that take the 5-step `thread` job from the compile step to a published release on the
//! real Core, behind a `CorePort`. The port is the only thing that talks to Core (`live_core::LiveCore` over
//! `core-client`); the handlers hold the engine's rules and are tested offline with a fake port (`tests/live_handlers.rs`).
//!
//! Handler order (`live_handlers`): sensors, recompute, validation, compile (dry-run hook = Core dry-run digest),
//! `arms` (freeze the draft, run base and candidate arms through `run_arm`, feed the reports to the V2 gate),
//! gate (stand-in judge, unchanged), `native_eval` (Core's evaluation, the registry approves only `evaluated`),
//! `authority` (the U21 state machine decides; a gate that is not `pass` needs an explicit LABELLED SIMULATED human
//! override, else `blocked(gate)`), and `publish` (effectful: Core approve with the simulated-human JWS, publish to
//! staging, alias readback must equal the published release).
//!
//! Honest labels: the human is a claude-standin (`simulated=true`), the judge is `claude-standin`, the oracle of the
//! arms is the suite's own `expect` blocks (authored by the suite author), `quality_claims` stay forbidden.
//! `stub_handlers` is kept for runs WITHOUT a Core (offline default: not_exercised / blocked(core)).
use crate::adapters::DryRun;
use abi::*;
use authority::{Authority, Event, GateVerdict, Override, SimTicket, SimulatedIssuer, Target};
use core_client::canon;
use core_client::dto::ArmReport;
use serde_json::{Map, Value, json};
use std::rc::Rc;

// ---------------------------------------------------------------------------------------------------------------
// Port to the Core
// ---------------------------------------------------------------------------------------------------------------

/// A proposal frozen by the writer stage, as carried in the job payload (so a resumed job needs no memory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenInfo {
    pub proposal_id: String,
    /// Bare lowercase hex-64.
    pub candidate_hash: String,
    pub base_release_id: String,
    pub task_binding_ref: String,
    pub plan_ref: String,
    pub title: String,
    pub rev: u64,
    /// The draft version of the world assets (`2.0.0`), the key of the live draft variant.
    pub version: String,
}

impl FrozenInfo {
    pub fn to_json(&self) -> Value {
        json!({"proposal_id": self.proposal_id, "candidate_hash": self.candidate_hash, "base_release_id": self.base_release_id,
            "task_binding_ref": self.task_binding_ref, "plan_ref": self.plan_ref, "title": self.title, "rev": self.rev, "version": self.version})
    }

    pub fn from_json(v: &Value) -> Result<FrozenInfo, String> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("frozen.{k} missing"));
        Ok(FrozenInfo {
            proposal_id: s("proposal_id")?,
            candidate_hash: s("candidate_hash")?,
            base_release_id: s("base_release_id")?,
            task_binding_ref: s("task_binding_ref")?,
            plan_ref: s("plan_ref")?,
            title: s("title")?,
            rev: v.get("rev").and_then(Value::as_u64).ok_or("frozen.rev missing")?,
            version: s("version")?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Base,
    Candidate,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Base => "base",
            Side::Candidate => "cand",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmCall {
    pub key: String,
    pub side: Side,
    pub case_ref: String,
}

/// The sealed evaluation suite: its Core digest (64 hex) and the case ids the arms run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteInfo {
    pub digest: String,
    pub cases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishInfo {
    pub release_id: String,
    /// What the staging alias shows after the publish (the readback).
    pub staging_release_id: String,
}

/// Everything the live handlers need from the Core. Errors are strings: the handler turns them into `HandlerError`.
pub trait CorePort {
    /// Core dry-run of the compiled operations (canonical JSON each): `sha256:<hex>` of the candidate.
    fn dry_run(&self, ops: &[String]) -> Result<String, String>;
    /// Dry-run, seal and freeze the draft through the writer stage (stable key per `job_id`: a replay is the same proposal).
    fn freeze(&self, ops: &[String], job_id: &str) -> Result<FrozenInfo, String>;
    fn suite(&self, f: &FrozenInfo) -> Result<SuiteInfo, String>;
    /// One arm run through `run_arm` with the given idempotency key.
    fn run_arm(&self, f: &FrozenInfo, call: &ArmCall) -> Result<ArmReport, String>;
    /// Core's native evaluation: `pass`, `failed_infra` or `fail`.
    fn evaluate(&self, f: &FrozenInfo, job_id: &str) -> Result<String, String>;
    /// Core approve with the (simulated) human authorization; returns the approving actor.
    fn approve(&self, f: &FrozenInfo) -> Result<String, String>;
    /// Core publish to staging with the idempotency key, then the staging alias readback.
    fn publish(&self, f: &FrozenInfo, idempotency_key: &str) -> Result<PublishInfo, String>;
}

#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// The simulated human (a config value: DEMO-0 rule).
    pub human_actor: String,
    /// Explicit labelled override of a gate that is not `pass`; without it `authority` blocks.
    pub human_override: Option<Override>,
    pub decision_ttl_seconds: u64,
}

/// Idempotency key of an arm run: derived from job, step, executor fence token and attempt (plus the arm side and case).
/// A re-run under a new fence/attempt is a new, distinct evaluation execution; an identical re-send is a Core replay.
pub fn arm_key(job: &str, step: &str, fence: u64, attempt: u64, side: &str, case: &str) -> Result<String, String> {
    for (n, v) in [("job", job), ("step", step), ("side", side), ("case", case)] {
        if v.is_empty() || v.contains('|') || v.contains(char::is_control) {
            return Err(format!("{n} {v:?} cannot take part in an idempotency key"));
        }
    }
    let h = canon::sha256_hex(format!("{job}|{step}|{fence}|{attempt}|{side}|{case}").as_bytes());
    Ok(format!("arm-{}", &h[..48]))
}

/// Stable key of the publish (no fence/attempt: a retried publish is the same operation).
fn publish_key(job: &str) -> String {
    format!("pub-{}", &canon::sha256_hex(format!("{job}|publish").as_bytes())[..48])
}

/// The compile step's dry-run hook: the Core digest becomes the plan digest. The hook cannot fail by type, so a Core
/// failure becomes an unmistakably malformed digest that the compile step rejects with the reason in its message.
pub fn dry_run_hook(port: Rc<dyn CorePort>) -> DryRun {
    Box::new(move |ops: &[String]| match port.dry_run(ops) {
        Ok(d) => d,
        Err(e) => format!("core-dry-run-failed: {}", e.chars().take(200).collect::<String>()),
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Payload helpers
// ---------------------------------------------------------------------------------------------------------------

fn bad(m: impl Into<String>) -> HandlerError {
    HandlerError::Invalid(m.into())
}

fn failed(m: impl Into<String>) -> HandlerError {
    HandlerError::Failed(m.into())
}

fn parse(input: &InputEnvelope) -> Result<Value, HandlerError> {
    serde_json::from_str(&input.payload).map_err(|e| bad(format!("payload: {e}")))
}

fn out_of<'a>(p: &'a Value, step: &str) -> Result<&'a Value, HandlerError> {
    p.get("out").and_then(|o| o.get(step)).ok_or_else(|| bad(format!("out.{step} missing: the {step} step did not run before this one")))
}

fn put_out(p: &mut Value, step: &str, v: Value) -> Result<(), HandlerError> {
    p.get_mut("out").and_then(Value::as_object_mut).ok_or_else(|| bad("payload has no out object"))?.insert(step.into(), v);
    Ok(())
}

fn finish(p: Value, event: String, effect: EffectState) -> Result<OutputEnvelope, HandlerError> {
    Ok(OutputEnvelope { payload: serde_json::to_string(&p).map_err(|e| bad(e.to_string()))?, events: vec![event], effect })
}

fn frozen_of(p: &Value) -> Result<FrozenInfo, HandlerError> {
    FrozenInfo::from_json(out_of(p, "arms")?.get("frozen").ok_or_else(|| bad("out.arms.frozen missing"))?).map_err(bad)
}

// ---------------------------------------------------------------------------------------------------------------
// arms
// ---------------------------------------------------------------------------------------------------------------

/// The oracle of the arms is the suite's own `expect` blocks, authored by the suite author (not by Claude, not by the judge).
const SUITE_ORACLE: &str = "oracle:suite-expect@handwritten";

struct Arms {
    port: Rc<dyn CorePort>,
}

impl JobHandler for Arms {
    fn id(&self) -> HandlerId {
        HandlerId("arms".into())
    }

    /// Not effectful in the C-7 sense: the freeze is keyed by the job (a replay is the same proposal) and every arm key
    /// carries fence and attempt, so a re-run is a distinct evaluation execution, never a duplicate business effect.
    fn run(&self, fence: &Fence, input: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        let mut p = parse(input)?;
        let compile = out_of(&p, "compile")?;
        let plan = compile.get("draft_plan").ok_or_else(|| bad("out.compile.draft_plan missing (compile was denied?)"))?;
        let digest = plan.get("digest").and_then(Value::as_str).ok_or_else(|| bad("draft_plan.digest missing"))?.to_string();
        let ops: Vec<String> = plan.get("operations").and_then(Value::as_array).ok_or_else(|| bad("draft_plan.operations missing"))?.iter().map(|o| o.to_string()).collect();
        let frozen = self.port.freeze(&ops, &input.job_id).map_err(|e| failed(format!("freeze: {e}")))?;
        if format!("sha256:{}", frozen.candidate_hash) != digest {
            return Err(failed(format!("the frozen candidate_hash {} is not the compiled draft_plan digest {digest}", frozen.candidate_hash)));
        }
        let suite = self.port.suite(&frozen).map_err(|e| failed(format!("suite: {e}")))?;
        let mut sides: [Vec<ArmReport>; 2] = [vec![], vec![]];
        let mut rows = Vec::new();
        for (idx, side) in [Side::Base, Side::Candidate].into_iter().enumerate() {
            for case in &suite.cases {
                let key = arm_key(&input.job_id, "arms", fence.fence_token, fence.attempt, side.as_str(), case).map_err(bad)?;
                let mut r = self.port.run_arm(&frozen, &ArmCall { key: key.clone(), side, case_ref: case.clone() }).map_err(|e| failed(format!("arm {}/{case}: {e}", side.as_str())))?;
                if !r.is_completed() && r.raw.get("status").and_then(Value::as_str) != Some("candidate_failed") {
                    return Err(failed(format!("arm {}/{case} ended {}: not evidence ({:?})", side.as_str(), r.raw["status"].as_str().unwrap_or("?"), r.reason)));
                }
                rows.push(json!({"side": side.as_str(), "case": case, "key": key, "execution_id": r.execution_id, "status": r.raw["status"]}));
                if r.oracle_ref.is_none() {
                    r.oracle_ref = Some(SUITE_ORACLE.into());
                }
                sides[idx].push(r);
            }
        }
        // the V2 gate consumes the live reports: replace spec.gate with the gate step's exact input over them
        let g = p.get("spec").and_then(|s| s.get("gate")).ok_or_else(|| bad("spec.gate missing"))?;
        let gi = g.get("gate_in").ok_or_else(|| bad("spec.gate.gate_in missing"))?;
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| bad(format!("spec.gate {k} missing")));
        let (run_id, judge) = (s(gi, "run_id")?, s(gi, "judge_actor")?);
        let authors: Vec<String> = gi.get("author_actors").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let wa = g.get("world_authors").ok_or_else(|| bad("spec.gate.world_authors missing"))?;
        let (world_author, suite_author) = (s(wa, "world")?, s(wa, "suite")?);
        let env = eval::gate::gate_env(&eval::gate::GateInput {
            suite_digest: &suite.digest,
            run_id: &run_id,
            judge_actor: &judge,
            author_actors: authors.iter().map(String::as_str).collect(),
            world_author: &world_author,
            suite_author: &suite_author,
            base: &sides[0],
            candidate: &sides[1],
        })
        .map_err(failed)?;
        p.get_mut("spec").and_then(Value::as_object_mut).ok_or_else(|| bad("spec"))?.insert("gate".into(), env);
        let completed = rows.iter().filter(|r| r["status"] == "completed").count();
        let event = format!("thread:arms:runs={},completed={completed}:semantics=core-arms-real,oracle=suite-expect,gate_reports=live", rows.len());
        put_out(&mut p, "arms", json!({"frozen": frozen.to_json(), "suite_digest": suite.digest, "runs": rows}))?;
        finish(p, event, EffectState::NoEffect)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// native_eval
// ---------------------------------------------------------------------------------------------------------------

struct NativeEval {
    port: Rc<dyn CorePort>,
}

impl JobHandler for NativeEval {
    fn id(&self) -> HandlerId {
        HandlerId("native_eval".into())
    }

    /// Replay-safe (admission and stage keys derive from the job): not effectful in the C-7 sense.
    fn run(&self, _fence: &Fence, input: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        let mut p = parse(input)?;
        let frozen = frozen_of(&p)?;
        let verdict = self.port.evaluate(&frozen, &input.job_id).map_err(|e| failed(format!("native evaluation: {e}")))?;
        if verdict != "pass" {
            return Err(failed(format!("blocked: native evaluation verdict {verdict}: the registry approves only an evaluated proposal")));
        }
        put_out(&mut p, "native_eval", json!({"verdict": verdict}))?;
        finish(p, format!("thread:native_eval:verdict={verdict}:semantics=core-real"), EffectState::NoEffect)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// authority
// ---------------------------------------------------------------------------------------------------------------

struct AuthorityStep {
    cfg: LiveConfig,
}

fn gate_verdict(s: &str) -> Option<GateVerdict> {
    match s {
        "pass" => Some(GateVerdict::Pass),
        "fail" => Some(GateVerdict::Fail),
        "not_evaluable" => Some(GateVerdict::NotEvaluable),
        _ => None,
    }
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl JobHandler for AuthorityStep {
    fn id(&self) -> HandlerId {
        HandlerId("authority".into())
    }

    /// The U21 state machine decides before any Core effect: a gate that is not `pass` is `blocked(gate)` unless the
    /// configured human override is complete (by=human, actor, reason). The "human" is SIMULATED (DEMO-0).
    fn run(&self, _fence: &Fence, input: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        let mut p = parse(input)?;
        let frozen = frozen_of(&p)?;
        out_of(&p, "native_eval")?;
        let gate = out_of(&p, "gate")?;
        let verdict = gate.get("verdict").and_then(Value::as_str).ok_or_else(|| bad("out.gate.verdict missing"))?.to_string();
        let gates = gate.get("gates").and_then(Value::as_array).ok_or_else(|| bad("out.gate.gates missing"))?;
        let st = |name: &str| -> Result<GateVerdict, HandlerError> {
            let g = gates.iter().find(|g| g.get("gate").and_then(Value::as_str) == Some(name)).ok_or_else(|| bad(format!("gate {name} missing")))?;
            gate_verdict(g.get("status").and_then(Value::as_str).unwrap_or("")).ok_or_else(|| bad(format!("gate {name} status unknown")))
        };
        let (safety, improvement) = (st("safety")?, st("improvement")?);
        let now = now_s();
        let target = Target { proposal_ref: frozen.proposal_id.clone(), proposal_rev: frozen.rev, candidate_hash: frozen.candidate_hash.clone() };
        let issuer = SimulatedIssuer::default().with("sim-ticket", SimTicket::approver(&self.cfg.human_actor, now + self.cfg.decision_ttl_seconds + 60));
        let mut a = Authority::new(target.clone());
        let decision_id = format!("dec-{}-{}", input.job_id, frozen.proposal_id);
        let mut step = |ev: Event| a.apply(ev, now, &issuer).map_err(|e| failed(format!("authority: {e:?}")));
        step(Event::StartEvaluation)?;
        step(Event::RecordGates { safety, improvement })?;
        step(Event::RequestDecision { decision_id: decision_id.clone(), expires_at: now + self.cfg.decision_ttl_seconds })?;
        let both_pass = safety == GateVerdict::Pass && improvement == GateVerdict::Pass;
        let ov = if both_pass { None } else { self.cfg.human_override.clone() };
        match a.apply(Event::Approve { decision_id: decision_id.clone(), target, ticket: "sim-ticket".into(), override_: ov.clone() }, now, &issuer) {
            Ok(_) => {}
            Err(authority::AuthorityError::GateFailed) => {
                return Err(failed(format!("blocked(gate): gate verdict {verdict} is not pass and no complete explicit human override was given (simulated human, by=human, actor, reason)")));
            }
            Err(e) => return Err(failed(format!("authority: {e:?}"))),
        }
        let ap = a.approval().ok_or_else(|| failed("authority approved without an approval record"))?;
        let override_json = match &ov {
            Some(o) => json!({"label": "human_override", "by": o.by, "actor": o.actor.trim(), "reason": o.reason.trim(), "simulated": true, "of_gate_verdict": verdict}),
            None => Value::Null,
        };
        let event = format!(
            "thread:authority:state=approved,gate={verdict},override={},simulated=true,quality_claims={}:semantics=claude-standin(simulated-human)",
            if ov.is_some() { "human_override" } else { "none" },
            if ap.quality_claims_forbidden { "forbidden" } else { "forbidden-by-default" }
        );
        let mut rec = Map::new();
        rec.insert("state".into(), json!("approved"));
        rec.insert("decision_id".into(), json!(ap.decision_id));
        rec.insert("actor".into(), json!(ap.actor_ref));
        rec.insert("simulated".into(), json!(true));
        rec.insert("override".into(), override_json);
        put_out(&mut p, "authority", Value::Object(rec))?;
        finish(p, event, EffectState::NoEffect)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// publish
// ---------------------------------------------------------------------------------------------------------------

struct Publish {
    port: Rc<dyn CorePort>,
}

impl JobHandler for Publish {
    fn id(&self) -> HandlerId {
        HandlerId("publish".into())
    }

    /// EFFECTFUL (C-7): the executor records the dispatch before this runs and never re-runs it after an unacknowledged
    /// crash; a publish to staging is immutable in the registry.
    fn effectful(&self) -> bool {
        true
    }

    fn run(&self, _fence: &Fence, input: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        let mut p = parse(input)?;
        let frozen = frozen_of(&p)?;
        let auth = out_of(&p, "authority")?;
        if auth.get("state").and_then(Value::as_str) != Some("approved") {
            return Err(bad("out.authority is not approved"));
        }
        let simulated = auth.get("simulated").and_then(Value::as_bool).unwrap_or(false);
        let over = auth.get("override").filter(|o| !o.is_null()).map(|o| o["label"].as_str().unwrap_or("").to_string());
        let approver = self.port.approve(&frozen).map_err(|e| failed(format!("approve: {e}")))?;
        let info = self.port.publish(&frozen, &publish_key(&input.job_id)).map_err(|e| failed(format!("publish: {e}")))?;
        if info.staging_release_id != info.release_id {
            return Err(failed(format!("alias readback: staging shows {:?}, the publish answered {:?}", info.staging_release_id, info.release_id)));
        }
        if info.release_id == frozen.base_release_id {
            return Err(failed("the published release is the base release: the draft was not published"));
        }
        let event = format!(
            "thread:publish:release={},alias=staging,readback=equal,override={},simulated_human={simulated}:semantics=core-real+claude-standin(simulated-human)",
            info.release_id,
            over.as_deref().unwrap_or("none")
        );
        put_out(&mut p, "publish", json!({"release_id": info.release_id, "alias": "staging", "readback_release_id": info.staging_release_id, "approver": approver, "simulated_human": simulated, "override": over}))?;
        finish(p, event, EffectState::AppliedAcknowledged)
    }
}

/// The nine live handlers: `five` are the thread step handlers in pipeline order (sensors, recompute, validation,
/// compile, gate); `arms` goes between compile and gate, and `native_eval`, `authority`, `publish` follow the gate.
pub fn live_handlers(five: Vec<Box<dyn JobHandler>>, port: Rc<dyn CorePort>, cfg: LiveConfig) -> Vec<Box<dyn JobHandler>> {
    assert_eq!(five.len(), 5, "live_handlers takes the five thread step handlers");
    let mut out: Vec<Box<dyn JobHandler>> = Vec::new();
    for (i, h) in five.into_iter().enumerate() {
        if i == 4 {
            out.push(Box::new(Arms { port: port.clone() }));
        }
        out.push(h);
    }
    out.push(Box::new(NativeEval { port: port.clone() }));
    out.push(Box::new(AuthorityStep { cfg }));
    out.push(Box::new(Publish { port }));
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Offline stubs (no Core)
// ---------------------------------------------------------------------------------------------------------------

struct Stub {
    id: &'static str,
    event: &'static str,
}

impl JobHandler for Stub {
    fn id(&self) -> HandlerId {
        HandlerId(self.id.into())
    }
    fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        // payload passes through unchanged: the stub produced nothing and says so
        Ok(OutputEnvelope { payload: i.payload.clone(), events: vec![self.event.into()], effect: EffectState::NoEffect })
    }
}

/// OFFLINE default (no Core reachable): `arms` (not exercised) then `publish` (blocked on core), appended after the five
/// step handlers. The live path is `live_handlers`.
pub fn stub_handlers() -> Vec<Box<dyn JobHandler>> {
    vec![
        Box::new(Stub { id: "arms", event: "thread:arms:not_exercised:needs=core-client(K3)" }),
        Box::new(Stub { id: "publish", event: "thread:publish:blocked(core):needs=core-client(K3)+INT" }),
    ]
}
