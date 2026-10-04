//! Mutation testing of the executor conformance suite: stores with a deliberately broken `commit_guarded`
//! must fail it.
use engine::conformance::{run_suite, Backend};
use engine::{CommitGuard, FileStore, JobStore};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
enum Mutant {
    NoGuard,
    IgnoreFence,
    IgnoreExpiry,
    IgnoreWorker,
    AllowOverwrite,
    AcceptWithoutLease,
}

struct Broken(FileStore, Mutant);
impl JobStore for Broken {
    fn get(&self, key: &str) -> Result<Option<(u64, String)>, String> { self.0.get(key) }
    fn cas(&self, key: &str, expected: u64, value: &str) -> Result<u64, String> { self.0.cas(key, expected, value) }
    fn commit_guarded(&self, key: &str, value: &str, g: &CommitGuard) -> Result<u64, String> {
        let lease = self.0.get("lease")?;
        let ok = match (self.1, &lease) {
            (Mutant::NoGuard, _) => true,
            (Mutant::AcceptWithoutLease, None) => true,
            (_, None) => false,
            (m, Some((_, l))) => {
                let p: Vec<&str> = l.split('|').collect();
                let (w, f, exp) = (p[0], p[1].parse::<u64>().unwrap(), p[3].parse::<u64>().unwrap());
                (matches!(m, Mutant::IgnoreWorker) || w == g.worker_id)
                    && (matches!(m, Mutant::IgnoreFence) || f == g.fence_token)
                    && (matches!(m, Mutant::IgnoreExpiry) || g.now < exp)
            }
        };
        if !ok {
            return Err("stale fence".into());
        }
        let have = if matches!(self.1, Mutant::AllowOverwrite) { self.0.get(key)?.map_or(0, |(v, _)| v) } else { 0 };
        self.0.cas(key, have, value)
    }
}

struct Files(Mutant);
fn dir(name: &str, m: Mutant) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("e2mut-{m:?}-{name}-{}", std::process::id()))
}
impl Backend for Files {
    fn fresh(&self, name: &str) -> Box<dyn JobStore> {
        let d = dir(name, self.0);
        let _ = std::fs::remove_dir_all(&d);
        Box::new(Broken(FileStore::open(d).unwrap(), self.0))
    }
    fn reopener(&self, name: &str) -> Arc<dyn Fn() -> Box<dyn JobStore>> {
        let (d, m) = (dir(name, self.0), self.0);
        Arc::new(move || Box::new(Broken(FileStore::open(&d).unwrap(), m)))
    }
}

#[test]
fn every_broken_commit_guarded_fails_the_suite() {
    let survivors: Vec<String> = [Mutant::NoGuard, Mutant::IgnoreFence, Mutant::IgnoreExpiry, Mutant::IgnoreWorker, Mutant::AllowOverwrite, Mutant::AcceptWithoutLease]
        .into_iter()
        .filter(|m| run_suite(&Files(*m)).is_empty())
        .map(|m| format!("{m:?}"))
        .collect();
    assert!(survivors.is_empty(), "executor suite does not catch: {survivors:?}");
}
