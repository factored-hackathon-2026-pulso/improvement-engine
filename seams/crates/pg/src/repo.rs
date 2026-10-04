//! Job repository port (C-7 v1.1 claim-next shape) plus the in-memory reference implementation.
//! Mirrors (never depends on) crates/core durable_jobs.rs: a lease is reclaimable when `now >= expires`,
//! every claim bumps `fence_token` and `attempt` by one, a job whose effect state is not NoEffect is never
//! claimable, and only the holder of the CURRENT fence with an unexpired lease may touch/begin-effect/commit.
//! Deviation from the frozen signature: methods take `&self` (interior mutability) so threads can race.
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimed {
    pub job: String,
    pub fence_token: u64,
    pub attempt: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoError {
    InvalidLeaseDuration,
    InvalidId(String),
    /// Not the current fence holder, wrong worker, or lease expired.
    StaleFence,
    /// Unique-constraint violation (an output for that step already exists).
    Conflict(String),
    Store(String),
}

pub trait JobRepository: Send + Sync {
    /// Admit a Queued job; ids are handed out in admission order. Returns the job id.
    fn admit(&self, tenant: &str) -> Result<String, RepoError>;
    /// Oldest Queued job, or Leased with `now >= expires`, with effect state NoEffect, as ONE conditional update.
    fn claim_next(&self, tenant: &str, worker: &str, now: u64, lease_seconds: u64) -> Result<Option<Claimed>, RepoError>;
    /// Renew the lease (same fence). Returns the new expiry.
    fn touch_lease(&self, tenant: &str, job: &str, worker: &str, fence: u64, now: u64, lease_seconds: u64) -> Result<u64, RepoError>;
    /// Record that an external effect is about to be dispatched: the job is then never claimable again.
    fn begin_effect(&self, tenant: &str, job: &str, worker: &str, fence: u64, now: u64) -> Result<(), RepoError>;
    /// Commit out/`step` iff `fence` is current and the lease unexpired at `now`; one winner per step.
    fn commit_output(&self, tenant: &str, job: &str, step: u32, worker: &str, fence: u64, now: u64, record: &str) -> Result<(), RepoError>;
    fn output(&self, tenant: &str, job: &str, step: u32) -> Result<Option<String>, RepoError>;
}

pub fn check_ids(tenant: &str, worker: &str) -> Result<(), RepoError> {
    let bad = |s: &str| s.is_empty() || s.len() > 100 || s.chars().any(|c| c.is_control() || c == '|');
    if bad(tenant) {
        return Err(RepoError::InvalidId(format!("tenant {tenant:?}")));
    }
    if bad(worker) {
        return Err(RepoError::InvalidId(format!("worker {worker:?}")));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct Job {
    tenant: String,
    id: String,
    leased: bool,
    worker: String,
    fence: u64,
    attempt: u64,
    expires: u64,
    effect: bool,
    outputs: BTreeMap<u32, String>,
}

/// Deliberate defects for mutation-testing the conformance suite (`MemRepo::with_fault`); `None` is correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fault {
    #[default]
    None,
    /// holder-only operations accept any fence (stale-fence commit)
    IgnoreFence,
    /// holder-only operations accept a holder past its expiry (lost lease)
    IgnoreExpiry,
    /// holder-only operations accept any worker name
    IgnoreWorker,
    /// claim also takes jobs whose effect state is not NoEffect
    ClaimIgnoresEffect,
    /// `begin_effect` returns Ok but records nothing
    EffectNotRecorded,
    /// a reclaim does not bump `attempt`
    AttemptNotBumped,
    /// a reclaim bumps `attempt` twice
    AttemptBumpedTwice,
    /// a reclaim does not bump the fence
    FenceNotBumped,
    /// reclaim only when `now > expires` (off by one)
    ReclaimOffByOne,
    /// a live lease is reclaimable
    ReclaimLiveLease,
    /// claim picks the newest job
    ClaimNewestFirst,
    /// claim ignores the tenant
    ClaimIgnoresTenant,
    /// holder-only operations ignore the tenant
    ForeignTenantOps,
    /// commit_output overwrites an existing out/N instead of conflicting
    OverwriteOutput,
    /// touch_lease reports success but does not move the expiry
    TouchNoExtend,
}

impl Job {
    fn holds(&self, f: Fault, worker: &str, fence: u64, now: u64) -> bool {
        self.leased
            && (f == Fault::IgnoreWorker || self.worker == worker)
            && (f == Fault::IgnoreFence || self.fence == fence)
            && (f == Fault::IgnoreExpiry || now < self.expires)
    }
    fn claimable(&self, f: Fault, now: u64) -> bool {
        let expired = match f {
            Fault::ReclaimOffByOne => now > self.expires,
            Fault::ReclaimLiveLease => true,
            _ => now >= self.expires,
        };
        (f == Fault::ClaimIgnoresEffect || !self.effect) && (!self.leased || expired)
    }
}

#[derive(Default)]
pub struct MemRepo {
    jobs: Mutex<Vec<Job>>,
    fault: Fault,
}

impl MemRepo {
    pub fn new() -> Self {
        Self::default()
    }
    /// A repository with one deliberate defect, to prove the conformance suite catches it.
    pub fn with_fault(fault: Fault) -> Self {
        Self { fault, ..Self::default() }
    }
    /// Oldest claimable job id (read only). Public so a deliberately broken repository can be built in tests.
    pub fn peek_candidate(&self, tenant: &str, now: u64) -> Option<String> {
        let jobs = self.jobs.lock().unwrap();
        jobs.iter().find(|j| j.tenant == tenant && j.claimable(self.fault, now)).map(|j| j.id.clone())
    }
    /// Unconditional lease write (no re-check): the building block of a read-then-write bug.
    pub fn force_claim(&self, tenant: &str, job: &str, worker: &str, now: u64, lease: u64) -> Claimed {
        let mut jobs = self.jobs.lock().unwrap();
        let j = jobs.iter_mut().find(|j| j.tenant == tenant && j.id == job).unwrap();
        Self::lease_it(self.fault, j, worker, now, lease)
    }
    fn lease_it(f: Fault, j: &mut Job, worker: &str, now: u64, lease: u64) -> Claimed {
        j.leased = true;
        j.worker = worker.into();
        if f != Fault::FenceNotBumped {
            j.fence += 1;
        }
        if f != Fault::AttemptNotBumped {
            j.attempt += if f == Fault::AttemptBumpedTwice { 2 } else { 1 };
        }
        j.expires = now + lease;
        Claimed { job: j.id.clone(), fence_token: j.fence, attempt: j.attempt, expires_at: j.expires }
    }
    fn with_holder<T>(
        &self,
        tenant: &str,
        job: &str,
        worker: &str,
        fence: u64,
        now: u64,
        f: impl FnOnce(&mut Job, Fault) -> Result<T, RepoError>,
    ) -> Result<T, RepoError> {
        check_ids(tenant, worker)?;
        let mut jobs = self.jobs.lock().unwrap();
        let fault = self.fault;
        match jobs.iter_mut().find(|j| (fault == Fault::ForeignTenantOps || j.tenant == tenant) && j.id == job) {
            Some(j) if j.holds(fault, worker, fence, now) => f(j, fault),
            _ => Err(RepoError::StaleFence),
        }
    }
}

impl JobRepository for MemRepo {
    fn admit(&self, tenant: &str) -> Result<String, RepoError> {
        check_ids(tenant, "w")?;
        let mut jobs = self.jobs.lock().unwrap();
        let id = format!("job-{}", jobs.len());
        jobs.push(Job {
            tenant: tenant.into(),
            id: id.clone(),
            leased: false,
            worker: String::new(),
            fence: 0,
            attempt: 0,
            expires: 0,
            effect: false,
            outputs: BTreeMap::new(),
        });
        Ok(id)
    }
    fn claim_next(&self, tenant: &str, worker: &str, now: u64, lease_seconds: u64) -> Result<Option<Claimed>, RepoError> {
        check_ids(tenant, worker)?;
        if lease_seconds == 0 {
            return Err(RepoError::InvalidLeaseDuration);
        }
        // selection and lease are one critical section (no read-then-write)
        let mut jobs = self.jobs.lock().unwrap();
        let fault = self.fault;
        let mut it = jobs.iter_mut().filter(|j| (fault == Fault::ClaimIgnoresTenant || j.tenant == tenant) && j.claimable(fault, now));
        let Some(j) = (if fault == Fault::ClaimNewestFirst { it.last() } else { it.next() }) else { return Ok(None) };
        Ok(Some(Self::lease_it(fault, j, worker, now, lease_seconds)))
    }
    fn touch_lease(&self, tenant: &str, job: &str, worker: &str, fence: u64, now: u64, lease_seconds: u64) -> Result<u64, RepoError> {
        if lease_seconds == 0 {
            return Err(RepoError::InvalidLeaseDuration);
        }
        self.with_holder(tenant, job, worker, fence, now, |j, f| {
            if f != Fault::TouchNoExtend {
                j.expires = now + lease_seconds;
            }
            Ok(now + lease_seconds)
        })
    }
    fn begin_effect(&self, tenant: &str, job: &str, worker: &str, fence: u64, now: u64) -> Result<(), RepoError> {
        self.with_holder(tenant, job, worker, fence, now, |j, f| {
            j.effect = f != Fault::EffectNotRecorded;
            Ok(())
        })
    }
    fn commit_output(&self, tenant: &str, job: &str, step: u32, worker: &str, fence: u64, now: u64, record: &str) -> Result<(), RepoError> {
        self.with_holder(tenant, job, worker, fence, now, |j, f| {
            if f != Fault::OverwriteOutput && j.outputs.contains_key(&step) {
                return Err(RepoError::Conflict(format!("out/{step} exists")));
            }
            j.outputs.insert(step, record.into());
            Ok(())
        })
    }
    fn output(&self, tenant: &str, job: &str, step: u32) -> Result<Option<String>, RepoError> {
        let jobs = self.jobs.lock().unwrap();
        Ok(jobs.iter().find(|j| j.tenant == tenant && j.id == job).and_then(|j| j.outputs.get(&step).cloned()))
    }
}
