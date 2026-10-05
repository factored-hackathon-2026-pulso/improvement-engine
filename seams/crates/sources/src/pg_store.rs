//! `pulso.source_watermark` (migration 0053) behind the `WatermarkStore` port. Same rules as the other stores, enforced inside
//! one transaction with the row locked, so two monitors can never both advance a source from the same watermark.
use crate::pg_product::pg;
use crate::store::{WatermarkRecord, WatermarkStore, validate_commit};
use crate::{SourceError, SourceId, Watermark};
use postgres::{Client, Config, NoTls};
use std::sync::Mutex;

pub struct PgStore {
    client: Mutex<Client>,
}

impl PgStore {
    /// Connect as the engine role (`pulso_app`), which may write `pulso.*`; never as a reader role.
    pub fn connect(dsn: &str) -> Result<PgStore, SourceError> {
        let cfg: Config = dsn.parse().map_err(|_| SourceError::BadConfig("invalid postgres DSN".into()))?;
        Ok(PgStore { client: Mutex::new(cfg.connect(NoTls).map_err(pg)?) })
    }
}

fn load(c: &mut impl postgres::GenericClient, id: &SourceId, lock: bool) -> Result<Option<WatermarkRecord>, SourceError> {
    let sql = format!("SELECT watermark, adapter, first_event_time, cases_opened, batches FROM pulso.source_watermark WHERE source_id = $1{}", if lock { " FOR UPDATE" } else { "" });
    let Some(r) = c.query_opt(&sql, &[&id.as_str()]).map_err(pg)? else { return Ok(None) };
    Ok(Some(WatermarkRecord {
        watermark: Watermark::decode(&r.get::<_, String>(0))?,
        adapter: r.get(1),
        first_event_time: r.get(2),
        cases_opened: r.get::<_, i64>(3) as u64,
        batches: r.get::<_, i64>(4) as u64,
    }))
}

impl WatermarkStore for PgStore {
    fn get(&self, id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError> {
        load(&mut *self.client.lock().unwrap_or_else(|p| p.into_inner()), id, false)
    }

    fn commit(&self, id: &SourceId, expected: Option<&Watermark>, rec: &WatermarkRecord) -> Result<(), SourceError> {
        let mut c = self.client.lock().unwrap_or_else(|p| p.into_inner());
        let mut tx = c.transaction().map_err(pg)?;
        let existing = load(&mut tx, id, true)?;
        validate_commit(id, existing.as_ref(), expected, rec)?;
        let (w, cases, batches) = (rec.watermark.encode(), rec.cases_opened as i64, rec.batches as i64);
        if existing.is_some() {
            tx.execute("UPDATE pulso.source_watermark SET watermark = $2, first_event_time = $3, cases_opened = $4, batches = $5, updated_at = now() WHERE source_id = $1", &[&id.as_str(), &w, &rec.first_event_time, &cases, &batches]).map_err(pg)?;
        } else {
            let n = tx.execute("INSERT INTO pulso.source_watermark (source_id, data_mode, adapter, watermark, first_event_time, cases_opened, batches) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING", &[&id.as_str(), &id.mode().as_str(), &rec.adapter, &w, &rec.first_event_time, &cases, &batches]).map_err(pg)?;
            if n == 0 {
                return Err(SourceError::Conflict(format!("{} was created concurrently", id.as_str())));
            }
        }
        tx.commit().map_err(pg)
    }
}
