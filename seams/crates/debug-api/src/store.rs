//! Run-event store: per-run append-only log with a monotonic sequence (1, 2, 3 ... per run), a purge floor and the
//! projection folded from every event. Memory, or a directory with one `<run>.jsonl` per run (event lines plus
//! `{"purge": {"floor": n}}` lines; a purge is logical, the file keeps appending). Postgres is not implemented here.
use crate::event::{NewEvent, RunEventSink};
use crate::project;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_EVENT_BYTES: usize = 256 * 1024;
/// Bounds of the model-call log of one run.
const MAX_CALL_BYTES: usize = 256 * 1024;
const MAX_CALLS_PER_RUN: usize = 2000;
const CALLS_SUFFIX: &str = ".calls.ndjson";

fn call_key(c: &Value) -> Option<(String, String, u64)> {
    Some((c["evidence_ref"].as_str()?.to_string(), c["role"].as_str()?.to_string(), c["n"].as_u64()?))
}

struct Log {
    floor: i64,
    events: Vec<Value>,
    state: Value,
    /// `pulso.model_call/1` records of the run (kept apart from the event stream: they carry prompt and response content).
    calls: Vec<Value>,
}

impl Log {
    fn head(&self) -> i64 {
        self.floor + self.events.len() as i64
    }
}

#[derive(Default)]
struct Inner {
    order: Vec<String>,
    logs: HashMap<String, Log>,
}

pub struct Store {
    inner: Mutex<Inner>,
    dir: Option<PathBuf>,
}

pub fn valid_run_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && !s.starts_with('.')
        && !s.ends_with('.')
        && !is_windows_device(s)
}

/// `con`, `nul`, `com1.x` ... open a device, not a file, on Windows.
fn is_windows_device(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or(s).to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && stem.ends_with(|c: char| c.is_ascii_digit() && c != '0'))
}

/// RFC 3339 UTC, seconds precision (civil-from-days, no time crate).
pub fn now_iso() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn event_id(run: &str, seq: i64) -> String {
    let h = Sha256::digest(format!("{run}:{seq}"));
    let x: String = h.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-4{}-8{}-{}", &x[0..8], &x[8..12], &x[13..16], &x[17..20], &x[20..32])
}

fn wire_event(run: &str, seq: i64, ev: NewEvent) -> Value {
    let (stage, status, reason) = match ev.kind.as_str() {
        "node_status_changed" => {
            let n = &ev.data["node"];
            (n["stage"].as_str().unwrap_or("run").to_string(), n["status"].as_str().unwrap_or("recorded").to_string(), n["reason_code"].clone())
        }
        "run_started" | "run_state_changed" => ("run".to_string(), ev.data["state"].as_str().unwrap_or("recorded").to_string(), Value::Null),
        _ => ("run".to_string(), "recorded".to_string(), Value::Null),
    };
    json!({
        "event_id": event_id(run, seq), "run_id": run, "run_ref": run, "sequence": seq,
        "entity_ref": {"kind": ev.entity_kind, "id": ev.entity_id}, "projection_revision": seq,
        "kind": ev.kind, "event_code": ev.kind, "stage": stage, "status": status, "reason_code": reason,
        "job_ref": null, "artifact_ref": null, "details_ref": null, "trace_id": null,
        "occurred_at": ev.occurred_at.unwrap_or_else(now_iso), "data": ev.data,
    })
}

impl Store {
    pub fn memory() -> Store {
        Store { inner: Mutex::new(Inner::default()), dir: None }
    }

