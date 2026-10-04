//! Idempotent migration runner. Files are `NNNN_<slug>.sql`; order is (version, file name), so
//! several files may share a version (the three `0002_*`) and gaps are tolerated. Each migration
//! runs in its own transaction and is recorded with a SHA-256 of its LF-normalised bytes.
use postgres::Client;
use sha2::{Digest, Sha256};
use std::{fmt, fs, path::Path};

use crate::schema::MIGRATIONS_TABLE;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Io(String),
    BadName(String),
    Db(String),
    ChecksumMismatch { id: String, recorded: String, current: String },
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<postgres::Error> for Error {
    fn from(e: postgres::Error) -> Self {
        Error::Db(e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Migration {
    pub id: String,
    pub version: u32,
    pub checksum: String,
    pub sql: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
}

pub fn checksum(sql: &str) -> String {
    let normalised = sql.replace("\r\n", "\n");
    Sha256::digest(normalised.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse `NNNN_slug.sql` into (version, id).
pub fn parse_name(file: &str) -> Result<(u32, String), Error> {
    let bad = || Error::BadName(file.to_owned());
    let stem = file.strip_suffix(".sql").ok_or_else(bad)?;
    let (num, slug) = stem.split_once('_').ok_or_else(bad)?;
    let slug_ok = !slug.is_empty() && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if num.len() != 4 || !num.bytes().all(|b| b.is_ascii_digit()) || !slug_ok {
        return Err(bad());
    }
    Ok((num.parse().map_err(|_| bad())?, stem.to_owned()))
}

/// Read every `*.sql` in `dir`, ordered by (version, id).
pub fn discover(dir: &Path) -> Result<Vec<Migration>, Error> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| Error::Io(e.to_string()))? {
        let path = entry.map_err(|e| Error::Io(e.to_string()))?.path();
        if path.extension().is_none_or(|e| e != "sql") {
            continue;
        }
        let file = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        let (version, id) = parse_name(&file)?;
        let sql = fs::read_to_string(&path).map_err(|e| Error::Io(format!("{file}: {e}")))?;
        out.push(Migration { id, version, checksum: checksum(&sql), sql });
    }
    out.sort_by(|a, b| (a.version, &a.id).cmp(&(b.version, &b.id)));
    Ok(out)
}

const LOCK_KEY: i64 = 0x7075_6c73_6f5f_6d67; // "pulso_mg"

/// Apply every unrecorded migration in order; verify the checksum of recorded ones.
pub fn migrate(client: &mut Client, migrations: &[Migration]) -> Result<Report, Error> {
    client.execute("SELECT pg_advisory_lock($1)", &[&LOCK_KEY])?;
    let result = run(client, migrations);
    let _ = client.execute("SELECT pg_advisory_unlock($1)", &[&LOCK_KEY]);
    result
}

fn run(client: &mut Client, migrations: &[Migration]) -> Result<Report, Error> {
    client.batch_execute(&format!(
        "CREATE TABLE IF NOT EXISTS {MIGRATIONS_TABLE} (\
           id TEXT PRIMARY KEY, version INTEGER NOT NULL, \
           checksum TEXT NOT NULL CHECK (checksum ~ '^[0-9a-f]{{64}}$'), \
           applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP)"
    ))?;
    let mut report = Report::default();
    for m in migrations {
        let recorded: Option<String> = client
            .query_opt(&format!("SELECT checksum FROM {MIGRATIONS_TABLE} WHERE id = $1"), &[&m.id])?
            .map(|r| r.get(0));
        match recorded {
            Some(r) if r == m.checksum => report.skipped.push(m.id.clone()),
            Some(r) => return Err(Error::ChecksumMismatch { id: m.id.clone(), recorded: r, current: m.checksum.clone() }),
            None => {
                let mut tx = client.transaction()?;
                tx.batch_execute(&m.sql).map_err(|e| Error::Db(format!("{}: {e}", m.id)))?;
                tx.execute(
                    &format!("INSERT INTO {MIGRATIONS_TABLE} (id, version, checksum) VALUES ($1, $2, $3)"),
                    &[&m.id, &(m.version as i32), &m.checksum],
                )?;
                tx.commit()?;
                report.applied.push(m.id.clone());
            }
        }
    }
    Ok(report)
}
