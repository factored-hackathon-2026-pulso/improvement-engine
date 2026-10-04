//! K3: the compile writer path. A draft plan (typed `Change`s, integers and strings only) is dry-run, sealed as an
//! artifact, committed through a typed `RegistryMutationCommitment` and executed by the Core `pulso-writer` stage
//! (a projection of sealed writes: no model, no HTTP registry route). The stage result must EQUAL the sealed
//! commitment: any difference is `OpError::CommitmentMismatch`. Mirrors `e2e-core/.../core_hooks.py` `RealCore.freeze`.
//!
//! Out of scope here (needs the live stack and Core's `/v1/registry`, not an `/internal/v1` route): approve and publish
//! with a human-issued JWS. `check_alias_readback` verifies the staging alias after a publish.
use crate::authoring::{Alias, AliasState, Change, DryRunRequest};
use crate::canon::{self, CanonError};
use crate::client::CoreClient;
use crate::dto::{DecodeError, Fact, Stage, TaskInvocation};
use crate::ops::OpError;
use serde_json::{Map, Value, json};

pub const WRITER_AGENT: &str = "pulso-writer";
pub const CREATE_ORIGIN: &str = "builder_chat";

/// The registry writes a writer stage may commit, in the order the commitment seals them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterOp {
    CreateProposal,
    PutDraft,
    Freeze,
}

impl WriterOp {
    pub fn as_str(self) -> &'static str {
        match self {
            WriterOp::CreateProposal => "create_proposal",
            WriterOp::PutDraft => "put_draft",
            WriterOp::Freeze => "freeze",
        }
    }
}

/// The only write sequence of the K3 path.
pub const WRITE_OPS: [WriterOp; 3] = [WriterOp::CreateProposal, WriterOp::PutDraft, WriterOp::Freeze];

fn changes_json(changes: &[Change]) -> Value {
    Value::Array(changes.iter().map(|c| json!({"kind": c.kind, "content": c.content, "docs": c.docs})).collect())
}

/// The draft plan: what the writer stage reads from the sealed artifact `draft_plan_ref`.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftPlan {
    pub agent_id: String,
    pub title: String,
    pub changes: Vec<Change>,
}

impl DraftPlan {
    pub fn new(agent_id: &str, title: &str, changes: Vec<Change>) -> DraftPlan {
        DraftPlan { agent_id: agent_id.into(), title: title.into(), changes }
    }

    /// Integers and strings only: Core and the draft digest canonicalise non-integer numbers differently, which made
    /// the put_draft commitment be denied on the real image (THREAD01). Also the dry-run shape rules.
    pub fn validate(&self) -> Result<(), String> {
        if self.title.is_empty() {
            return Err("title is empty".into());
        }
        if self.changes.is_empty() {
            return Err("a draft plan needs at least one change".into());
        }
        DryRunRequest::new("t", &self.agent_id, None, self.changes.clone()).validate()
    }

    /// The artifact content sealed under `draft_plan_ref`.
    pub fn artifact(&self) -> Value {
        json!({"agent_id": self.agent_id, "title": self.title, "changes": changes_json(&self.changes)})
    }

    /// `digest_json(artifact)`.
    pub fn artifact_digest(&self) -> Result<String, CanonError> {
        canon::digest_json(&self.artifact())
    }

    /// `put_draft_digest(None, None, changes)`: `digest_json({"proposal_id": null, "expected_rev": null, "changes": ...})`.
    pub fn put_draft_digest(&self) -> Result<String, CanonError> {
        canon::digest_json(&json!({"proposal_id": null, "expected_rev": null, "changes": changes_json(&self.changes)}))
    }
}

/// Uploads a draft-plan artifact where the writer stage can read it (the broker). This is NOT an `/internal/v1`
/// route (the stand-in uses the double's `configure(artifacts=...)` hook), so the transport stays pluggable.
pub trait ArtifactSealer {
    fn seal(&self, plan_ref: &str, content: &Value) -> Result<(), String>;
}

/// `RegistryMutationCommitment`, mode `write`.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteCommitment {
    pub base_release_id: Option<String>,
    pub create_agent_id: String,
    pub create_origin: String,
    pub create_title: String,
    pub put_draft_digest: String,
    pub operations: Vec<WriterOp>,
}

impl WriteCommitment {
    pub fn for_plan(plan: &DraftPlan, base_release_id: Option<&str>) -> Result<WriteCommitment, CanonError> {
        Ok(WriteCommitment {
            base_release_id: base_release_id.map(str::to_string),
            create_agent_id: plan.agent_id.clone(),
            create_origin: CREATE_ORIGIN.into(),
            create_title: plan.title.clone(),
            put_draft_digest: plan.put_draft_digest()?,
            operations: WRITE_OPS.to_vec(),
        })
    }

