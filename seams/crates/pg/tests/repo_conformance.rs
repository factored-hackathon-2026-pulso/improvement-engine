mod common;
use common::{migrations_dir, TempDb};
use pg::conformance::run_suite;
use pg::repo::{Claimed, JobRepository, MemRepo, RepoError};
use std::sync::Arc;

#[test]
fn mem_repo_conforms() {
    let f = run_suite(&|| Arc::new(MemRepo::new()));
    assert!(f.is_empty(), "{f:#?}");
}

/// A repository that selects in one step and writes the lease in another (read-then-write): double claims.
struct Racy(MemRepo);
impl JobRepository for Racy {
    fn admit(&self, t: &str) -> Result<String, RepoError> { self.0.admit(t) }
    fn claim_next(&self, t: &str, w: &str, now: u64, lease: u64) -> Result<Option<Claimed>, RepoError> {
        if lease == 0 { return Err(RepoError::InvalidLeaseDuration); }
        let Some(id) = self.0.peek_candidate(t, now) else { return Ok(None) };
        std::thread::sleep(std::time::Duration::from_millis(5));
        Ok(Some(self.0.force_claim(t, &id, w, now, lease)))
    }
    fn touch_lease(&self, t: &str, j: &str, w: &str, f: u64, n: u64, l: u64) -> Result<u64, RepoError> { self.0.touch_lease(t, j, w, f, n, l) }
    fn begin_effect(&self, t: &str, j: &str, w: &str, f: u64, n: u64) -> Result<(), RepoError> { self.0.begin_effect(t, j, w, f, n) }
    fn commit_output(&self, t: &str, j: &str, s: u32, w: &str, f: u64, n: u64, r: &str) -> Result<(), RepoError> { self.0.commit_output(t, j, s, w, f, n, r) }
    fn output(&self, t: &str, j: &str, s: u32) -> Result<Option<String>, RepoError> { self.0.output(t, j, s) }
    fn complete(&self, t: &str, j: &str, w: &str, f: u64, n: u64) -> Result<(), RepoError> { self.0.complete(t, j, w, f, n) }
    fn admit_keyed(&self, t: &str, k: &str) -> Result<String, RepoError> { self.0.admit_keyed(t, k) }
    fn job_key(&self, t: &str, j: &str) -> Result<Option<String>, RepoError> { self.0.job_key(t, j) }
}

#[test]
fn suite_fails_on_a_repository_that_double_claims() {
    let f = run_suite(&|| Arc::new(Racy(MemRepo::new())));
    assert!(f.iter().any(|l| l.starts_with("single_winner_under_threads")), "double claim not caught: {f:#?}");
}

#[test]
fn pg_repo_conforms() {
    let Some(db) = TempDb::create() else { return };
    let mut c = db.connect();
    pg::migrate::migrate(&mut c, &pg::migrate::discover(&migrations_dir()).unwrap()).unwrap();
    let cfg = db.config();
    let f = run_suite(&|| Arc::new(pg::pgrepo::PgRepo::new(cfg.clone())));
    assert!(f.is_empty(), "{f:#?}");
}
