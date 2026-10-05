//! W11 "evaluate before announce": a proposal is ANNOUNCED only after the engine proved it.
//!
//! 1. `scripts/regression/build_suite.py` (subprocess, JSON contract below) builds the regression `eval_suite` of the finding.
//! 2. The suite is attached to a draft of the BASE (no change) and to a draft of each CANDIDATE attempt; each is frozen and
//!    evaluated by agent-core through the guarded writer (`eval`, MANUAL-origin drafts: the 10/24h `auto_detect` quota is untouched).
//! 3. `scripts/regression/judge_story.py` (subprocess) turns the raw runs into the REG1 verdict story with the same rules as the
//!    Python proof: `regression_suite_proven | non_discriminating | not_fixed | guard_regressed | infra_failed`.
//! 4. `reasoning::dossier::build` carries the story into the decision dossier (ES + PT). `announce` is true only for a proven
//!    suite on a corroborated finding with a real change.
//!
//! The caller delivers (origin `auto_detect`) only when `Proof::announce` is true, with the dossier ES text as the docs of the
//! change (`announce_submission`) and the attached eval_suite so the human flow can freeze and evaluate it. Otherwise the outcome is
//! the internal `not_announced:<verdict>` and the dossier stays in the engine job record: never silently dropped.
//!
//! Subprocess contracts (stdin/stdout JSON, no network, no credential):
//! * `build_suite.py --finding <file> --target <ref> --out <dir>`: exit 0 prints `{"suite": <id>, ...}` and writes
//!   `<dir>/<id>.bundle.json`; exit 2 prints `{"refused": <code>, "why": ..}` (k floor, PII lint, no mechanism, no guards).
//! * `judge_story.py` reads `w11.judge_input/1` and prints `reg1.verdict_story/1` (see that script).
//!
//! W13 (prompt and new-agent targets):
//! * PROMPT patch: the suite asserts natively that the response came from the model path (`response_emitted` kind generated,
//!   `fallback_used` false) and a harness probe (`generated_contains`, samples collected by `sample_probes.py` from the local gateway)
//!   judges the wording the scorer cannot read. agent-core `evaluate` only exercises a candidate prompt with PR 50 (gateway bound to
//!   the evaluated closure), so the proof ALSO evaluates a CONTROL: a text-identical version bump of the base prompt. If the control
//!   fails the native assertions that the base passes, `evaluate` ignored the candidate prompt: the native result of the candidate is not
//!   used, the harness probe decides alone, the story is labelled `native_not_candidate_bound` and nothing is announced.
//! * NEW AGENT (clone-closure of the donor): the suite runs on the NEW agent itself. The base has no such agent: it is not evaluated
//!   (`verdict: absent`, the finding cases fail by absence, not measured). The candidate draft is the compiled proposal plus its closure
//!   (`closure` module) and, for the evaluation only, the donor's release settings; the `recepcion` routing is not measured.
//!
//! Idempotent per finding key: a `ProofStore` remembers the conclusive proof of (finding key, candidate digest) and a replay returns
//! it with no HTTP and no subprocess; an inconclusive one (`infra_failed`) is re-run under a fresh `Idempotency-Key` salt.
use crate::closure;
use crate::eval::{EvalJob, EvalOptions, with_default_thresholds};
use crate::writer::Writer;
use crate::{Submission, guard};
use reasoning::catalog::Artifact;
use reasoning::dossier::{self, Labels};
use reasoning::finding::Finding;
use reasoning::patch::Compiled;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const PROOF_CONTRACT: &str = "w11.proof/1";
pub const JUDGE_INPUT: &str = "w11.judge_input/1";
/// INH1: closed problem codes of an evaluation draft that carried `release_settings.inherit_from` and was refused (see `eval`).
const INHERIT_PROBLEMS: [&str; 3] = ["core_without_inherit_from", "inherit_from_rejected", "donor_release_unknown"];

// ------------------------------------------------------------------------------------------------------------------ scripts

#[derive(Debug, Clone, PartialEq)]
pub enum SuiteError {
    /// build_suite refused on purpose: `(code, why)`.
    Refused(String, String),
    /// The script could not run or answered nonsense.
    Failed(String),
}

