//! `product-postgres`: the platform tables replicated into the shared Postgres (`product` schema). The connection is made
//! as the read-only role (`pulso_product_ro`: column-level SELECT) with `default_transaction_read_only=on`; the adapter
//! refuses to run if the server does not report that setting on, and builds every query from the allow-list.
use crate::policy::{self, DENIED_TABLES, EVENT_READ_COLUMNS};
use crate::{Batch, PlatformEvent, Row, SchemaReport, SourceAdapter, SourceError, SourceId, Watermark};
use postgres::{Client, Config, NoTls};
use std::sync::Mutex;

pub(crate) fn pg<E: std::fmt::Display>(e: E) -> SourceError {
    SourceError::Io(format!("postgres: {e}"))
}

pub(crate) fn ident(s: &str) -> Result<&str, SourceError> {
    let ok = !s.is_empty() && s.len() <= 63 && s.bytes().enumerate().all(|(i, b)| b.is_ascii_lowercase() || b == b'_' || (i > 0 && b.is_ascii_digit()));
    if ok { Ok(s) } else { Err(SourceError::AccessDenied(format!("bad identifier {s:?}"))) }
}

/// Connect read-only: session options force read-only transactions and UTC; the server's answer is verified.
pub(crate) fn connect_read_only(dsn: &str) -> Result<Client, SourceError> {
    let mut cfg: Config = dsn.parse().map_err(|_| SourceError::BadConfig("invalid postgres DSN".into()))?;
    cfg.options("-c default_transaction_read_only=on -c TimeZone=UTC");
    let mut c = cfg.connect(NoTls).map_err(pg)?;
    let on: String = c.query_one("SHOW default_transaction_read_only", &[]).map_err(pg)?.get(0);
    if on != "on" {
        return Err(SourceError::AccessDenied("default_transaction_read_only is not on; refusing a writable connection".into()));
    }
    Ok(c)
}

/// Sequences per payload query (a keyed read, never a min..max range over events the feed does not need).
pub(crate) const PAYLOAD_CHUNK: usize = 5000;

/// Query of the platform-cells feed for the payload of given sequences: only `sequence` and `payload`, keyed by `ANY($1)`.
pub(crate) fn payload_sql(schema: &str) -> String {
    format!("SELECT sequence, payload::text FROM {schema}.event_log WHERE sequence = ANY($1)")
}

/// Sorted, de-duplicated sequences in chunks of `PAYLOAD_CHUNK`.
pub(crate) fn payload_chunks(sequences: &[i64]) -> Vec<Vec<i64>> {
    let set: std::collections::BTreeSet<i64> = sequences.iter().copied().collect();
    let all: Vec<i64> = set.into_iter().collect();
    all.chunks(PAYLOAD_CHUNK).map(<[i64]>::to_vec).collect()
}

/// Keyed read of a dimension table by id: allow-listed columns only, ids as one `ANY($1)` parameter (never interpolated).
pub(crate) fn dimension_by_ids_sql(schema: &str, table: &str, cols: &[&str]) -> String {
    format!("SELECT {} FROM {schema}.{table} WHERE id::text = ANY($1)", cols.iter().map(|c| format!("{c}::text")).collect::<Vec<_>>().join(", "))
}

pub struct PostgresProduct {
    id: SourceId,
    schema: String,
    client: Mutex<Client>,
}

impl PostgresProduct {
    pub fn connect(dsn: &str, schema: &str, id: SourceId) -> Result<PostgresProduct, SourceError> {
        let schema = ident(schema)?.to_owned();
        Ok(PostgresProduct { id, schema, client: Mutex::new(connect_read_only(dsn)?) })
    }

    /// True when the connection could create a table (it must not).
    pub fn can_write(&self) -> bool {
        let mut c = self.client.lock().unwrap_or_else(|p| p.into_inner());
        c.batch_execute("CREATE TEMP TABLE _r1m_probe(x int)").is_ok()
    }

