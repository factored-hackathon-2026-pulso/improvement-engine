//! `LiveCore`: the real `CorePort` over `core-client` (bridge `/internal/v1` + Core `/v1/registry`). Nothing here is
//! exercised by the offline tests; `tests/live_thread.rs` (ignored) runs it against the real image.
//!
//! Stand-ins, labelled: the platform that seals the draft artifact and issues the writer/evaluation bindings is the e2e
//! fixtures double (`Platform`, BRG1 gap candidate); the human is the claude-standin `LocalSimAuthorizer`
//! (`auth.simulated=true`); the world assets (prompt, eval suite) come from the generated fixture JSON of the seeded
//! `attention-task` world (`LiveWorld`), keyed by the draft version the compile step names (`...@2` -> `2.0.0`).
use crate::live::{ArmCall, CorePort, FrozenInfo, PublishInfo, Side, SuiteInfo};
use core_client::authorizer::{Jws, LocalSimAuthorizer};
use core_client::authoring::{Alias, Change, CredentialRequest, DryRunRequest};
use core_client::canon;
use core_client::client::CallError;
use core_client::dto::ArmReport;
use core_client::evaluate::{BindingPreauthorizer, EvaluationRun, SuiteRef};
use core_client::registry::{RegistryClient, RegistryFlow};
use core_client::writer::{ArtifactSealer, DraftPlan, FrozenProposal, WriteCommitment, WriterRun};
use core_client::{CoreClient, OpError};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// What the platform stand-in must do for the Core: hold sealed artifacts and issue bindings.
pub trait Platform: ArtifactSealer + BindingPreauthorizer {}
impl<T: ArtifactSealer + BindingPreauthorizer> Platform for T {}

/// One draft variant of the live world (assets re-versioned), from `gen_live_world_fixture.py`.
#[derive(Debug, Clone)]
pub struct Draft {
    pub changes: Vec<Change>,
    pub suite: SuiteRef,
    pub draft_plan_digest: String,
    pub scenarios: Vec<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct LiveWorld {
    drafts: BTreeMap<String, Draft>,
}

impl LiveWorld {
    /// `variants` of the fixture, keyed by draft version.
    pub fn from_fixture(json: &str) -> Result<LiveWorld, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut drafts = BTreeMap::new();
        for (_, d) in v["variants"].as_object().ok_or("fixture has no variants")? {
            let s = |k: &str| d[k].as_str().map(str::to_string).ok_or_else(|| format!("variant.{k} missing"));
            let changes: Vec<Change> = d["changes"].as_array().ok_or("variant.changes missing")?.iter().map(|c| Change::new(c["kind"].as_str().unwrap_or(""), c["content"].clone(), c["docs"].clone())).collect();
            let scenarios = d["changes"].as_array().and_then(|cs| cs.iter().find(|c| c["kind"] == "eval_suite")).and_then(|c| c["content"]["scenarios"].as_array().cloned()).ok_or("variant has no eval suite scenarios")?;
            drafts.insert(
                s("version")?,
                Draft { changes, suite: SuiteRef { id: s("suite_id")?, version: s("suite_version")?, digest: s("suite_digest")? }, draft_plan_digest: s("draft_plan_digest")?, scenarios },
            );
        }
        Ok(LiveWorld { drafts })
    }

    fn draft(&self, version: &str) -> Result<&Draft, String> {
        self.drafts.get(version).ok_or_else(|| format!("no live draft variant for version {version} (have {:?})", self.drafts.keys().collect::<Vec<_>>()))
    }

    /// The draft version the compiled operations name: every `new_ref` must be `kind:name@<major>` with the SAME major.
    pub fn version_of(ops: &[String]) -> Result<String, String> {
        let mut majors = std::collections::BTreeSet::new();
        for o in ops {
            let v: Value = serde_json::from_str(o).map_err(|e| format!("operation is not JSON: {e}"))?;
            let r = v["new_ref"].as_str().ok_or("operation without new_ref")?;
            let major = r.rsplit_once('@').map(|(_, m)| m).filter(|m| !m.is_empty() && m.bytes().all(|b| b.is_ascii_digit())).ok_or_else(|| format!("new_ref {r:?} has no @major"))?;
            majors.insert(major.to_string());
        }
        match majors.len() {
            1 => Ok(format!("{}.0.0", majors.into_iter().next().unwrap())),
            n => Err(format!("operations name {n} different draft versions")),
        }
    }
}

pub struct LiveCoreConfig {
    pub tenant: String,
    pub agent_id: String,
    pub writer_release_id: String,
    pub budget_ref: String,
    pub world: LiveWorld,
}

pub struct LiveCore {
    client: CoreClient,
    registry: RegistryClient,
    auth: LocalSimAuthorizer,
    platform: Box<dyn Platform>,
    cfg: LiveCoreConfig,
    seq: AtomicU64,
}