/// The Python side of the proof. A trait so the offline tests script it.
pub trait Scripts {
    fn build_suite(&self, finding: &Value, target_ref: &str) -> Result<Value, SuiteError>;
    /// Like `build_suite`, for a NEW agent the suite runs on (`--agent <slug>`). A script set without it refuses on purpose.
    fn build_suite_for(&self, finding: &Value, target_ref: &str, new_agent: Option<&str>) -> Result<Value, SuiteError> {
        match new_agent {
            None => self.build_suite(finding, target_ref),
            Some(_) => Err(SuiteError::Refused("new_agent_unsupported".into(), "this script set cannot build a new-agent suite".into())),
        }
    }
    fn judge(&self, input: &Value) -> Result<Value, String>;
    /// The model samples of the `generated_contains` probes of a judge input (`{"generated": {key: {samples}|{error}}}`). The default
    /// has no sampler: the probes are then NOT measured and their cases fail.
    fn sample(&self, _input: &Value) -> Result<Value, String> {
        Err("no probe sampler configured".into())
    }
}

/// Runs the repo scripts as subprocesses. `python` is the command and its leading arguments (`["python"]`, or
/// `["uv", "run", "--with", "pyyaml", "python"]`): build_suite needs PyYAML.
pub struct PythonScripts {
    pub python: Vec<String>,
    pub script_dir: PathBuf,
    pub work: PathBuf,
    /// Extra environment of the child processes (tests point the probe sampler at a local gateway double). Credentials are NOT
    /// listed here: `sample_probes.py` reads `GATEWAY_TOKEN_AGENT_CORE` from the inherited process environment.
    pub env: Vec<(String, String)>,
}

impl PythonScripts {
    fn cmd(&self, script: &str) -> Result<Command, String> {
        let (prog, args) = self.python.split_first().ok_or("no python command configured")?;
        let mut c = Command::new(prog);
        c.args(args).arg(self.script_dir.join(script));
        c.env("PYTHONIOENCODING", "utf-8");
        c.envs(self.env.iter().map(|(k, v)| (k, v)));
        Ok(c)
    }
}

fn last_json(out: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(out);
    text.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok().filter(Value::is_object))
}

impl Scripts for PythonScripts {
    fn build_suite(&self, finding: &Value, target_ref: &str) -> Result<Value, SuiteError> {
        self.build_suite_for(finding, target_ref, None)
    }