    /// The exact wire object of the goldens (`mode: write`; no `evaluate_enabled`, no proposal fields).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("mode".into(), json!("write"));
        if let Some(b) = &self.base_release_id {
            m.insert("base_release_id".into(), json!(b));
        }
        m.insert("create_agent_id".into(), json!(self.create_agent_id));
        m.insert("create_origin".into(), json!(self.create_origin));
        m.insert("create_title".into(), json!(self.create_title));
        m.insert("put_draft_digest".into(), json!(self.put_draft_digest));
        m.insert("operations".into(), Value::Array(self.operations.iter().map(|o| json!(o.as_str())).collect()));
        Value::Object(m)
    }
}

/// `RegistryMutationCommitment`, mode `evaluate_only` (no registry writes: native evaluation of a frozen proposal).
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluateOnlyCommitment {
    pub base_release_id: Option<String>,
    pub proposal_id: String,
    pub evaluation_context_ref: String,
}

impl EvaluateOnlyCommitment {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("mode".into(), json!("evaluate_only"));
        if let Some(b) = &self.base_release_id {
            m.insert("base_release_id".into(), json!(b));
        }
        m.insert("evaluate_enabled".into(), json!(true));
        m.insert("evaluation_context_ref".into(), json!(self.evaluation_context_ref));
        m.insert("proposal_id".into(), json!(self.proposal_id));
        m.insert("operations".into(), json!([]));
        Value::Object(m)
    }
}

/// The writer stage `input` slots. `proposal_id` is always present (null before the proposal exists).
#[derive(Debug, Clone, PartialEq)]
pub struct WriterInput {
    pub draft_plan_ref: String,
    pub base_release_id: Option<String>,
    pub evaluate_enabled: bool,
    pub proposal_id: Option<String>,
    /// `(suite id, suite version)` for the evaluate-only stage.
    pub evaluation_suite: Option<(String, String)>,
}

impl WriterInput {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("draft_plan_ref".into(), json!(self.draft_plan_ref));
        if let Some(b) = &self.base_release_id {
            m.insert("base_release_id".into(), json!(b));
        }
        m.insert("evaluate_enabled".into(), json!(self.evaluate_enabled));
        m.insert("proposal_id".into(), self.proposal_id.clone().map_or(Value::Null, Value::String));
        if let Some((id, ver)) = &self.evaluation_suite {
            m.insert("evaluation_suite_id".into(), json!(id));
            m.insert("evaluation_suite_version".into(), json!(ver));
        }
        Value::Object(m)
    }
}

/// One verified registry write the stage reports.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteReceipt {
    pub op: String,
    pub verified: bool,
    pub rev_after: Option<i64>,
    pub request_hash: Option<String>,
}

/// The `pulso_writer_receipts` fact of a writer run.
#[derive(Debug, Clone, PartialEq)]
pub struct WriterReceipts {
    pub state: String,
    pub proposal_id: Option<String>,
    pub candidate_hash: Option<String>,
    pub rev: Option<i64>,
    pub write_receipts: Vec<WriteReceipt>,
    pub native_evaluation: Option<Value>,
}

impl WriterReceipts {
    pub fn from_fact(f: &Fact) -> Result<WriterReceipts, DecodeError> {
        let m = f.value.as_object().ok_or_else(|| DecodeError("pulso_writer_receipts: not an object".into()))?;
        let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
        let mut write_receipts = Vec::new();
        for r in m.get("write_receipts").and_then(Value::as_array).ok_or_else(|| DecodeError("pulso_writer_receipts: missing write_receipts".into()))? {
            let o = r.as_object().ok_or_else(|| DecodeError("write receipt: not an object".into()))?;
            write_receipts.push(WriteReceipt {
                op: o.get("op").and_then(Value::as_str).ok_or_else(|| DecodeError("write receipt: missing op".into()))?.to_string(),
                verified: o.get("verified").and_then(Value::as_bool).unwrap_or(false),
                rev_after: o.get("rev_after").and_then(Value::as_i64),
                request_hash: o.get("request_hash").and_then(Value::as_str).map(str::to_string),
            });
        }
        Ok(WriterReceipts {
            state: s("state").unwrap_or_default(),
            proposal_id: s("proposal_id"),
            candidate_hash: s("candidate_hash"),
            rev: m.get("rev").and_then(Value::as_i64),
            write_receipts,
            native_evaluation: m.get("native_evaluation").filter(|v| !v.is_null()).cloned(),
        })
    }

    /// The stage result must equal the sealed commitment: the committed operations, each verified, in order and no
    /// others; a confirmed state; a proposal; and the frozen `candidate_hash` equal to the dry-run digest.
    pub fn verify_against(&self, c: &WriteCommitment, dry_run_candidate_hash: &str, allow_placeholder: bool) -> Result<(), OpError> {
        let bad = |why: String| Err(OpError::CommitmentMismatch(why));
        if self.state != "confirmed" {
            return bad(format!("writer state {:?} is not \"confirmed\"", self.state));
        }
        let got: Vec<&str> = self.write_receipts.iter().map(|r| r.op.as_str()).collect();
        let want: Vec<&str> = c.operations.iter().map(|o| o.as_str()).collect();
        if got != want {
            return bad(format!("committed operations {want:?} but the stage wrote {got:?}"));
        }
        if let Some(r) = self.write_receipts.iter().find(|r| !r.verified) {
            return bad(format!("write {:?} is not verified", r.op));
        }
        if self.proposal_id.as_deref().is_none_or(str::is_empty) {
            return bad("the stage reported no proposal_id".into());
        }
        if self.native_evaluation.is_some() {
            return bad("the stage reports an evaluation that the write commitment does not contain".into());
        }
        match self.candidate_hash.as_deref() {
            Some(h) if h == dry_run_candidate_hash => {}
            Some(h) if allow_placeholder && h.starts_with('<') && h.ends_with('>') => {}
            other => return bad(format!("frozen candidate_hash {other:?} differs from the dry-run digest {dry_run_candidate_hash}")),
        }
        Ok(())
    }
}