impl LiveCore {
    pub fn new(client: CoreClient, registry: RegistryClient, auth: LocalSimAuthorizer, platform: Box<dyn Platform>, cfg: LiveCoreConfig) -> LiveCore {
        LiveCore { client, registry, auth, platform, cfg, seq: AtomicU64::new(0) }
    }

    fn e(what: &str, e: impl std::fmt::Display) -> String {
        format!("{what}: {e}")
    }

    fn deadline(&self) -> String {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
        core_client::authorizer::iso_z(now + 3600).replace(".000Z", "Z")
    }

    /// The registry validates a new draft against the STAGING alias (which a publish moves), so staging is the base.
    fn staging(&self) -> Result<String, String> {
        self.client.read_alias(&self.cfg.tenant, None, &self.cfg.agent_id, Alias::Staging).map_err(|x| Self::e("staging alias", x))?.release_id.ok_or_else(|| "staging alias has no release".into())
    }

    fn bot(&self) -> Result<Jws, String> {
        let n = self.seq.fetch_add(1, Ordering::SeqCst);
        let job = format!("job-live-cred-{}-{n}", std::process::id());
        let c = self.client.issue_credential(&job, &CredentialRequest::new(&self.cfg.tenant, "constructor", "registry_write")).map_err(|x| Self::e("registry credential", x))?;
        Ok(Jws::new(c.jws().to_string()))
    }

    fn rev(&self, proposal_id: &str) -> Result<(u64, String), String> {
        let p = self.registry.proposal(proposal_id, &self.bot()?).map_err(|x| Self::e("proposal read", format!("{x:?}")))?;
        Ok((p.rev, p.state))
    }

    fn plan_of(&self, f: &FrozenInfo, d: &Draft) -> DraftPlan {
        DraftPlan::new(&self.cfg.agent_id, &f.title, d.changes.clone())
    }

    fn frozen_proposal(&self, f: &FrozenInfo, d: &Draft) -> Result<FrozenProposal, String> {
        Ok(FrozenProposal {
            proposal_id: f.proposal_id.clone(),
            candidate_hash: f.candidate_hash.clone(),
            base_release_id: Some(f.base_release_id.clone()),
            task_binding_ref: f.task_binding_ref.clone(),
            plan_ref: f.plan_ref.clone(),
            title: f.title.clone(),
            release_id_preview: None,
            commitment: WriteCommitment::for_plan(&self.plan_of(f, d), Some(&f.base_release_id)).map_err(|x| Self::e("commitment", x))?,
        })
    }

    fn flow<'a>(&'a self, f: &FrozenInfo) -> Result<RegistryFlow<'a, LocalSimAuthorizer>, String> {
        Ok(RegistryFlow::resumed_after_approval(&self.registry, &self.auth, self.bot()?, &self.cfg.agent_id, &f.proposal_id, &f.candidate_hash, Some(f.base_release_id.clone())))
    }
}

impl CorePort for LiveCore {
    fn is_real(&self) -> bool {
        true
    }

    fn dry_run(&self, ops: &[String]) -> Result<String, String> {
        let d = self.cfg.world.draft(&LiveWorld::version_of(ops)?)?;
        let base = self.staging()?;
        let r = self.client.dry_run("job-live-dry-run", &DryRunRequest::new(&self.cfg.tenant, &self.cfg.agent_id, Some(&base), d.changes.clone())).map_err(|x| Self::e("core dry-run", x))?;
        Ok(format!("sha256:{}", r.candidate_hash.ok_or("dry-run without candidate_hash")?))
    }

    fn freeze(&self, ops: &[String], job_id: &str) -> Result<FrozenInfo, String> {
        let version = LiveWorld::version_of(ops)?;
        let d = self.cfg.world.draft(&version)?;
        let base = self.staging()?;
        let title = format!("pulso-key:{job_id}");
        let plan = DraftPlan::new(&self.cfg.agent_id, &title, d.changes.clone());
        let run = WriterRun {
            tenant_id: self.cfg.tenant.clone(),
            job_id: format!("job-writer-{job_id}"),
            logical_key: "freeze".into(),
            pulso_run_ref: format!("pr-{job_id}"),
            lab_grant_ref: "grant-contract".into(),
            writer_release_id: self.cfg.writer_release_id.clone(),
            writer_agent_version: "1.0.0".into(),
        };
        let fz = self.client.freeze_draft(&*self.platform as &dyn ArtifactSealer, &plan, &run, Some(&base)).map_err(|x| Self::e("freeze", x))?;
        let (rev, _) = self.rev(&fz.proposal_id)?;
        Ok(FrozenInfo { proposal_id: fz.proposal_id, candidate_hash: fz.candidate_hash, base_release_id: base, task_binding_ref: fz.task_binding_ref, plan_ref: fz.plan_ref, title, rev, version })
    }

    fn suite(&self, f: &FrozenInfo) -> Result<SuiteInfo, String> {
        let d = self.cfg.world.draft(&f.version)?;
        let cases = d.scenarios.iter().filter_map(|s| s["id"].as_str().map(str::to_string)).collect();
        Ok(SuiteInfo { digest: d.suite.digest.clone(), cases })
    }

