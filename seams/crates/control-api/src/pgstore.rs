//! Postgres-backed `Store` (CPG) over migration 0052 (`pulso_ca_*`). Every statement is parameterised; every tenant-owned
//! table is read and written by `(tenant_id, ...)`, and the primary keys / unique constraints are the single-winner
//! guarantees, so concurrent writers (threads or processes) cannot double-bind, overwrite or cross tenants.
//!
//! The `Store` port is infallible: a database failure inside a call panics that request's thread (no response is sent,
//! nothing half-written: each call is one statement or one transaction) instead of answering from a guess.
use crate::store::{BindingRec, PutOutcome, Store};
use postgres::{Client, Config, NoTls};
use serde_json::Value;
use std::fmt;
use std::sync::Mutex;

/// A connection URL that may carry a password: `Debug` is redacted and the value is never printed by this crate.
#[derive(Clone)]
pub struct DatabaseUrl(String);

impl DatabaseUrl {
    pub fn new(url: impl Into<String>) -> DatabaseUrl {
        DatabaseUrl(url.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DatabaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DatabaseUrl(<redacted>)")
    }
}

const MIGRATION_ID: &str = "0052_pulso_control_api";
const MIGRATION_SQL: &str = include_str!("../../../../migrations/0052_pulso_control_api.sql");
const MIGRATION_54_ID: &str = "0054_pulso_ca_docs_release_ns";
const MIGRATION_54_SQL: &str = include_str!("../../../../migrations/0054_pulso_ca_docs_release_ns.sql");

pub struct PgStore {
    config: Config,
    client: Mutex<Client>,
}

impl fmt::Debug for PgStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PgStore { .. }")
    }
}

impl PgStore {
    /// Connects, applies migrations 0052 and 0054 through the MIG0 runner (idempotent, advisory-locked) and returns the store.
    /// Errors never contain the URL.
    pub fn connect(url: &str) -> Result<PgStore, String> {
        let config: Config = url.parse().map_err(|_| "invalid database url".to_string())?;
        let mut client = config.connect(NoTls).map_err(|e| format!("database connect failed: {e}"))?;
        let sql = MIGRATION_SQL.replace("\r\n", "\n");
        let migration = pg::Migration { id: MIGRATION_ID.into(), version: 52, checksum: pg::migrate::checksum(&sql), sql };
        let sql54 = MIGRATION_54_SQL.replace("
", "
");
        let m54 = pg::Migration { id: MIGRATION_54_ID.into(), version: 54, checksum: pg::migrate::checksum(&sql54), sql: sql54 };
        pg::migrate::migrate(&mut client, &[migration, m54]).map_err(|e| format!("migration failed: {e}"))?;
        Ok(PgStore { config, client: Mutex::new(client) })
    }

    pub fn connect_url(url: &DatabaseUrl) -> Result<PgStore, String> {
        PgStore::connect(url.expose())
    }

    /// Empties every `pulso_ca_*` table. Test support only: the server binary calls it solely when
    /// `CONTROL_API_DATABASE_FRESH=1` together with the loopback-only admin channel, so a black-box run starts clean.
    pub fn reset(&self) {
        self.with(|c| c.batch_execute("TRUNCATE pulso_ca_bindings, pulso_ca_binding_refs, pulso_ca_artifacts, pulso_ca_docs, pulso_ca_jti"));
    }

    fn with<R>(&self, f: impl FnOnce(&mut Client) -> Result<R, postgres::Error>) -> R {
        let mut g = self.client.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if g.is_closed() {
            *g = self.config.connect(NoTls).unwrap_or_else(|e| panic!("database reconnect failed: {e}"));
        }
        f(&mut g).unwrap_or_else(|e| panic!("database error: {e}"))
    }
}

impl Store for PgStore {
    fn binding(&self, tenant: &str, command_key: &str) -> Option<BindingRec> {
        self.with(|c| {
            let row = c.query_opt(
                "SELECT request_digest, job_id, core_run_id, attempt, task_binding_ref FROM pulso_ca_bindings WHERE tenant_id = $1 AND command_key = $2",
                &[&tenant, &command_key],
            )?;
            Ok(row.map(|r| BindingRec { request_digest: r.get(0), job_id: r.get(1), core_run_id: r.get(2), attempt: r.get(3), task_binding_ref: r.get(4) }))
        })
    }

    fn job_owner(&self, tenant: &str, job_id: &str) -> Option<String> {
        self.with(|c| Ok(c.query_opt("SELECT command_key FROM pulso_ca_bindings WHERE tenant_id = $1 AND job_id = $2", &[&tenant, &job_id])?.map(|r| r.get(0))))
    }

    fn put_binding(&self, tenant: &str, command_key: &str, rec: BindingRec) -> bool {
        self.with(|c| {
            let mut tx = c.transaction()?;
            tx.execute("INSERT INTO pulso_ca_binding_refs (binding_ref, tenant_id) VALUES ($1, $2) ON CONFLICT DO NOTHING", &[&rec.task_binding_ref, &tenant])?;
            let owner: String = tx.query_one("SELECT tenant_id FROM pulso_ca_binding_refs WHERE binding_ref = $1", &[&rec.task_binding_ref])?.get(0);
            if owner != tenant {
                tx.rollback()?;
                return Ok(false);
            }
            // PRIMARY KEY (tenant, command_key) and UNIQUE (tenant, job_id): whoever inserts first is the single winner.
            let n = tx.execute(
                "INSERT INTO pulso_ca_bindings (tenant_id, command_key, request_digest, job_id, core_run_id, attempt, task_binding_ref) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING",
                &[&tenant, &command_key, &rec.request_digest, &rec.job_id, &rec.core_run_id, &rec.attempt, &rec.task_binding_ref],
            )?;
            if n == 0 {
                tx.rollback()?;
                return Ok(false);
            }
            tx.commit()?;
            Ok(true)
        })
    }