    fn build_suite_for(&self, finding: &Value, target_ref: &str, new_agent: Option<&str>) -> Result<Value, SuiteError> {
        if let Some(slug) = new_agent
            && !guard::ok_seg(slug)
        {
            return Err(SuiteError::Failed("new agent slug is not a safe id".into()));
        }
        let tag = steps::compile::sha256_hex(format!("{finding}{target_ref}{}", new_agent.unwrap_or("")).as_bytes());
        let dir = self.work.join(format!("w11-suite-{}", &tag[..12]));
        std::fs::create_dir_all(&dir).map_err(|e| SuiteError::Failed(format!("work dir: {e}")))?;
        let f = dir.join("finding.json");
        std::fs::write(&f, serde_json::to_vec(finding).expect("json")).map_err(|e| SuiteError::Failed(format!("finding file: {e}")))?;
        let mut cmd = self.cmd("build_suite.py").map_err(SuiteError::Failed)?;
        cmd.arg("--finding").arg(&f).args(["--target", target_ref]).arg("--out").arg(&dir);
        if let Some(slug) = new_agent {
            cmd.args(["--agent", slug]);
        }
        let out = cmd.output().map_err(|e| SuiteError::Failed(format!("spawn: {}", e.kind())))?;
        let said = last_json(&out.stdout);
        match (out.status.code(), said) {
            (Some(0), Some(v)) => {
                let id = v["suite"].as_str().filter(|s| guard::ok_seg(s)).ok_or(SuiteError::Failed("build_suite printed no suite id".into()))?;
                let raw = std::fs::read_to_string(dir.join(format!("{id}.bundle.json"))).map_err(|e| SuiteError::Failed(format!("bundle file: {}", e.kind())))?;
                serde_json::from_str(&raw).map_err(|_| SuiteError::Failed("bundle is not JSON".into()))
            }
            (Some(2), Some(v)) => Err(SuiteError::Refused(v["refused"].as_str().unwrap_or("refused").chars().filter(|c| c.is_ascii_lowercase() || *c == '_').collect(), v["why"].as_str().unwrap_or("").chars().take(160).collect())),
            (code, _) => Err(SuiteError::Failed(format!("build_suite exit {code:?}: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").chars().take(120).collect::<String>()))),
        }
    }

    fn judge(&self, input: &Value) -> Result<Value, String> {
        let mut child = self.cmd("judge_story.py")?.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("spawn: {}", e.kind()))?;
        child.stdin.take().ok_or("no stdin")?.write_all(input.to_string().as_bytes()).map_err(|e| format!("stdin: {}", e.kind()))?;
        let out = child.wait_with_output().map_err(|e| format!("wait: {}", e.kind()))?;
        let story = last_json(&out.stdout).ok_or_else(|| format!("judge_story exit {:?} without JSON", out.status.code()))?;
        if out.status.code() != Some(0) || story["schema"] != "reg1.verdict_story/1" {
            return Err(format!("judge_story exit {:?}: {}", out.status.code(), story["error"].as_str().unwrap_or("not a verdict story")));
        }
        Ok(story)
    }

    fn sample(&self, input: &Value) -> Result<Value, String> {
        let mut child = self.cmd("sample_probes.py")?.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("spawn: {}", e.kind()))?;
        child.stdin.take().ok_or("no stdin")?.write_all(input.to_string().as_bytes()).map_err(|e| format!("stdin: {}", e.kind()))?;
        let out = child.wait_with_output().map_err(|e| format!("wait: {}", e.kind()))?;
        let v = last_json(&out.stdout).ok_or_else(|| format!("sample_probes exit {:?} without JSON", out.status.code()))?;
        if out.status.code() != Some(0) || !v["generated"].is_object() {
            return Err(format!("sample_probes exit {:?}: {}", out.status.code(), v["error"].as_str().unwrap_or("no samples")));
        }
        Ok(v)
    }
}

// ------------------------------------------------------------------------------------------------------------------ store

/// What a finished proof leaves behind (ids, outcome and the dossier; no token, no model free text).
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    /// Inconclusive runs so far (salts the Idempotency-Key of the next one).
    pub runs: u32,
    pub proof: Option<Value>,
}

pub trait ProofStore {
    fn get(&self, key: &str) -> Option<Stored>;
    fn put(&self, key: &str, s: Stored) -> Result<(), String>;
}

#[derive(Default)]
pub struct MemoryProofStore(RefCell<BTreeMap<String, Stored>>);

impl ProofStore for MemoryProofStore {
    fn get(&self, key: &str) -> Option<Stored> {
        self.0.borrow().get(key).cloned()
    }
    fn put(&self, key: &str, s: Stored) -> Result<(), String> {
        self.0.borrow_mut().insert(key.into(), s);
        Ok(())
    }
}

pub struct FileProofStore {
    path: PathBuf,
}

impl FileProofStore {
    pub fn new(path: impl Into<PathBuf>) -> FileProofStore {
        FileProofStore { path: path.into() }
    }
    fn load(&self) -> serde_json::Map<String, Value> {
        std::fs::read_to_string(&self.path).ok().and_then(|r| serde_json::from_str::<Value>(&r).ok()).and_then(|v| v["proofs"].as_object().cloned()).unwrap_or_default()
    }
}

impl ProofStore for FileProofStore {
    fn get(&self, key: &str) -> Option<Stored> {
        let m = self.load();
        let v = m.get(key)?;
        Some(Stored { runs: v["runs"].as_u64().unwrap_or(0) as u32, proof: v.get("proof").filter(|p| p.is_object()).cloned() })
    }
    fn put(&self, key: &str, s: Stored) -> Result<(), String> {
        let mut m = self.load();
        m.insert(key.into(), json!({"runs": s.runs, "proof": s.proof}));
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&json!({"proofs": m})).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

// ------------------------------------------------------------------------------------------------------------------ proof

#[derive(Debug, Clone, PartialEq)]
pub struct Proof {
    pub announce: bool,
    /// `announced` or `not_announced:<verdict>` (the verdict is the story outcome, or `suite_refused`, `suite_error`, `dossier_error`).
    pub outcome: String,
    pub verdict: String,
    pub replayed: bool,
    /// The bundle's suite with the agent's default thresholds (what the announced proposal carries).
    pub suite: Option<Value>,
    pub story: Value,
    pub dossier: Value,
    /// Evaluation proposals opened for this proof (manual origin), for the record.
    pub eval_proposals: Vec<String>,
    /// Unchanged closure copies a NEW agent proposal must carry (delivered with the announced proposal, never settings).
    pub extra_changes: Vec<Value>,
}

impl Proof {
    fn to_stored(&self) -> Value {
        json!({"announce": self.announce, "outcome": self.outcome, "verdict": self.verdict, "suite": self.suite, "story": self.story, "dossier": self.dossier, "eval_proposals": self.eval_proposals, "extra_changes": self.extra_changes})
    }

