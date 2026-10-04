//! Postgres `JobRepository` over MIG0 `pulso_jobs` (+ 0051 `pulso_job_outputs`).
//! - claim: ONE `UPDATE ... WHERE (tenant, id) = (SELECT ... FOR UPDATE SKIP LOCKED)`; `lease_version` is the
//!   fence token and `attempt` the claim counter, both +1 in that statement.
//! - every holder-only operation is a conditional UPDATE `WHERE lease_owner = $w AND lease_version = $fence AND
//!   lease_until > $now`; `commit_output` runs it and the `pulso_job_outputs` INSERT in ONE transaction, so a
//!   superseded worker (older fence) cannot commit however late or clock-skewed it reaches the database.
//! Time is the caller's injected unix-second clock, not the database clock (tests and kill/resume drive it).
use crate::repo::{check_ids, Claimed, JobRepository, RepoError};
use postgres::error::SqlState;
use postgres::{Client, Config, NoTls};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct PgRepo {
    cfg: Config,
    ns: String,
}

static NS: AtomicU64 = AtomicU64::new(0);
static LAST_US: AtomicU64 = AtomicU64::new(0);

fn db(e: postgres::Error) -> RepoError {
    RepoError::Store(e.to_string())
}

/// UUIDv7-shaped id (48-bit ms-ish timestamp prefix, strictly increasing in this process).
fn next_uuid7() -> String {
    let wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64);
    let mut last = LAST_US.load(Ordering::SeqCst);
    let ts = loop {
        let cand = wall.max(last + 1);
        match LAST_US.compare_exchange(last, cand, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break cand,
            Err(v) => last = v,
        }
    };
    // 48 bits of microseconds is ~8.9 years of range per epoch: wrap-safe enough for ordering within a test db,
    // and the version/variant nibbles satisfy the MIG0 CHECKs.
    let ms48 = ts & 0xffff_ffff_ffff;
    let pid = std::process::id() as u64;
    format!("{:08x}-{:04x}-7{:03x}-8{:03x}-{:012x}", ms48 >> 16, ms48 & 0xffff, pid & 0xfff, (ts >> 8) & 0xfff, (pid << 24 | NS.fetch_add(1, Ordering::SeqCst)) & 0xffff_ffff_ffff)
}

impl PgRepo {
    /// `cfg` must name a database already migrated through 0051. Each instance gets a private tenant
    /// namespace so one database can host many independent repositories.
    pub fn new(cfg: Config) -> Self {
        let ns = format!("ns{}x{}-", std::process::id(), NS.fetch_add(1, Ordering::SeqCst));
        Self { cfg, ns }
    }
    fn client(&self) -> Result<Client, RepoError> {
        self.cfg.connect(NoTls).map_err(db)
    }
    fn tenant(&self, t: &str) -> String {
        format!("{}{t}", self.ns)
    }
    fn uuid(job: &str) -> Result<uuid_text::Uuid, RepoError> {
        uuid_text::Uuid::parse(job)
    }
}

/// Minimal textual UUID handling (the `postgres` crate is used without the uuid feature: ids travel as text).
mod uuid_text {
    use crate::repo::RepoError;
    pub struct Uuid(pub String);
    impl Uuid {
        pub fn parse(s: &str) -> Result<Uuid, RepoError> {
            let ok = s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() });
            if ok { Ok(Uuid(s.to_ascii_lowercase())) } else { Err(RepoError::InvalidId(format!("job {s:?}"))) }
        }
    }
}

impl JobRepository for PgRepo {
    fn admit(&self, tenant: &str) -> Result<String, RepoError> {
        check_ids(tenant, "w")?;
        let id = next_uuid7();
        self.client()?
            .execute(
                "INSERT INTO pulso_jobs (id, tenant_id, run_ref, kind, logical_key, generation, parent_job_id, status, lane, due_at, input_ref, config_ref) \
                 VALUES ($1::text::uuid, $2, $1::text::uuid, 'conformance', $1, 0, $1::text::uuid, 'queued', 'default', CURRENT_TIMESTAMP, 'in', 'cfg')",
                &[&id, &self.tenant(tenant)],
            )
            .map_err(db)?;
        Ok(id)
    }

