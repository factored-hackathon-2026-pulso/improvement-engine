//! `dataset-pg`: E0 history loaded by the user into `raw` (`e0_*`) or the canonical `augmented` layer, mapped to platform
//! events (design section 6) so the sensor sees the same stream as in platform mode. Watermark: `_ingested_at` +
//! `_batch_id` (+ the row key so a batch larger than the cap resumes exactly). Only ids, timestamps and `author_role` are
//! selected: no E0 text column is ever read. Mapped: case.opened, turn.created, case.closed. Not mapped yet (gaps):
//! case.assigned, case.first_responded, case.queued, escalations, calls.
use crate::pg_product::{connect_read_only, ident, pg};
use crate::{Batch, PlatformEvent, Row, SchemaReport, SourceAdapter, SourceError, SourceId, Watermark, policy};
use postgres::Client;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

/// Evaluator tables and the pseudonym map never exist in a readable schema; reported if they do.
const FORBIDDEN: &[&str] = &["labels", "timeline", "pseudonym_map"];

pub struct DatasetPg {
    id: SourceId,
    schema: String,
    /// (case table, turn table, close table)
    tables: (&'static str, &'static str, &'static str),
    client: Mutex<Client>,
}

impl DatasetPg {
    /// `schema` is `raw` (e0_* tables) or `augmented` (canonical layer).
    pub fn connect(dsn: &str, schema: &str, id: SourceId) -> Result<DatasetPg, SourceError> {
        let tables = match ident(schema)? {
            "raw" => ("e0_case", "e0_turn", "e0_case_close"),
            "augmented" => ("cases", "turns", "case_closes"),
            other => return Err(SourceError::BadConfig(format!("dataset schema must be raw or augmented, not {other:?}"))),
        };
        Ok(DatasetPg { id, schema: schema.to_owned(), tables, client: Mutex::new(connect_read_only(dsn)?) })
    }

    fn required(&self) -> [(&'static str, &'static [&'static str]); 3] {
        [
            (self.tables.0, &["case_id", "opened_at", "_ingested_at", "_batch_id"]),
            (self.tables.1, &["turn_id", "case_id", "event_time", "author_role", "_ingested_at", "_batch_id"]),
            (self.tables.2, &["case_id", "closed_at", "_ingested_at", "_batch_id"]),
        ]
    }

    fn columns(&self) -> Result<Vec<(String, String)>, SourceError> {
        let mut c = self.client.lock().unwrap_or_else(|p| p.into_inner());
        let rows = c.query("SELECT table_name::text, column_name::text FROM information_schema.columns WHERE table_schema = $1", &[&self.schema]).map_err(pg)?;
        Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
    }
}

fn event_id(key: &str) -> String {
    let h: String = Sha256::digest(key.as_bytes()).iter().take(10).map(|b| format!("{b:02x}")).collect();
    format!("ds-{h}")
}

impl SourceAdapter for DatasetPg {
    fn source_id(&self) -> &SourceId {
        &self.id
    }
    fn adapter(&self) -> &'static str {
        "dataset-pg"
    }
    fn data_class(&self) -> &'static str {
        "e0"
    }

    fn list_tables(&self) -> Result<Vec<String>, SourceError> {
        let cols = self.columns()?;
        Ok([self.tables.0, self.tables.1, self.tables.2].iter().filter(|t| cols.iter().any(|(ct, _)| ct == *t)).map(|t| (*t).to_owned()).collect())
    }

    fn schema_check(&self) -> Result<SchemaReport, SourceError> {
        let cols = self.columns()?;
        let mut r = SchemaReport { tables: self.list_tables()?, ..Default::default() };
        for (t, need) in self.required() {
            for c in need.iter().filter(|c| !cols.iter().any(|(ct, cc)| ct == t && cc == *c)) {
                r.missing.push((t.to_owned(), (*c).to_owned()));
            }
        }
        for f in FORBIDDEN {
            if cols.iter().any(|(t, _)| t == f) {
                r.denied_present.push((*f).to_owned());
            }
        }
        Ok(r)
    }

    fn read_events(&self, after: &Watermark, limit: usize) -> Result<Batch, SourceError> {
        let limit = policy::check_limit(limit)?;
        let Watermark::Dataset { ingested_at, batch_id, key } = after else {
            return Err(SourceError::BadWatermark(format!("{} needs a dataset watermark", self.adapter())));
        };
        let r = self.schema_check()?;
        if !r.ok() {
            return Err(SourceError::SchemaDrift(format!("{:?}", r.missing)));
        }
        let (s, (tc, tt, tx)) = (&self.schema, self.tables);
        let ts = "to_char(_ingested_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US')::text AS ts, _batch_id::text AS batch";
        let sql = format!(
            "SELECT ts, batch, key, event_type, entity_id, case_id, actor_role, event_time FROM (
               SELECT {a}, 'case:' || case_id::text AS key, 'case.opened'::text AS event_type, case_id::text AS entity_id, case_id::text AS case_id, NULL::text AS actor_role, opened_at::text AS event_time FROM {s}.{tc}
               UNION ALL
               SELECT {b}, 'turn:' || turn_id::text, 'turn.created', turn_id::text, case_id::text, author_role::text, event_time::text FROM {s}.{tt}
               UNION ALL
               SELECT {c}, 'close:' || case_id::text, 'case.closed', case_id::text, case_id::text, NULL::text, closed_at::text FROM {s}.{tx}
             ) u
             WHERE (ts COLLATE \"C\", batch COLLATE \"C\", key COLLATE \"C\") > ($1::text COLLATE \"C\", $2::text COLLATE \"C\", $3::text COLLATE \"C\")
             ORDER BY ts COLLATE \"C\", batch COLLATE \"C\", key COLLATE \"C\" LIMIT $4",
            a = ts,
            b = ts,
            c = ts,
        );
        let mut c = self.client.lock().unwrap_or_else(|p| p.into_inner());
        let rows = c.query(&sql, &[ingested_at, batch_id, key, &((limit + 1) as i64)]).map_err(pg)?;
        let mut cursor = Vec::new();
        let mut events: Vec<PlatformEvent> = rows
            .iter()
            .map(|r| {
                let (ts, batch, key): (String, String, String) = (r.get(0), r.get(1), r.get(2));
                cursor.push(Watermark::Dataset { ingested_at: ts, batch_id: batch, key: key.clone() });
                PlatformEvent {
                    sequence: None,
                    event_id: event_id(&key),
                    event_type: r.get(3),
                    entity: Some(if key.starts_with("turn:") { "turn" } else { "case" }.to_owned()),
                    entity_id: r.get(4),
                    case_id: r.get(5),
                    actor_role: r.get(6),
                    actor_id: None,
                    event_time: r.get::<_, Option<String>>(7).unwrap_or_default(),
                }
            })
            .collect();
        let more = events.len() > limit;
        events.truncate(limit);
        cursor.truncate(limit);
        let next = cursor.pop().unwrap_or_else(|| after.clone());
        Ok(Batch { events, next, more })
    }

    fn read_dimension(&self, table: &str, _limit: usize) -> Result<Vec<Row>, SourceError> {
        Err(SourceError::AccessDenied(format!("dataset mode exposes no dimension tables ({table:?})")))
    }
}
