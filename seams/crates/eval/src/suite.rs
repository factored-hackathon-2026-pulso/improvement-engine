//! The assets evaluation suite, sealed before any arm runs, and the arm requests built from it.
use core_client::canon;
use core_client::dto::{ArmRequest, ExecutionProfile};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub struct SuiteCase {
    pub case_ref: String,
    pub scenario_manifest_ref: String,
    pub oracle_ref: Option<String>,
    pub seed: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suite {
    pub suite_id: String,
    pub version: String,
    /// Actor that authored the suite (feeds the author-is-not-judge rule).
    pub author_actor: String,
    pub cases: Vec<SuiteCase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiteError {
    Invalid(String),
    ChangedAfterSeal,
    AlreadyStarted,
    Request(String),
}

impl Suite {
    /// Canonical digest committed at seal time (JCS + sha256, same canon as core-client).
    pub fn digest(&self) -> Result<String, SuiteError> {
        let cases: Vec<Value> = self
            .cases
            .iter()
            .map(|c| json!({"case_ref": c.case_ref, "scenario_manifest_ref": c.scenario_manifest_ref, "oracle_ref": c.oracle_ref, "seed": c.seed}))
            .collect();
        let v = json!({"suite_id": self.suite_id, "version": self.version, "author_actor": self.author_actor, "cases": cases});
        canon::digest_json(&v).map_err(|e| SuiteError::Invalid(format!("{e:?}")))
    }

    fn check(&self) -> Result<(), SuiteError> {
        if self.suite_id.is_empty() || self.version.is_empty() || self.author_actor.is_empty() {
            return Err(SuiteError::Invalid("suite_id, version and author_actor are required".into()));
        }
        if self.cases.is_empty() {
            return Err(SuiteError::Invalid("a suite needs at least one case".into()));
        }
        let uniq: BTreeSet<&str> = self.cases.iter().map(|c| c.case_ref.as_str()).collect();
        if uniq.len() != self.cases.len() || uniq.contains("") {
            return Err(SuiteError::Invalid("case_ref must be non-empty and unique".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RunParams {
    pub binding_ref: String,
    pub budget_ref: String,
    pub baseline_target: Value,
    pub candidate_target: Value,
    pub repetitions: u32,
}

/// A suite plus its committed digest. There is no amend: the only way to a different suite is a new package.
#[derive(Debug, Clone)]
pub struct EvalPackage {
    suite: Suite,
    suite_digest: String,
    started: bool,
}

/// Injective-in-practice case token: a digest of the exact `case_ref` (a lossy sanitiser would let `a b` and `a-b` collide).
fn case_token(s: &str) -> String {
    canon::sha256_hex(s.as_bytes())[..16].to_string()
}

impl EvalPackage {
    pub fn try_seal(suite: Suite) -> Result<EvalPackage, SuiteError> {
        suite.check()?;
        let suite_digest = suite.digest()?;
        Ok(EvalPackage { suite, suite_digest, started: false })
    }

    /// Panics on an invalid suite; use [`EvalPackage::try_seal`] for untrusted input.
    pub fn seal(suite: Suite) -> EvalPackage {
        Self::try_seal(suite).expect("valid suite")
    }

    pub fn suite_digest(&self) -> &str {
        &self.suite_digest
    }

    pub fn suite(&self) -> &Suite {
        &self.suite
    }

    pub fn arms_started(&self) -> bool {
        self.started
    }

    /// Fails unless `presented` hashes to the committed digest.
    pub fn verify(&self, presented: &Suite) -> Result<(), SuiteError> {
        if presented.digest()? == self.suite_digest { Ok(()) } else { Err(SuiteError::ChangedAfterSeal) }
    }

    /// Builds the arm requests (baseline and candidate x cases x repetitions) from the sealed suite and marks the
    /// arms as started. Keys are deterministic, so a replay of the same package is single-flight.
    pub fn start_arms(&mut self, p: &RunParams) -> Result<Vec<ArmRequest>, SuiteError> {
        if self.started {
            return Err(SuiteError::AlreadyStarted);
        }
        if p.repetitions == 0 {
            return Err(SuiteError::Invalid("repetitions must be at least 1".into()));
        }
        let mut out = Vec::new();
        for (arm, target) in [("baseline", &p.baseline_target), ("candidate", &p.candidate_target)] {
            for c in &self.suite.cases {
                for rep in 0..p.repetitions {
                    let key = format!("ev-{}-{}-{}-{}", &self.suite_digest[..16], arm, case_token(&c.case_ref), rep);
                    let mut r = ArmRequest::new(&key, &p.binding_ref, &c.case_ref, arm, rep, c.seed.clone(), target.clone(), &c.scenario_manifest_ref, &p.budget_ref);
                    r.execution_profile = Some(ExecutionProfile::EvolutionTask);
                    r.oracle_ref = c.oracle_ref.clone();
                    r.validate().map_err(SuiteError::Request)?;
                    out.push(r);
                }
            }
        }
        self.started = true;
        Ok(out)
    }
}