    /// (table, column) pairs visible to this role in the product schema (privilege-filtered by information_schema).
    fn columns(&self) -> Result<Vec<(String, String)>, SourceError> {
        let mut c = self.client.lock().unwrap_or_else(|p| p.into_inner());
        let rows = c.query("SELECT table_name::text, column_name::text FROM information_schema.columns WHERE table_schema = $1", &[&self.schema]).map_err(pg)?;
        Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
    }
}

impl SourceAdapter for PostgresProduct {
    fn source_id(&self) -> &SourceId {
        &self.id
    }
    fn adapter(&self) -> &'static str {
        "product-postgres"
    }
    fn data_class(&self) -> &'static str {
        "treated"
    }

    fn list_tables(&self) -> Result<Vec<String>, SourceError> {
        let mut t: Vec<String> = self.columns()?.into_iter().map(|(t, _)| t).filter(|t| policy::allowed_columns(t).is_some()).collect();
        t.sort();
        t.dedup();
        Ok(t)
    }

    fn schema_check(&self) -> Result<SchemaReport, SourceError> {
        let cols = self.columns()?;
        let mut r = SchemaReport::default();
        for t in self.list_tables()? {
            r.tables.push(t);
        }
        for d in DENIED_TABLES {
            if cols.iter().any(|(t, _)| t == d) {
                r.denied_present.push((*d).to_owned());
            }
        }
        for c in EVENT_READ_COLUMNS {
            if !cols.iter().any(|(t, col)| t == "event_log" && col == c) {
                r.missing.push(("event_log".into(), (*c).to_owned()));
            }
        }
        Ok(r)
    }

    fn read_events(&self, after: &Watermark, limit: usize) -> Result<Batch, SourceError> {
        let limit = policy::check_limit(limit)?;
        let Watermark::Sequence(seq) = after else {
            return Err(SourceError::BadWatermark(format!("{} needs a sequence watermark", self.adapter())));
        };
        policy::assert_columns_allowed("event_log", EVENT_READ_COLUMNS)?;
        let r = self.schema_check()?;
        if !r.ok() {
            return Err(SourceError::SchemaDrift(format!("{:?}", r.missing)));
        }
        let select: Vec<String> = EVENT_READ_COLUMNS.iter().map(|c| if *c == "sequence" { "sequence".to_owned() } else { format!("{c}::text") }).collect();
        let sql = format!("SELECT {} FROM {}.event_log WHERE sequence > $1 ORDER BY sequence LIMIT $2", select.join(", "), self.schema);
        let mut c = self.client.lock().unwrap_or_else(|p| p.into_inner());
        let rows = c.query(&sql, &[seq, &((limit + 1) as i64)]).map_err(pg)?;
        let mut events: Vec<PlatformEvent> = rows
            .iter()
            .map(|r| PlatformEvent {
                sequence: Some(r.get::<_, i64>(0)),
                event_id: r.get::<_, Option<String>>(1).unwrap_or_default(),
                event_type: r.get::<_, Option<String>>(2).unwrap_or_default(),
                entity: r.get(3),
                entity_id: r.get(4),
                case_id: r.get(5),
                actor_role: r.get(6),
                actor_id: r.get(7),
                event_time: r.get::<_, Option<String>>(8).unwrap_or_default(),
            })
            .collect();
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
        let visible = self.columns()?;
        let cols: Vec<&str> = policy::allowed_columns(&t).unwrap_or(&[]).iter().copied().filter(|c| visible.iter().any(|(vt, vc)| *vt == t && vc == c)).collect();
        if cols.is_empty() {
            return Ok(vec![]);
        }
        policy::assert_columns_allowed(&t, &cols)?;
        let sql = format!("SELECT {} FROM {}.{t} LIMIT {limit}", cols.iter().map(|c| format!("{c}::text")).collect::<Vec<_>>().join(", "), self.schema);
        let mut c = self.client.lock().unwrap_or_else(|p| p.into_inner());
        let rows = c.query(&sql, &[]).map_err(pg)?;
        Ok(rows.iter().map(|r| cols.iter().enumerate().map(|(i, c)| ((*c).to_owned(), r.get::<_, Option<String>>(i))).collect()).collect())
    }

    fn read_event_payloads(&self, sequences: &[i64]) -> Result<std::collections::BTreeMap<i64, String>, SourceError> {
        if sequences.is_empty() {
            return Ok(Default::default());
        }
        policy::assert_columns_allowed("event_log", &["sequence", "payload"])?;
        let sql = payload_sql(&self.schema);
        let mut c = self.client.lock().unwrap_or_else(|p| p.into_inner());
        let mut out = std::collections::BTreeMap::new();
        for chunk in payload_chunks(sequences) {
            // a role without a grant on `payload` (the default read-only role) yields no payloads: the feed degrades, it never fails
            let Ok(rows) = c.query(&sql, &[&chunk]) else { return Ok(Default::default()) };
            out.extend(rows.iter().filter_map(|r| Some((r.get::<_, i64>(0), r.get::<_, Option<String>>(1)?))));
        }
        Ok(out)
    }

    fn read_dimension_by_ids(&self, table: &str, ids: &[String]) -> Result<Vec<Row>, SourceError> {
        let t = policy::assert_table_allowed(table)?;
        if t == "event_log" {
            return Err(SourceError::AccessDenied("event_log is read through read_events".into()));
        }
        let visible = self.columns()?;
        let cols: Vec<&str> = policy::allowed_columns(&t).unwrap_or(&[]).iter().copied().filter(|c| visible.iter().any(|(vt, vc)| *vt == t && vc == c)).collect();
        if !cols.contains(&"id") {
            return Ok(vec![]);
        }
        policy::assert_columns_allowed(&t, &cols)?;
        let sql = dimension_by_ids_sql(&self.schema, &t, &cols);
        let mut c = self.client.lock().unwrap_or_else(|p| p.into_inner());
        let mut out = vec![];
        for chunk in ids.chunks(1000) {
            let list: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let rows = c.query(&sql, &[&list]).map_err(pg)?;
            out.extend(rows.iter().map(|r| cols.iter().enumerate().map(|(i, c)| ((*c).to_owned(), r.get::<_, Option<String>>(i))).collect::<Row>()));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_query_reads_only_sequence_and_payload_by_a_keyed_any() {
        let sql = payload_sql("product");
        assert_eq!(sql, "SELECT sequence, payload::text FROM product.event_log WHERE sequence = ANY($1)");
        for denied in ["actor_id", "tenant_id", "customer_id", "event_time", "*", "min(", "BETWEEN", ">="] {
            assert!(!sql.contains(denied), "{denied} in {sql}");
        }
    }

    #[test]
    fn payload_sequences_are_sorted_deduplicated_and_chunked_so_sparse_ranges_cost_nothing() {
        assert!(payload_chunks(&[]).is_empty());
        assert_eq!(payload_chunks(&[9, 3, 3, 1_000_000_000]), vec![vec![3, 9, 1_000_000_000]]);
        let many: Vec<i64> = (0..(PAYLOAD_CHUNK as i64 * 2 + 1)).rev().collect();
        let chunks = payload_chunks(&many);
        assert_eq!(chunks.iter().map(Vec::len).collect::<Vec<_>>(), vec![PAYLOAD_CHUNK, PAYLOAD_CHUNK, 1]);
        assert_eq!(chunks[0][0], 0);
        assert_eq!(chunks.concat().len(), many.len());
    }

    #[test]
    fn the_keyed_dimension_query_passes_ids_as_a_parameter_and_selects_the_given_columns_only() {
        let sql = dimension_by_ids_sql("product", "cases", &["id", "case_type", "opened_at"]);
        assert_eq!(sql, "SELECT id::text, case_type::text, opened_at::text FROM product.cases WHERE id::text = ANY($1)");
        assert!(!sql.contains("LIMIT"), "a keyed read has no cap that could drop a case: {sql}");
    }

    #[test]
    fn schema_and_identifiers_stay_closed() {
        assert!(ident("product").is_ok());
        for bad in ["", "Product", "a-b", "1a", "x; drop table y", "a b"] {
            assert!(ident(bad).is_err(), "{bad}");
        }
    }
}
