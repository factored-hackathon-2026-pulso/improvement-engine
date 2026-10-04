//! Database side of `pulso run`: migrations embedded in the binary, applied on start under the pg crate's
//! advisory lock (a second task blocks, then finds everything recorded and applies nothing), and the readiness probe.
use crate::config::Secret;
use crate::health::DbProbe;
use pg::migrate::{Migration, Report, checksum, parse_name};
use postgres::{Client, Config, NoTls};
use std::sync::Mutex;
use std::time::Duration;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/migrations.rs"));
}

/// The repo `migrations/*.sql`, as embedded at build time, ordered by (version, id).
pub fn migrations() -> Result<Vec<Migration>, String> {
    let mut out = Vec::new();
    for (file, sql) in embedded::MIGRATIONS {
        let (version, id) = parse_name(file).map_err(|e| format!("embedded migration {file}: {e}"))?;
        let sql = sql.replace("\r\n", "\n");
        out.push(Migration { id, version, checksum: checksum(&sql), sql });
    }
    out.sort_by(|a, b| (a.version, &a.id).cmp(&(b.version, &b.id)));
    Ok(out)
}

/// Parses the DSN; the error names the variable and never contains the DSN.
pub fn pg_config(dsn: &Secret) -> Result<Config, String> {
    let mut cfg: Config = dsn.expose().parse().map_err(|_| "PULSO_DATABASE_URL is not a valid Postgres connection string (value not shown)".to_string())?;
    cfg.connect_timeout(Duration::from_secs(5));
    Ok(cfg)
}

/// Applies every unrecorded migration. Safe to run from several tasks at once.
pub fn apply(cfg: &Config) -> Result<Report, String> {
    let mut client = cfg.connect(NoTls).map_err(|e| format!("database connect failed: {:?}", e.as_db_error().map(|d| d.code().code().to_string())))?;
    let ms = migrations()?;
    pg::migrate::migrate(&mut client, &ms).map_err(|e| match e {
        pg::migrate::Error::ChecksumMismatch { id, .. } => format!("migration {id} was edited after it was applied"),
        // `Error::Db` is "<migration id>: <server text>" for a failing migration; the server text can carry row values, the id cannot.
        pg::migrate::Error::Db(m) => match m.split_once(": ") {
            Some((id, _)) if id.len() > 5 && id[..4].bytes().all(|b| b.is_ascii_digit()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') => format!("migration {id} failed in the database"),
            _ => "migration failed in the database".to_string(),
        },
        other => format!("migration error: {other:?}"),
    })
}

/// Readiness probe: one cached connection, `SELECT 1`, reconnect on failure.
pub struct PgProbe {
    cfg: Config,
    client: Mutex<Option<Client>>,
}

impl PgProbe {
    pub fn new(cfg: Config) -> PgProbe {
        PgProbe { cfg, client: Mutex::new(None) }
    }
}

impl DbProbe for PgProbe {
    fn ping(&self) -> Result<(), String> {
        let mut slot = self.client.lock().unwrap();
        if let Some(c) = slot.as_mut() {
            if c.simple_query("SELECT 1").is_ok() {
                return Ok(());
            }
            *slot = None;
        }
        let mut c = self.cfg.connect(NoTls).map_err(|_| "unreachable".to_string())?;
        c.simple_query("SELECT 1").map_err(|_| "query failed".to_string())?;
        *slot = Some(c);
        Ok(())
    }
}