    fn from_stored(v: &Value) -> Option<Proof> {
        Some(Proof {
            announce: v["announce"].as_bool()?,
            outcome: v["outcome"].as_str()?.into(),
            verdict: v["verdict"].as_str()?.into(),
            replayed: true,
            suite: v["suite"].as_object().map(|_| v["suite"].clone()),
            story: v["story"].clone(),
            dossier: v["dossier"].clone(),
            eval_proposals: v["eval_proposals"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect(),
            extra_changes: v["extra_changes"].as_array().cloned().unwrap_or_default(),
        })
    }

    /// The record the engine job store keeps: closed codes, ids, counts and the dossier. The probe samples and per-case texts of the
    /// story are left out (the story's `per_case` reasons are scorer messages, not needed to read the verdict).
    pub fn record(&self) -> Value {
        let run = |r: &Value| json!({"label": r["label"], "proposal_id": r["proposal_id"], "verdict": r["verdict"], "failed_cases": r["failed_cases"], "guards_failed": r["guards_failed"],
                                     "gate_items": r["gate_items"], "problem": r["problem"], "infra_retries": r["infra_retries"].as_array().map_or(0, Vec::len), "verdict_note": r["verdict_note"]});
        json!({"contract": PROOF_CONTRACT, "outcome": self.outcome, "verdict": self.verdict, "announce": self.announce, "replayed": self.replayed,
               "reason": self.story["reason"], "suite_id": self.story["suite_id"], "mechanism": self.story["mechanism"],
               "base": run(&self.story["base"]), "attempts": self.story["attempts"].as_array().map(|a| a.iter().map(run).collect::<Vec<_>>()),
               "story_text": self.story["story_text"], "model_policy": self.story["model_policy"], "eval_proposals": self.eval_proposals,
               "native_binding": self.story["native_binding"], "coverage": self.story["coverage"], "new_agent": self.story["new_agent"],
               "labels": {"verdict_judge": "deterministic checks (no model judge)", "rubric": self.dossier["honesty"]["rubric"], "judge_family": self.dossier["honesty"]["judge_family"],
                          "calibration": self.dossier["honesty"]["calibration"], "runtime": self.dossier["honesty"]["runtime"]},
               "dossier": self.dossier})
    }
}

/// Everything the proof needs about the proposal that reasoning produced.
pub struct ProofInput<'a> {
    pub finding: &'a Finding,
    pub compiled: &'a Compiled,
    /// Candidate attempts to evaluate in order, stopping at the first that passes (default: the compiled changes alone).
    pub attempts: Vec<Vec<Value>>,
    /// The live (or fixture) artifact the patch was compiled against: its texts are what the wording probe renders for the base.
    pub base_artifact: Option<&'a Artifact>,
    pub labels: Labels,
    /// Honesty inputs of the reasoning record (`doubles`, `rubric`), forwarded to the dossier.
    pub doubles: Value,
    pub rubric: Value,
}

fn digest_of(changes: &[Vec<Value>]) -> String {
    steps::compile::sha256_hex(serde_json::to_string(changes).unwrap_or_default().as_bytes())[..16].to_string()
}

fn stub_story(finding: &Value, target: &str, outcome: &str, reason: &str) -> Value {
    json!({"schema": "reg1.verdict_story/1", "finding_key": null, "finding_id": finding["finding_id"], "target": target, "outcome": outcome, "reason": reason, "announce": false,
           "suite_is_regression_suite": false, "base": {"per_case": {}, "failed_cases": [], "guards_failed": [], "verdict": "not_run", "gate_items": []}, "attempts": [], "gate_items": [],
           "story_text": {}, "model_policy": {"judge": "none", "judge_rule": "deterministic checks only"}})
}

/// Is this bundle's wording judged by a model (a prompt target)?
fn has_generated_probes(bundle: &Value) -> bool {
    bundle["probes"].as_array().is_some_and(|p| p.iter().any(|x| x["kind"] == "generated_contains"))
}

/// The binding control: the candidate's prompt entities with the BASE texts and the candidate's version (a text-identical bump).
fn control_changes(attempt: &[Value], base: &Artifact) -> Vec<Value> {
    attempt
        .iter()
        .filter(|c| c["kind"] == "prompt")
        .map(|c| {
            let mut c = c.clone();
            c["content"]["locales"] = json!(base.locales);
            c
        })
        .collect()
}

/// Proves (or fails to prove) one compiled proposal. Never panics, never approves. `w` must be a writer whose transport tolerates a
/// long `evaluate` (the caller's timeout).
pub fn prove(w: &Writer, scripts: &dyn Scripts, store: &dyn ProofStore, opts: &EvalOptions, inp: &ProofInput) -> Proof {
    let c = inp.compiled;
    let attempts = if inp.attempts.is_empty() { vec![c.changes.clone()] } else { inp.attempts.clone() };
    let key = Submission::new(inp.finding, c).key();
    let pkey = format!("{key}-{}", digest_of(&attempts));
    let prev = store.get(&pkey);
    if let Some(p) = prev.as_ref().and_then(|s| s.proof.as_ref()).and_then(Proof::from_stored) {
        return p; // conclusive before: no HTTP, no subprocess
    }
    let runs = prev.map_or(0, |s| s.runs);
    let mut signal = inp.finding.to_signal_json();
    signal["source"] = json!(inp.finding.source.as_str());
    // ART2: the structured params of a tool link or policy draft travel in the finding file (build_suite.py reads `art2`); never model text.
    if let Some(p) = c.expected_effect["art2"]["suite_params"].as_object() {
        signal["art2"] = json!(p);
    }
    let owner_ack = c.expected_effect["art2"]["owner_ack"] == true;
    let finish = |story: Value, suite: Option<Value>, evals: Vec<String>, extra: Vec<Value>| -> Proof {
        let record = json!({"proposal": c.to_json(), "doubles": inp.doubles, "rubric": inp.rubric});
        let (dossier_v, dossier_err) = match dossier::build(&signal, &record, Some(&story), &inp.labels) {
            Ok(d) => (d, false),
            Err(e) => (json!({"schema": dossier::SCHEMA, "announce": false, "announce_reason": "dossier_error", "error": e}), true),
        };
        let verdict = if dossier_err { "dossier_error".to_string() } else { story["outcome"].as_str().unwrap_or("unknown").to_string() };
        let proven = !dossier_err && (dossier_v["announce"].as_bool() == Some(true) || dossier_v["announce_reason"] == "needs_owner_ack") && story["announce"].as_bool() == Some(true);
        // ART2: a human-owned policy draft is proven natively but NEVER announced automatically: it waits for the owner (`needs_owner_ack`).
        let announce = proven && !owner_ack;
        let p = Proof {
            announce,
            outcome: if announce { "announced".into() } else if proven && owner_ack { "needs_owner_ack".into() } else { format!("not_announced:{verdict}") },
            verdict,
            replayed: false,
            suite: if announce { suite } else { None },
            story,
            dossier: dossier_v,
            eval_proposals: evals,
            extra_changes: if announce { extra } else { vec![] },
        };
        let conclusive = !matches!(p.verdict.as_str(), "infra_failed" | "suite_error" | "not_exercised");
        let _ = store.put(&pkey, Stored { runs: if conclusive { runs } else { runs + 1 }, proof: conclusive.then(|| p.to_stored()) });
        p
    };

    let new_agent = (c.kind == "new_agent").then_some(c.agent_id.as_str());
    let bundle = match scripts.build_suite_for(&signal, &c.target_ref, new_agent) {
        Ok(b) => b,
        Err(SuiteError::Refused(code, _)) => return finish(stub_story(&signal, &c.target_ref, "suite_refused", &code), None, vec![], vec![]),
        Err(SuiteError::Failed(why)) => return finish(stub_story(&signal, &c.target_ref, "suite_error", &why), None, vec![], vec![]),
    };
    let agent = bundle["agent"].as_str().unwrap_or("");
    let agent_entity = if new_agent.is_some() { None } else { w.fetch_content("agent", agent) };
    let suite = with_default_thresholds(&bundle["suite"], agent_entity.as_ref());
    let salt = format!("{pkey}-r{runs}"); // finding + candidate digest + inconclusive-run count: a new candidate never replays an old draft
    let mut evals: Vec<String> = vec![];
    // New agent: the draft of every evaluated candidate carries the donor closure; the evaluation also gets the donor's release settings.
    let closure = match new_agent {
        Some(_) => match closure::donor_closure(w, &c.changes) {
            Ok(cl) => Some(cl),
            Err(why) => return finish(stub_story(&signal, &c.target_ref, "suite_error", &why), None, vec![], vec![]),
        },
        None => None,
    };
    // INH1: the closure (and with it the settings mode) can switch ONCE to the explicit-admin fallback, so it lives in a cell.
    let closure = RefCell::new(closure);
    let eval_changes_of = |a: &Vec<Value>| closure.borrow().as_ref().map_or_else(|| a.clone(), |cl| closure::eval_changes(a, cl));
    let judge_changes_of = |a: &Vec<Value>| closure.borrow().as_ref().map_or_else(|| a.clone(), |cl| closure::delivered_changes(a, cl));
    let base_artifacts = inp.base_artifact.map(|a| json!({"artifacts": [{"kind": a.kind, "id": a.id, "version": a.version, "locales": a.locales}]}));
    let is_prompt = has_generated_probes(&bundle);
    let generated = RefCell::new(json!({}));
    let judge = |base: &Value, atts: &[Value], control: Option<&Value>| -> Result<Value, String> {
        let mut doc = json!({"schema": JUDGE_INPUT, "bundle": bundle, "base": base, "attempts": atts});
        if let Some(b) = &base_artifacts {
            doc["base_artifacts"] = b.clone();
        }
        if let Some(ctl) = control {
            doc["control"] = ctl.clone();
        }
        if let Some(cl) = closure.borrow().as_ref() {
            doc["settings"] = json!(cl.settings.code()); // the dossier says HOW the clone got its safety settings
        }
        if is_prompt {
            // The model samples of the wording probes (the judge itself has no network); a failure leaves them uncollected: not measured.
            doc["generated"] = generated.borrow().clone();
            if let Ok(s) = scripts.sample(&doc) {
                *generated.borrow_mut() = s["generated"].clone();
                doc["generated"] = s["generated"].clone();
            }
        }
        scripts.judge(&doc)
    };

    let base = if new_agent.is_some() {
        json!({"label": "base", "proposal_id": null, "verdict": "absent", "gate_items": [], "per_case_native": {}, "detail": "the new agent does not exist on the base", "problem": null, "infra_retries": []})
    } else {
        let b = w.evaluate_run(&EvalJob { label: "base", key: &salt, agent_id: agent, suite: &bundle["suite"], agent_entity: agent_entity.as_ref(), changes: &[] }, opts);
        if let Some(id) = b["proposal_id"].as_str() {
            evals.push(id.to_string());
        }
        b
    };
    let mut story = match judge(&base, &[], None) {
        Ok(s) => s,
        Err(e) => return finish(stub_story(&signal, &c.target_ref, "suite_error", &format!("judge: {e}")), None, evals, vec![]),
    };
    if story["outcome"] == "base_only" {
        // Binding control (prompt targets): a text-identical bump of the base prompt; see the module docs.
        let control = match (is_prompt, inp.base_artifact) {
            (true, Some(art)) => {
                let ctl = control_changes(&attempts[0], art);
                if ctl.is_empty() {
                    None
                } else {
                    let run = w.evaluate_run(&EvalJob { label: "control", key: &salt, agent_id: agent, suite: &bundle["suite"], agent_entity: agent_entity.as_ref(), changes: &ctl }, opts);
                    if let Some(id) = run["proposal_id"].as_str() {
                        evals.push(id.to_string());
                    }
                    Some(run)
                }
            }
            _ => None,
        };
        let mut done: Vec<Value> = vec![];
        for (n, attempt) in attempts.iter().enumerate() {
            let label: &'static str = ["candidate-1", "candidate-2", "candidate-3"][n.min(2)];
            let mut changes = eval_changes_of(attempt);
            let mut run = w.evaluate_run(&EvalJob { label, key: &salt, agent_id: agent, suite: &bundle["suite"], agent_entity: agent_entity.as_ref(), changes: &changes }, opts);
            if let Some(id) = run["proposal_id"].as_str() {
                evals.push(id.to_string());
            }
            // INH1: a Core that does not take the donor reference. Only an explicit admin credential may fall back to the old way.
            if let Some(code) = run["problem"]["code"].as_str().filter(|c| INHERIT_PROBLEMS.contains(c)) {
                if code == "core_without_inherit_from" && w.admin_settings_fallback() && closure.borrow().as_ref().is_some_and(|cl| matches!(cl.settings, closure::Settings::Inherit { .. })) {
                    match closure::donor_closure_with(w, &c.changes, true) {
                        Ok(cl) => {
                            *closure.borrow_mut() = Some(cl);
                            changes = eval_changes_of(attempt);
                            run = w.evaluate_run(&EvalJob { label, key: &format!("{salt}-assumed"), agent_id: agent, suite: &bundle["suite"], agent_entity: agent_entity.as_ref(), changes: &changes }, opts);
                            if let Some(id) = run["proposal_id"].as_str() {
                                evals.push(id.to_string());
                            }
                        }
                        Err(why) => return finish(stub_story(&signal, &c.target_ref, "suite_error", &why), None, evals, vec![]),
                    }
                }
                if let Some(code) = run["problem"]["code"].as_str().filter(|c| INHERIT_PROBLEMS.contains(c)) {
                    let why = match code {
                        "core_without_inherit_from" => "core_without_inherit_from: this agent-core does not know release_settings.inherit_from (older Core); the clone cannot be evaluated without an admin credential",
                        "donor_release_unknown" => "donor_release_unknown: agent-core does not know the donor release named by inherit_from",
                        _ => "inherit_from_rejected: agent-core rejected the donor reference",
                    };
                    return finish(stub_story(&signal, &c.target_ref, "suite_error", why), None, evals, vec![]);
                }
            }
            done.push(json!({"attempt": n + 1, "changes": judge_changes_of(attempt), "run": run}));
            story = match judge(&base, &done, control.as_ref()) {
                Ok(s) => s,
                Err(e) => return finish(stub_story(&signal, &c.target_ref, "suite_error", &format!("judge: {e}")), None, evals, vec![]),
            };
            if story["attempts"].as_array().and_then(|a| a.last()).is_some_and(|a| matches!(a["verdict"].as_str(), Some("pass" | "probe_only_pass"))) {
                break;
            }
        }
    }
    let extra = closure.borrow().as_ref().map(closure::delivered_extra).unwrap_or_default();
    finish(story, Some(suite), evals, extra)
}

/// The submission of an ANNOUNCED proposal: the compiled changes with the dossier ES text as their docs (description, rationale,
/// changelog) plus the proven `eval_suite` as one more change, so the supervisor's own freeze and evaluate can run it.
pub fn announce_submission(f: &Finding, c: &Compiled, proof: &Proof) -> Submission {
    let mut s = Submission::new(f, c);
    let es = &proof.dossier["es"];
    for ch in s.changes.iter_mut() {
        let docs = json!({"description": es["description"], "rationale": es["rationale"], "changelog": es["changelog"]});
        ch["docs"] = docs;
    }
    s.changes.extend(proof.extra_changes.iter().cloned()); // a new agent's unchanged closure (their docs are their own)
    if let Some(suite) = &proof.suite {
        s.changes.push(json!({"kind": "eval_suite", "content": suite, "docs": {
            "description": format!("Regression eval_suite {} proven on the base (fails) and the candidate (passes).", suite["id"].as_str().unwrap_or("")),
            "rationale": es["sections"]["result"],
            "changelog": "Adds the regression suite the engine used to prove this proposal."}}));
    }
    s
}