    fn run_arm(&self, f: &FrozenInfo, call: &ArmCall) -> Result<ArmReport, String> {
        let d = self.cfg.world.draft(&f.version)?;
        let sc = d.scenarios.iter().find(|s| s["id"] == call.case_ref.as_str()).ok_or_else(|| format!("case {} is not in the suite", call.case_ref))?;
        let manifest = format!("manifest-{}", &canon::sha256_hex(format!("{}|{}", f.proposal_id, call.case_ref).as_bytes())[..24]);
        self.platform.seal(&manifest, &json!({"scenarios": [sc], "entries": {call.case_ref.clone(): {}}})).map_err(|x| Self::e("sealing the scenario manifest", x))?;
        let (rev, _) = self.rev(&f.proposal_id)?;
        let (arm, target) = match call.side {
            Side::Base => ("baseline", json!({"kind": "published_release", "release_id": f.base_release_id})),
            Side::Candidate => ("candidate", json!({"kind": "frozen_candidate", "proposal_id": f.proposal_id, "expected_rev": rev, "base_release_id": f.base_release_id, "candidate_hash": f.candidate_hash, "draft_plan_digest": d.draft_plan_digest})),
        };
        let mut req = core_client::dto::ArmRequest::new(&call.key, &f.task_binding_ref, &call.case_ref, arm, 0, json!(7), target, &manifest, &self.cfg.budget_ref);
        req.execution_profile = Some(core_client::dto::ExecutionProfile::EvolutionTask);
        req.campaign_ref = Some("camp-live".into());
        req.deadline = Some(self.deadline());
        self.client.run_arm(&self.cfg.tenant, None, &req).map_err(|x| Self::e("run_arm", x))
    }

    fn evaluate(&self, f: &FrozenInfo, job_id: &str) -> Result<String, String> {
        let d = self.cfg.world.draft(&f.version)?;
        let fz = self.frozen_proposal(f, d)?;
        let run = EvaluationRun {
            tenant_id: self.cfg.tenant.clone(),
            job_id: format!("job-evalonly-{job_id}"),
            logical_key: "evalonly".into(),
            attempt: 1,
            pulso_run_ref: format!("pr-evalonly-{job_id}"),
            lab_grant_ref: "grant-contract".into(),
            writer_release_id: self.cfg.writer_release_id.clone(),
            writer_agent_version: "1.0.0".into(),
            budget_ref: self.cfg.budget_ref.clone(),
            deadline: self.deadline(),
        };
        let ev = match self.client.evaluate_frozen(&*self.platform as &dyn BindingPreauthorizer, &fz, &d.suite, &run) {
            Ok(ev) => ev,
            // a re-run after the evaluation already ran: the admission is consumed and the proposal is no longer frozen;
            // the identical invoke returns the stored run
            Err(OpError::Call(CallError::Api(a))) if a.code == "pulso:candidate_changed" => self.client.replay_evaluation(&fz, &d.suite, &run).map_err(|x| Self::e("evaluate replay", x))?,
            Err(x) => return Err(Self::e("evaluate", x)),
        };
        match ev.verdict() {
            Some(v) => Ok(v.to_string()),
            None if ev.receipt.is_success() => {
                // a failed gate: verified `evaluate` write, no verdict, the proposal back in draft (observed live)
                match self.rev(&f.proposal_id)?.1.as_str() {
                    "draft" => Ok("fail".into()),
                    other => Err(format!("evaluation without a verdict and the proposal is {other:?}, not draft")),
                }
            }
            None => Err(format!("the evaluate stage did not complete: {:?} {:?}", ev.receipt.state, ev.receipt.reason)),
        }
    }

    fn approve(&self, f: &FrozenInfo) -> Result<String, String> {
        let bot = self.bot()?;
        let mut flow = RegistryFlow::new(&self.registry, &self.auth, bot, &self.cfg.agent_id, &f.proposal_id, &f.candidate_hash, Some(f.base_release_id.clone()));
        let r = flow.approve().map_err(|x| Self::e("approve", format!("{x:?}")))?;
        if !(r.tamper_refused && r.replay_refused) {
            return Err("Core did not refuse the tampered hash and the replayed authorization".into());
        }
        Ok(r.approver)
    }

    fn publish(&self, f: &FrozenInfo, idempotency_key: &str) -> Result<PublishInfo, String> {
        let mut flow = self.flow(f)?;
        let p = flow.publish(idempotency_key).map_err(|x| Self::e("publish", format!("{x:?}")))?;
        let staging = self.client.read_alias(&self.cfg.tenant, None, &self.cfg.agent_id, Alias::Staging).map_err(|x| Self::e("staging readback", x))?;
        Ok(PublishInfo { release_id: p.release_id, staging_release_id: staging.release_id.unwrap_or_default() })
    }
}
