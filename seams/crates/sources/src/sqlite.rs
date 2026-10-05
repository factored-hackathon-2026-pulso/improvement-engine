//! `product-sqlite`: the platform's SQLite file, opened read-only (`SQLITE_OPEN_READ_ONLY`, `PRAGMA query_only`) with an
//! engine-level authorizer that denies every write and every read outside the allow-list (defence in depth over the
//! allow-list-built queries, as `policy.py::install_sqlite_guard`).
use crate::policy::{self, DENIED_TABLES, EVENT_READ_COLUMNS};
use crate::{Batch, PlatformEvent, Row, SchemaReport, SourceAdapter, SourceError, SourceId, Watermark};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct SqliteProduct {
    id: SourceId,
    path: PathBuf,
    conn: Connection,
}

fn io<E: std::fmt::Display>(e: E) -> SourceError {
    SourceError::Io(e.to_string())
}

fn open_ro(path: &Path) -> Result<Connection, SourceError> {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(io)?;
    c.execute_batch("PRAGMA query_only = ON").map_err(io)?;
    Ok(c)
}

fn guard(ctx: AuthContext<'_>) -> Authorization {
    match ctx.action {
        AuthAction::Select | AuthAction::Function { .. } | AuthAction::Recursive => Authorization::Allow,
        AuthAction::Read { table_name, column_name } => {
            let t = table_name.to_ascii_lowercase();
            if t == "sqlite_master" || t == "sqlite_schema" {
                return Authorization::Allow; // schema names only
            }
            let cols = if t == "event_log" { Some(EVENT_READ_COLUMNS) } else { policy::allowed_columns(&t) };
            if DENIED_TABLES.contains(&t.as_str()) || cols.is_none_or(|c| !column_name.is_empty() && !c.contains(&column_name.to_ascii_lowercase().as_str())) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }
        _ => Authorization::Deny,
    }
}

impl SqliteProduct {
    pub fn open(path: &Path, id: SourceId) -> Result<SqliteProduct, SourceError> {
        let conn = open_ro(path)?;
        conn.authorizer(Some(guard)).map_err(io)?;
        Ok(SqliteProduct { id, path: path.to_owned(), conn })
    }

    /// True when the guarded connection could create a table (it must not).
    pub fn can_write(&self) -> bool {
        self.conn.execute_batch("CREATE TABLE _probe(x)").is_ok()
    }

    /// Diagnostic: prepare (never run) `sql` on the guarded connection; `Err` when the engine-level guard refuses it.
    pub fn guarded_probe(&self, sql: &str) -> Result<(), SourceError> {
        self.conn.prepare(sql).map(|_| ()).map_err(|e| SourceError::AccessDenied(e.to_string()))
    }

    /// table -> columns, from a fresh read-only connection (names only, no row data; denied tables get no columns).
    fn introspect(&self) -> Result<BTreeMap<String, Vec<String>>, SourceError> {
        let c = open_ro(&self.path)?;
        let names: Vec<String> = c
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
            .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
            .map_err(io)?;
        let mut out = BTreeMap::new();
        for n in names {
            let n = n.to_ascii_lowercase();
            let cols = if policy::allowed_columns(&n).is_some() {
                let t = policy::assert_table_allowed(&n)?;
                c.prepare(&format!("PRAGMA table_info({t})"))
                    .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(1))?.collect())
                    .map_err(io)?
            } else {
                vec![]
            };
            out.insert(n, cols);
        }
        Ok(out)
    }

    fn require_event_log(&self) -> Result<(), SourceError> {
        let r = self.schema_check()?;
        if r.ok() { Ok(()) } else { Err(SourceError::SchemaDrift(format!("{:?}", r.missing))) }
    }
}

fn text(v: ValueRef<'_>) -> Option<String> {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) | ValueRef::Blob(t) => Some(String::from_utf8_lossy(t).into_owned()),
    }
}

