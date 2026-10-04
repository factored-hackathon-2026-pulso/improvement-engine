//! Least-privilege role bootstrap. Role names follow `local/core/init/00-roles-and-dbs.sql`
//! (`core_app`, `core_eval_app`, `exporter_ro`). Passwords come from the environment only.
//! Run after `migrate::migrate` so the `pulso_*` tables and functions exist; idempotent.
use postgres::Client;
use std::fmt;

pub const ENV_CORE_APP: &str = "PULSO_PG_CORE_APP_PASSWORD";
pub const ENV_CORE_EVAL_APP: &str = "PULSO_PG_CORE_EVAL_APP_PASSWORD";
pub const ENV_EXPORTER_RO: &str = "PULSO_PG_EXPORTER_RO_PASSWORD";

#[derive(Debug)]
pub enum Error {
    MissingEnv(&'static str),
    Db(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MissingEnv(k) => write!(f, "environment variable {k} is not set"),
            Error::Db(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<postgres::Error> for Error {
    fn from(e: postgres::Error) -> Self {
        Error::Db(e.to_string())
    }
}

/// Role passwords; `Debug` is redacted and there is no `Clone`, so they do not reach logs.
pub struct RoleSecrets {
    pub core_app: String,
    pub core_eval_app: String,
    pub exporter_ro: String,
}

impl fmt::Debug for RoleSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RoleSecrets(<redacted>)")
    }
}

impl RoleSecrets {
    pub fn from_env() -> Result<Self, Error> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let need = |k: &'static str| get(k).filter(|v| !v.is_empty()).ok_or(Error::MissingEnv(k));
        Ok(Self { core_app: need(ENV_CORE_APP)?, core_eval_app: need(ENV_CORE_EVAL_APP)?, exporter_ro: need(ENV_EXPORTER_RO)? })
    }
}

fn upsert_role(c: &mut Client, role: &str, secret: &str) -> Result<(), Error> {
    let exists: bool = c.query_one("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)", &[&role])?.get(0);
    let verb = if exists { "ALTER" } else { "CREATE" };
    // format() quotes the identifier and the literal server-side; nothing is interpolated client-side.
    let sql: String = c
        .query_one(
            &format!("SELECT format('{verb} ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD %L', $1::text, $2::text)"),
            &[&role, &secret],
        )?
        .get(0);
    c.batch_execute(&sql)?;
    Ok(())
}

/// Create or refresh the three roles and apply least-privilege grants on the current database.
pub fn bootstrap(c: &mut Client, s: &RoleSecrets) -> Result<(), Error> {
    upsert_role(c, "core_app", &s.core_app)?;
    upsert_role(c, "core_eval_app", &s.core_eval_app)?;
    upsert_role(c, "exporter_ro", &s.exporter_ro)?;
    let db: String = c.query_one("SELECT quote_ident(current_database())", &[])?.get(0);
    c.batch_execute(&format!(
        "REVOKE ALL ON DATABASE {db} FROM PUBLIC;
         GRANT CONNECT ON DATABASE {db} TO core_app, core_eval_app, exporter_ro;
         REVOKE CREATE ON SCHEMA public FROM PUBLIC;
         GRANT USAGE ON SCHEMA public TO core_app, exporter_ro;"
    ))?;
    // Table grants are per pulso_* table (the runner's own bookkeeping table is excluded).
    let tables: Vec<String> = c
        .query(
            "SELECT quote_ident(tablename) FROM pg_tables WHERE schemaname = 'public' \
             AND left(tablename, 6) = 'pulso_' AND tablename <> $1",
            &[&crate::schema::MIGRATIONS_TABLE],
        )?
        .iter()
        .map(|r| r.get(0))
        .collect();
    for t in tables {
        c.batch_execute(&format!(
            "REVOKE ALL ON {t} FROM core_app, core_eval_app, exporter_ro;
             GRANT SELECT, INSERT, UPDATE ON {t} TO core_app;
             GRANT SELECT ON {t} TO exporter_ro;"
        ))?;
    }
    c.batch_execute(
        "GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA public TO core_app;
         REVOKE ALL ON pulso_schema_migrations FROM core_app, core_eval_app, exporter_ro;",
    )?;
    Ok(())
}