    fn binding_ref_tenant(&self, binding_ref: &str) -> Option<String> {
        self.with(|c| Ok(c.query_opt("SELECT tenant_id FROM pulso_ca_binding_refs WHERE binding_ref = $1", &[&binding_ref])?.map(|r| r.get(0))))
    }

    fn preauthorize_binding_ref(&self, binding_ref: &str, tenant: &str) {
        self.with(|c| c.execute("INSERT INTO pulso_ca_binding_refs (binding_ref, tenant_id) VALUES ($1, $2) ON CONFLICT DO NOTHING", &[&binding_ref, &tenant]).map(|_| ()));
    }

    fn binding_effects(&self, tenant: &str, job_id: &str) -> u32 {
        self.with(|c| {
            let n: i64 = c.query_one("SELECT count(*) FROM pulso_ca_bindings WHERE tenant_id = $1 AND job_id = $2", &[&tenant, &job_id])?.get(0);
            Ok(u32::try_from(n).unwrap_or(u32::MAX))
        })
    }

    fn put_artifact(&self, tenant: &str, envelope: Value) -> PutOutcome {
        let id = envelope["artifact"]["id"].as_str().unwrap_or_default().to_string();
        if id.is_empty() {
            return PutOutcome::Conflict; // an artifact without an id cannot be addressed
        }
        let rf = envelope["artifact"].clone();
        self.with(|c| {
            let mut tx = c.transaction()?;
            let n = tx.execute(
                "INSERT INTO pulso_ca_artifacts (tenant_id, artifact_id, artifact_ref, envelope) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                &[&tenant, &id, &rf, &envelope],
            )?;
            if n == 1 {
                tx.commit()?;
                return Ok(PutOutcome::Created);
            }
            let prior: Value = tx.query_one("SELECT artifact_ref FROM pulso_ca_artifacts WHERE tenant_id = $1 AND artifact_id = $2", &[&tenant, &id])?.get(0);
            tx.rollback()?;
            Ok(if prior == rf { PutOutcome::Exists } else { PutOutcome::Conflict })
        })
    }

    fn get_artifact(&self, tenant: &str, id: &str) -> Option<Value> {
        self.with(|c| Ok(c.query_opt("SELECT envelope FROM pulso_ca_artifacts WHERE tenant_id = $1 AND artifact_id = $2", &[&tenant, &id])?.map(|r| r.get(0))))
    }

    fn put_doc(&self, ns: &str, tenant: &str, id: &str, doc: Value) {
        self.with(|c| {
            c.execute(
                "INSERT INTO pulso_ca_docs (ns, tenant_id, doc_id, doc) VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (ns, tenant_id, doc_id) DO UPDATE SET doc = EXCLUDED.doc, updated_at = CURRENT_TIMESTAMP",
                &[&ns, &tenant, &id, &doc],
            )
            .map(|_| ())
        });
    }

    fn put_doc_new(&self, ns: &str, tenant: &str, id: &str, doc: Value) -> bool {
        self.with(|c| {
            let n = c.execute("INSERT INTO pulso_ca_docs (ns, tenant_id, doc_id, doc) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING", &[&ns, &tenant, &id, &doc])?;
            Ok(n == 1)
        })
    }

    fn get_doc(&self, ns: &str, tenant: &str, id: &str) -> Option<Value> {
        self.with(|c| Ok(c.query_opt("SELECT doc FROM pulso_ca_docs WHERE ns = $1 AND tenant_id = $2 AND doc_id = $3", &[&ns, &tenant, &id])?.map(|r| r.get(0))))
    }

    fn list_docs(&self, ns: &str, tenant: &str) -> Vec<(String, Value)> {
        self.with(|c| {
            let rows = c.query("SELECT doc_id, doc FROM pulso_ca_docs WHERE ns = $1 AND tenant_id = $2 ORDER BY doc_id COLLATE \"C\"", &[&ns, &tenant])?;
            Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
        })
    }

    fn jti_claim(&self, scope: &str, iss: &str, jti: &str, exp: f64, now: f64) -> bool {
        self.with(|c| {
            let mut tx = c.transaction()?;
            tx.execute("DELETE FROM pulso_ca_jti WHERE expires_at + $2 <= $1", &[&now, &crate::store::JTI_SKEW_SECS])?; // a token past exp is refused earlier: dead weight
            let n = tx.execute("INSERT INTO pulso_ca_jti (scope, iss, jti, expires_at) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING", &[&scope, &iss, &jti, &exp])?;
            tx.commit()?;
            Ok(n == 1)
        })
    }

    fn durable_replay(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_url_and_store_debug_never_show_the_password() {
        let url = DatabaseUrl::new("postgres://u:hunter2-secret@127.0.0.1:5432/db");
        assert!(!format!("{url:?}").contains("hunter2"));
        assert_eq!(format!("{url:?}"), "DatabaseUrl(<redacted>)");
    }

    #[test]
    fn connect_errors_never_echo_the_url() {
        for bad in ["postgres://u:hunter2-secret@127.0.0.1:1/db?connect_timeout=1", "not a url hunter2-secret", "postgres://u:hunter2-secret@[bad/db"] {
            let e = PgStore::connect(bad).err().expect("must fail");
            assert!(!e.contains("hunter2"), "{e}");
        }
    }
}