impl SourceAdapter for SqliteProduct {
    fn source_id(&self) -> &SourceId {
        &self.id
    }
    fn adapter(&self) -> &'static str {
        "product-sqlite"
    }
    fn data_class(&self) -> &'static str {
        "treated"
    }

    fn list_tables(&self) -> Result<Vec<String>, SourceError> {
        Ok(self.introspect()?.into_keys().filter(|t| policy::allowed_columns(t).is_some()).collect())
    }

    fn schema_check(&self) -> Result<SchemaReport, SourceError> {
        let schema = self.introspect()?;
        let mut r = SchemaReport::default();
        for (t, cols) in &schema {
            if DENIED_TABLES.contains(&t.as_str()) {
                r.denied_present.push(t.clone());
            } else if policy::allowed_columns(t).is_some() {
                r.tables.push(t.clone());
                if t == "event_log" {
                    for c in EVENT_READ_COLUMNS.iter().filter(|c| !cols.iter().any(|x| x == *c)) {
                        r.missing.push((t.clone(), (*c).to_owned()));
                    }
                }
            }
        }
        if !r.tables.iter().any(|t| t == "event_log") {
            r.missing.push(("event_log".into(), "*".into()));
        }
        Ok(r)
    }

    fn read_events(&self, after: &Watermark, limit: usize) -> Result<Batch, SourceError> {
        let limit = policy::check_limit(limit)?;
        let Watermark::Sequence(seq) = after else {
            return Err(SourceError::BadWatermark(format!("{} needs a sequence watermark", self.adapter())));
        };
        policy::assert_columns_allowed("event_log", EVENT_READ_COLUMNS)?;
        self.require_event_log()?;
        let sql = format!("SELECT {} FROM event_log WHERE sequence > ?1 ORDER BY sequence LIMIT ?2", EVENT_READ_COLUMNS.join(", "));
        let mut st = self.conn.prepare(&sql).map_err(io)?;
        let mut events = st
            .query_map(rusqlite::params![seq, (limit + 1) as i64], |r| {
                let g = |i: usize| text(r.get_ref(i).unwrap_or(ValueRef::Null));
                Ok(PlatformEvent {
                    sequence: r.get::<_, i64>(0).ok(),
                    event_id: g(1).unwrap_or_default(),
                    event_type: g(2).unwrap_or_default(),
                    entity: g(3),
                    entity_id: g(4),
                    case_id: g(5),
                    actor_role: g(6),
                    actor_id: g(7),
                    event_time: g(8).unwrap_or_default(),
                })
            })
            .map_err(io)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io)?;
        let more = events.len() > limit;
        events.truncate(limit);
        let next = events.last().and_then(|e| e.sequence).map_or_else(|| after.clone(), Watermark::Sequence);
        Ok(Batch { events, next, more })
    }

    fn read_dimension(&self, table: &str, limit: usize) -> Result<Vec<Row>, SourceError> {
        let limit = policy::check_limit(limit)?;
        let t = policy::assert_table_allowed(table)?;
        if t == "event_log" {
            return Err(SourceError::AccessDenied("event_log is read through read_events".into()));
        }
        let present = self.introspect()?.remove(&t).unwrap_or_default();
        let cols: Vec<&str> = policy::allowed_columns(&t).unwrap_or(&[]).iter().copied().filter(|c| present.iter().any(|p| p == c)).collect();
        if cols.is_empty() {
            return Ok(vec![]);
        }
        policy::assert_columns_allowed(&t, &cols)?;
        let mut st = self.conn.prepare(&format!("SELECT {} FROM {t} LIMIT {limit}", cols.join(", "))).map_err(io)?;
        st.query_map([], |r| Ok(cols.iter().enumerate().map(|(i, c)| ((*c).to_owned(), text(r.get_ref(i).unwrap_or(ValueRef::Null)))).collect::<Row>()))
            .map_err(io)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io)
    }
}
