//! Per-source watermark store (port + in-memory and file implementations; Postgres in `pg_store`).
//! One row per `source_id`, bound to one adapter and one watermark kind: sources never mix.
use crate::{SourceError, SourceId, Watermark};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatermarkRecord {
    pub watermark: Watermark,
    pub adapter: String,
    /// Earliest `event_time` ever read from this source (history span for the cold-start gate).
    pub first_event_time: Option<String>,
    /// Cumulative `case.opened` events read (history volume for the cold-start gate).
    pub cases_opened: u64,
    pub batches: u64,
}

pub trait WatermarkStore {
    fn get(&self, id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError>;
    /// Compare-and-set: `expected` is the watermark the caller read (None = no row yet).
    fn commit(&self, id: &SourceId, expected: Option<&Watermark>, rec: &WatermarkRecord) -> Result<(), SourceError>;
}

fn order_key(w: &Watermark) -> (i64, String, String, String) {
    match w {
        Watermark::Sequence(n) => (*n, String::new(), String::new(), String::new()),
        Watermark::Dataset { ingested_at, batch_id, key } => (0, ingested_at.clone(), batch_id.clone(), key.clone()),
    }
}

/// The rules every store enforces (the Postgres store enforces the same ones in one transaction).
pub fn validate_commit(id: &SourceId, existing: Option<&WatermarkRecord>, expected: Option<&Watermark>, rec: &WatermarkRecord) -> Result<(), SourceError> {
    let want_kind = if id.mode() == crate::DataMode::Dataset { "dataset" } else { "sequence" };
    if rec.watermark.kind() != want_kind {
        return Err(SourceError::BadWatermark(format!("{} needs a {want_kind} watermark", id.as_str())));
    }
    let adapter_ok = match rec.adapter.as_str() {
        "dataset-pg" => id.mode() == crate::DataMode::Dataset,
        "product-sqlite" | "product-postgres" => id.mode() == crate::DataMode::Platform,
        _ => false,
    };
    if !adapter_ok {
        return Err(SourceError::Mismatch(format!("adapter {} cannot serve {}", rec.adapter, id.as_str())));
    }
    if existing.map(|e| &e.watermark) != expected {
        return Err(SourceError::Conflict(format!("watermark of {} moved since it was read", id.as_str())));
    }
    if let Some(e) = existing {
        if e.adapter != rec.adapter {
            return Err(SourceError::Conflict(format!("{} is bound to adapter {}", id.as_str(), e.adapter)));
        }
        if order_key(&rec.watermark) < order_key(&e.watermark) {
            return Err(SourceError::BadWatermark("watermark may not move backwards".into()));
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct MemStore {
    rows: Mutex<HashMap<String, WatermarkRecord>>,
}
impl WatermarkStore for MemStore {
    fn get(&self, id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError> {
        Ok(self.rows.lock().unwrap_or_else(|p| p.into_inner()).get(id.as_str()).cloned())
    }
    fn commit(&self, id: &SourceId, expected: Option<&Watermark>, rec: &WatermarkRecord) -> Result<(), SourceError> {
        let mut rows = self.rows.lock().unwrap_or_else(|p| p.into_inner());
        validate_commit(id, rows.get(id.as_str()), expected, rec)?;
        rows.insert(id.as_str().to_owned(), rec.clone());
        Ok(())
    }
}

/// One small text file per source, replaced atomically (write temp, rename). One monitor per source and work dir.
pub struct FileStore {
    dir: PathBuf,
}
impl FileStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<FileStore, SourceError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| SourceError::Io(e.to_string()))?;
        Ok(FileStore { dir })
    }
    fn path(&self, id: &SourceId) -> PathBuf {
        let h: String = Sha256::digest(id.as_str().as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{h}.wm"))
    }
}
impl WatermarkStore for FileStore {
    fn get(&self, id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError> {
        let text = match std::fs::read_to_string(self.path(id)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(SourceError::Io(e.to_string())),
        };
        let bad = |m: &str| SourceError::Io(format!("corrupt watermark file: {m}"));
        let mut kv = HashMap::new();
        for l in text.lines() {
            let (k, v) = l.split_once('=').ok_or_else(|| bad("line"))?;
            kv.insert(k.to_owned(), v.to_owned());
        }
        if kv.get("source_id").map(String::as_str) != Some(id.as_str()) {
            return Err(bad("source_id mismatch"));
        }
        let num = |k: &str| kv.get(k).and_then(|v| v.parse::<u64>().ok()).ok_or_else(|| bad(k));
        Ok(Some(WatermarkRecord {
            watermark: Watermark::decode(kv.get("watermark").ok_or_else(|| bad("watermark"))?)?,
            adapter: kv.get("adapter").cloned().ok_or_else(|| bad("adapter"))?,
            first_event_time: kv.get("first_event_time").filter(|v| !v.is_empty()).cloned(),
            cases_opened: num("cases_opened")?,
            batches: num("batches")?,
        }))
    }
    fn commit(&self, id: &SourceId, expected: Option<&Watermark>, rec: &WatermarkRecord) -> Result<(), SourceError> {
        validate_commit(id, self.get(id)?.as_ref(), expected, rec)?;
        let line = |s: &str| s.replace(['\n', '\r'], " ");
        let body = format!(
            "source_id={}\nadapter={}\nwatermark={}\nfirst_event_time={}\ncases_opened={}\nbatches={}\n",
            id.as_str(),
            line(&rec.adapter),
            line(&rec.watermark.encode()),
            rec.first_event_time.as_deref().map(line).unwrap_or_default(),
            rec.cases_opened,
            rec.batches
        );
        let path = self.path(id);
        let tmp = path.with_extension("tmp");
        let io = |e: std::io::Error| SourceError::Io(e.to_string());
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp).map_err(io)?;
            f.write_all(body.as_bytes()).map_err(io)?;
            f.sync_all().map_err(io)?;
        }
        std::fs::rename(&tmp, &path).map_err(io)
    }
}