/// Parameters of one writer-stage run.
#[derive(Debug, Clone, PartialEq)]
pub struct WriterRun {
    pub tenant_id: String,
    pub job_id: String,
    pub logical_key: String,
    pub pulso_run_ref: String,
    pub lab_grant_ref: String,
    /// The `pulso-writer` release id of the bridge.
    pub writer_release_id: String,
    pub writer_agent_version: String,
}

/// A proposal frozen by the writer stage, bound to the dry-run digest.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenProposal {
    pub proposal_id: String,
    /// Bare lowercase hex-64: equal to the dry-run `candidate_hash`.
    pub candidate_hash: String,
    pub base_release_id: Option<String>,
    pub task_binding_ref: String,
    pub plan_ref: String,
    pub title: String,
    pub release_id_preview: Option<String>,
    pub commitment: WriteCommitment,
}

impl CoreClient {
    /// Dry-run, seal, commit and execute the writer stage for `plan` against `base_release_id`, and return the frozen
    /// proposal only when the stage result equals the sealed commitment.
    pub fn freeze_draft(&self, sealer: &dyn ArtifactSealer, plan: &DraftPlan, run: &WriterRun, base_release_id: Option<&str>) -> Result<FrozenProposal, OpError> {
        plan.validate().map_err(OpError::Invalid)?;
        let dry = self.dry_run(&run.job_id, &DryRunRequest::new(&run.tenant_id, &plan.agent_id, base_release_id, plan.changes.clone()))?;
        let digest = dry.candidate_hash.clone().expect("dry_run guarantees a candidate_hash");
        let commitment = WriteCommitment::for_plan(plan, base_release_id)?;
        let plan_ref = format!("plan-{}", &plan.artifact_digest()?[..24]);
        sealer.seal(&plan_ref, &plan.artifact()).map_err(|e| OpError::Invalid(format!("sealing the draft plan: {e}")))?;
        let mut inv = TaskInvocation::new(&run.tenant_id, &run.job_id, Stage::Writer, &run.logical_key);
        inv.agent_id = WRITER_AGENT.into();
        inv.agent_version = run.writer_agent_version.clone();
        inv.release_id = run.writer_release_id.clone();
        inv.pulso_run_ref = run.pulso_run_ref.clone();
        inv.lab_grant_ref = run.lab_grant_ref.clone();
        inv.input = WriterInput { draft_plan_ref: plan_ref.clone(), base_release_id: base_release_id.map(str::to_string), evaluate_enabled: false, proposal_id: None, evaluation_suite: None }.to_json();
        inv.registry_mutation_commitment = Some(commitment.to_json());
        let receipt = self.invoke(&inv)?;
        if !receipt.is_success() {
            return Err(OpError::CommitmentMismatch(format!("writer stage did not complete: state {:?}, outcome {:?}, reason {:?}, code {:?}", receipt.state, receipt.outcome, receipt.reason, receipt.code)));
        }
        let fact = receipt.fact("pulso_writer_receipts").ok_or_else(|| OpError::CommitmentMismatch("no pulso_writer_receipts fact".into()))?;
        let wr = WriterReceipts::from_fact(fact)?;
        wr.verify_against(&commitment, &digest, self.placeholders_ok())?;
        Ok(FrozenProposal {
            proposal_id: wr.proposal_id.clone().unwrap_or_default(),
            candidate_hash: digest,
            base_release_id: base_release_id.map(str::to_string),
            task_binding_ref: receipt.task_binding_ref,
            plan_ref,
            title: plan.title.clone(),
            release_id_preview: dry.release_id_preview,
            commitment,
        })
    }
}

/// Readback after a publish: the alias must show exactly the published release, which must not be the base.
pub fn check_alias_readback(alias: &AliasState, published_release_id: &str, base_release_id: Option<&str>) -> Result<(), OpError> {
    let bad = |why: String| Err(OpError::CommitmentMismatch(why));
    if alias.alias != Alias::Staging {
        return bad(format!("readback of alias {:?}: a publish lands on staging only", alias.alias.as_str()));
    }
    if base_release_id == Some(published_release_id) {
        return bad("the published release is the base release: the draft was not published".into());
    }
    match alias.release_id.as_deref() {
        Some(r) if r == published_release_id => Ok(()),
        other => bad(format!("staging shows {other:?}, the publish answered {published_release_id:?}")),
    }
}
