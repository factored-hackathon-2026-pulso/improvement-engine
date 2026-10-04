//! Postgres-backed engine `JobStore` (E1/E2 keys: lease, in/N, eff/N, out/N) over `pulso_job_kv` (0051).
//! `commit_guarded` is ONE transaction: `SELECT ... FOR UPDATE` on the `lease` row verifies that the committer
//! is the current, unexpired holder (the row lock blocks a concurrent reclaim until we commit), then inserts
//! out/N; the primary key `(tenant_id, job_ref, key)` makes a second out/N impossible.
use engine::{CommitGuard, JobStore};
use postgres::{Client, Config, NoTls};
use std::sync::Mutex;

pub struct PgJobStore {
    client: Mutex<Client>,
    tenant: String,
    job_ref: String,
}

fn e(err: postgres::Error) -> String {
    err.to_string()
}

fn check_key(key: &str) -> Result<(), String> {
    let ok = !key.is_empty() && key.len() <= 128 && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-'));
    if ok { Ok(()) } else { Err(format!("invalid store key {key:?}")) }
}

impl PgJobStore {
    /// `cfg` must name a database migrated through 0051; `(tenant, job_ref)` scopes the keys.
    pub fn open(cfg: &Config, tenant: &str, job_ref: &str) -> Result<Self, String> {
        if tenant.is_empty() || tenant.len() > 128 || job_ref.is_empty() || job_ref.len() > 128 {
            return Err("invalid tenant or job_ref".into());
        }
        let client = cfg.connect(NoTls).map_err(e)?;
        Ok(Self { client: Mutex::new(client), tenant: tenant.into(), job_ref: job_ref.into() })
    }
}

const INSERT: &str = "INSERT INTO pulso_job_kv (tenant_id, job_ref, key, version, value) VALUES ($1, $2, $3, 1, $4) ON CONFLICT DO NOTHING";

impl JobStore for PgJobStore {
    fn get(&self, key: &str) -> Result<Option<(u64, String)>, String> {
        check_key(key)?;
        let row = self
            .client
            .lock()
            .unwrap()
            .query_opt("SELECT version, value FROM pulso_job_kv WHERE tenant_id = $1 AND job_ref = $2 AND key = $3", &[&self.tenant, &self.job_ref, &key])
            .map_err(e)?;
        Ok(row.map(|r| (r.get::<_, i64>(0) as u64, r.get(1))))
    }

    fn cas(&self, key: &str, expected: u64, value: &str) -> Result<u64, String> {
        check_key(key)?;
        let mut c = self.client.lock().unwrap();
        let n = if expected == 0 {
            c.execute(INSERT, &[&self.tenant, &self.job_ref, &key, &value])
        } else {
            c.execute(
                "UPDATE pulso_job_kv SET version = version + 1, value = $4, updated_at = CURRENT_TIMESTAMP \
                 WHERE tenant_id = $1 AND job_ref = $2 AND key = $3 AND version = $5",
                &[&self.tenant, &self.job_ref, &key, &value, &(expected as i64)],
            )
        }
        .map_err(e)?;
        if n == 1 { Ok(expected + 1) } else { Err(format!("cas conflict on {key}: expected {expected}")) }
    }

    fn commit_guarded(&self, key: &str, value: &str, guard: &CommitGuard) -> Result<u64, String> {
        check_key(key)?;
        let mut c = self.client.lock().unwrap();
        let mut tx = c.transaction().map_err(e)?;
        let lease = tx
            .query_opt(
                "SELECT value FROM pulso_job_kv WHERE tenant_id = $1 AND job_ref = $2 AND key = 'lease' FOR UPDATE",
                &[&self.tenant, &self.job_ref],
            )
            .map_err(e)?;
        match lease {
            Some(r) if guard.holds(&r.get::<_, String>(0)) => {}
            _ => return Err("stale fence".into()),
        }
        if tx.execute(INSERT, &[&self.tenant, &self.job_ref, &key, &value]).map_err(e)? != 1 {
            return Err(format!("cas conflict on {key}: already exists"));
        }
        tx.commit().map_err(e)?;
        Ok(1)
    }
}