    fn claim_next(&self, tenant: &str, worker: &str, now: u64, lease_seconds: u64) -> Result<Option<Claimed>, RepoError> {
        check_ids(tenant, worker)?;
        if lease_seconds == 0 {
            return Err(RepoError::InvalidLeaseDuration);
        }
        let (now, expires) = (now as i64, (now + lease_seconds) as i64);
        let row = self
            .client()?
            .query_opt(
                "UPDATE pulso_jobs SET status = 'leased', lease_owner = $2, lease_until = to_timestamp($4::bigint), \
                        lease_version = lease_version + 1, attempt = attempt + 1, updated_at = CURRENT_TIMESTAMP \
                 WHERE (tenant_id, id) = (SELECT tenant_id, id FROM pulso_jobs \
                        WHERE tenant_id = $1 AND effect_state = 'no_effect' \
                          AND (status = 'queued' OR (status = 'leased' AND lease_until <= to_timestamp($3::bigint))) \
                        ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED) \
                 RETURNING id::text, lease_version, attempt",
                &[&self.tenant(tenant), &worker, &now, &expires],
            )
            .map_err(db)?;
        Ok(row.map(|r| Claimed { job: r.get(0), fence_token: r.get::<_, i64>(1) as u64, attempt: r.get::<_, i32>(2) as u64, expires_at: expires as u64 }))
    }

    fn touch_lease(&self, tenant: &str, job: &str, worker: &str, fence: u64, now: u64, lease_seconds: u64) -> Result<u64, RepoError> {
        check_ids(tenant, worker)?;
        if lease_seconds == 0 {
            return Err(RepoError::InvalidLeaseDuration);
        }
        let id = Self::uuid(job)?.0;
        let expires = (now + lease_seconds) as i64;
        let n = self
            .client()?
            .execute(
                "UPDATE pulso_jobs SET lease_until = to_timestamp($6::bigint), updated_at = CURRENT_TIMESTAMP \
                 WHERE tenant_id = $1 AND id = $2::text::uuid AND status = 'leased' AND lease_owner = $3 \
                   AND lease_version = $4 AND lease_until > to_timestamp($5::bigint)",
                &[&self.tenant(tenant), &id, &worker, &(fence as i64), &(now as i64), &expires],
            )
            .map_err(db)?;
        if n == 1 { Ok(expires as u64) } else { Err(RepoError::StaleFence) }
    }

    fn begin_effect(&self, tenant: &str, job: &str, worker: &str, fence: u64, now: u64) -> Result<(), RepoError> {
        check_ids(tenant, worker)?;
        let id = Self::uuid(job)?.0;
        let n = self
            .client()?
            .execute(
                "UPDATE pulso_jobs SET effect_state = 'unknown_pending_reconciliation', updated_at = CURRENT_TIMESTAMP \
                 WHERE tenant_id = $1 AND id = $2::text::uuid AND status = 'leased' AND lease_owner = $3 \
                   AND lease_version = $4 AND lease_until > to_timestamp($5::bigint)",
                &[&self.tenant(tenant), &id, &worker, &(fence as i64), &(now as i64)],
            )
            .map_err(db)?;
        if n == 1 { Ok(()) } else { Err(RepoError::StaleFence) }
    }

    fn commit_output(&self, tenant: &str, job: &str, step: u32, worker: &str, fence: u64, now: u64, record: &str) -> Result<(), RepoError> {
        check_ids(tenant, worker)?;
        let id = Self::uuid(job)?.0;
        let t = self.tenant(tenant);
        let mut c = self.client()?;
        let mut tx = c.transaction().map_err(db)?;
        // The conditional UPDATE both verifies the CURRENT fence + unexpired lease and row-locks the job until
        // commit, so a concurrent reclaim either happened first (0 rows here) or waits behind this commit.
        let n = tx
            .execute(
                "UPDATE pulso_jobs SET updated_at = CURRENT_TIMESTAMP \
                 WHERE tenant_id = $1 AND id = $2::text::uuid AND status = 'leased' AND lease_owner = $3 \
                   AND lease_version = $4 AND lease_until > to_timestamp($5::bigint)",
                &[&t, &id, &worker, &(fence as i64), &(now as i64)],
            )
            .map_err(db)?;
        if n != 1 {
            return Err(RepoError::StaleFence);
        }
        match tx.execute(
            "INSERT INTO pulso_job_outputs (tenant_id, job_id, step_index, fence_token, worker_id, record) VALUES ($1, $2::text::uuid, $3, $4, $5, $6)",
            &[&t, &id, &(step as i32), &(fence as i64), &worker, &record],
        ) {
            Ok(_) => tx.commit().map_err(db),
            Err(e) if e.code() == Some(&SqlState::UNIQUE_VIOLATION) => Err(RepoError::Conflict(format!("out/{step} exists"))),
            Err(e) => Err(db(e)),
        }
    }

    fn output(&self, tenant: &str, job: &str, step: u32) -> Result<Option<String>, RepoError> {
        let id = Self::uuid(job)?.0;
        let row = self
            .client()?
            .query_opt(
                "SELECT record FROM pulso_job_outputs WHERE tenant_id = $1 AND job_id = $2::text::uuid AND step_index = $3",
                &[&self.tenant(tenant), &id, &(step as i32)],
            )
            .map_err(db)?;
        Ok(row.map(|r| r.get(0)))
    }
}