    /// Opens (creating) a directory store and replays every `*.jsonl` file.
    pub fn open(dir: impl AsRef<Path>) -> Result<Store, String> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let mut loaded: Vec<(String, String, Log)> = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let (Some(run), Some("jsonl")) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|s| s.to_str())) else { continue };
            if !valid_run_id(run) {
                continue;
            }
            let text = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
            let mut log = Log { floor: 0, events: Vec::new(), state: project::empty_state(run), calls: Vec::new() };
            for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
                let v: Value = serde_json::from_str(line).map_err(|e| format!("{}:{}: {e}", path.display(), i + 1))?;
                if let Some(f) = v["purge"]["floor"].as_i64() {
                    log.floor = f;
                    log.events.retain(|e| e["sequence"].as_i64().is_some_and(|s| s > f));
                } else if v["sequence"].as_i64() == Some(log.head() + 1) {
                    project::apply(&mut log.state, &v);
                    log.events.push(v);
                } else {
                    return Err(format!("{}:{}: sequence is not contiguous", path.display(), i + 1));
                }
            }
            let first = log.events.first().and_then(|e| e["occurred_at"].as_str()).unwrap_or_default().to_string();
            loaded.push((first, run.to_string(), log));
        }
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let Some(run) = path.file_name().and_then(|s| s.to_str()).and_then(|n| n.strip_suffix(CALLS_SUFFIX)).map(str::to_string) else { continue };
            let Some((_, _, log)) = loaded.iter_mut().find(|(_, r, _)| *r == run) else { continue };
            for (i, line) in fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
                let v: Value = serde_json::from_str(line).map_err(|e| format!("{}:{}: {e}", path.display(), i + 1))?;
                if call_key(&v).is_some() && !log.calls.iter().any(|c| call_key(c) == call_key(&v)) {
                    log.calls.push(v);
                }
            }
        }
        loaded.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        let mut inner = Inner::default();
        for (_, run, log) in loaded {
            inner.order.push(run.clone());
            inner.logs.insert(run, log);
        }
        Ok(Store { inner: Mutex::new(inner), dir: Some(dir) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn persist(&self, run: &str, line: &Value) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let mut f = OpenOptions::new().create(true).append(true).open(dir.join(format!("{run}.jsonl"))).map_err(|e| format!("open log: {e}"))?;
        f.write_all(format!("{line}\n").as_bytes()).and_then(|()| f.sync_data()).map_err(|e| format!("write log: {e}"))
    }

    /// Records `pulso.model_call/1` calls of an existing run: validated (schema, `evidence_ref`, `role`, `n`), bounded (256 KiB per call, 2000 per
    /// run) and deduplicated by `(evidence_ref, role, n)`. Returns how many were new. Memory, plus `<run>.calls.ndjson` in a directory store.
    pub fn record_model_calls(&self, run: &str, calls: &[Value]) -> Result<usize, String> {
        let mut g = self.lock();
        let log = g.logs.get_mut(run).ok_or("unknown run")?;
        let mut fresh: Vec<&Value> = Vec::new();
        for c in calls {
            if c["schema"] != "pulso.model_call/1" || call_key(c).is_none() {
                return Err("not a pulso.model_call/1 record with evidence_ref, role and n".into());
            }
            if c.to_string().len() > MAX_CALL_BYTES {
                return Err("model call too large".into());
            }
            if !log.calls.iter().any(|x| call_key(x) == call_key(c)) && !fresh.iter().any(|x| call_key(x) == call_key(c)) {
                fresh.push(c);
            }
        }
        if log.calls.len() + fresh.len() > MAX_CALLS_PER_RUN {
            return Err("too many model calls for one run".into());
        }
        if let Some(dir) = &self.dir.as_ref().filter(|_| !fresh.is_empty()) {
            let mut f = OpenOptions::new().create(true).append(true).open(dir.join(format!("{run}{CALLS_SUFFIX}"))).map_err(|e| format!("open calls log: {e}"))?;
            let text: String = fresh.iter().map(|c| format!("{c}\n")).collect();
            f.write_all(text.as_bytes()).and_then(|()| f.sync_data()).map_err(|e| format!("write calls log: {e}"))?;
        }
        let n = fresh.len();
        log.calls.extend(fresh.into_iter().cloned());
        Ok(n)
    }

    /// The model calls recorded for a run, in recording order.
    pub fn model_calls(&self, run: &str) -> Vec<Value> {
        self.lock().logs.get(run).map(|l| l.calls.clone()).unwrap_or_default()
    }

    pub fn runs(&self) -> Vec<String> {
        self.lock().order.clone()
    }

    pub fn state(&self, run: &str) -> Option<Value> {
        self.lock().logs.get(run).map(|l| l.state.clone())
    }

    pub fn head(&self, run: &str) -> Option<i64> {
        self.lock().logs.get(run).map(Log::head)
    }

    pub fn floor(&self, run: &str) -> Option<i64> {
        self.lock().logs.get(run).map(|l| l.floor)
    }

    /// Retained events with `sequence > after`, in order, at most `limit`.
    pub fn events_after(&self, run: &str, after: i64, limit: usize) -> Vec<Value> {
        let g = self.lock();
        let Some(l) = g.logs.get(run) else { return Vec::new() };
        l.events.iter().filter(|e| e["sequence"].as_i64().is_some_and(|s| s > after)).take(limit).cloned().collect()
    }

    /// Drops retained history up to and including `seq` (cursors below the new floor get 410). The projection is kept.
    pub fn purge_through(&self, run: &str, seq: i64) -> Result<(), String> {
        let mut g = self.lock();
        let l = g.logs.get_mut(run).ok_or("unknown run")?;
        if seq < l.floor || seq > l.head() {
            return Err(format!("purge cursor {seq} outside {}..={}", l.floor, l.head()));
        }
        self.persist(run, &json!({"purge": {"floor": seq}}))?;
        l.events.retain(|e| e["sequence"].as_i64().is_some_and(|s| s > seq));
        l.floor = seq;
        Ok(())
    }
}

impl RunEventSink for Store {
    fn emit(&self, run_id: &str, ev: NewEvent) -> Result<Value, String> {
        if !valid_run_id(run_id) {
            return Err("invalid run id".into());
        }
        let kind_ok = !ev.kind.is_empty() && ev.kind.len() <= 64 && ev.kind.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !kind_ok || ev.entity_kind.is_empty() || ev.entity_id.is_empty() {
            return Err("invalid event".into());
        }
        if ev.data.to_string().len() > MAX_EVENT_BYTES {
            return Err("event too large".into());
        }
        let mut g = self.lock();
        if !g.logs.contains_key(run_id) {
            g.order.push(run_id.to_string());
            g.logs.insert(run_id.to_string(), Log { floor: 0, events: Vec::new(), state: project::empty_state(run_id), calls: Vec::new() });
        }
        let log = g.logs.get_mut(run_id).expect("inserted");
        let wire = wire_event(run_id, log.head() + 1, ev);
        self.persist(run_id, &wire)?;
        project::apply(&mut log.state, &wire);
        log.events.push(wire.clone());
        Ok(wire)
    }
}
