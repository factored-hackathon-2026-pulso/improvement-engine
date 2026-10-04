//! `monitor::tick`: one read-batch -> package -> sensor job -> run record -> watermark commit cycle.
//!
//! Order matters for crash safety: the watermark is committed LAST. A kill before that commit re-reads the same batch
//! (at-least-once); the run id, package id and job key are pure functions of (source, from, to), so the replay finds the
//! committed sensor output and the identical run record and changes nothing (idempotent downstream).
//!
//! Honesty: the sensor step is the claude-standin runner (its numbers do not come from the package); the signals in the run
//! record are computed here from the event batch. Release and observation are `simulated`; there is no model.
use crate::config::{Config, SensorKind};
use crate::store::{WatermarkRecord, WatermarkStore};
use crate::{Batch, DataMode, PlatformEvent, SourceAdapter, SourceError, Watermark};
use engine::adapters::{StepEnv, thread_handlers};
use engine::{FileStore as JobFileStore, Options, run_once};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Admitted event types of platform-contract 1.1.0 (`event-catalog.json`, drift-tested).
pub const ADMITTED_EVENT_TYPES: &[&str] = &[
    "case.opened",
    "case.queued",
    "case.assigned",
    "case.status_changed",
    "case.read",
    "case.first_responded",
    "case.closed",
    "case.viewed",
    "turn.created",
    "staff.availability_changed",
    "auth.login_failed",
    "auth.account_locked",
    "auth.session_started",
    "auth.session_ended",
];
/// Known security/credential telemetry: counted, never packaged.
pub const DENIED_EVENT_TYPES: &[&str] = &["auth.password_accepted", "auth.mfa_challenge_issued", "auth.mfa_failed", "customer.session_started"];

#[derive(Debug)]
pub enum TickOutcome {
    Idle { watermark: Watermark },
    Processed { run_id: String, events: usize, admitted: usize, quarantined: usize, from: Watermark, to: Watermark, more: bool, record: Value },
}

fn io<E: std::fmt::Display>(e: E) -> SourceError {
    SourceError::Io(e.to_string())
}

pub fn tick(config: &Config, adapter: &dyn SourceAdapter, store: &dyn WatermarkStore) -> Result<TickOutcome, SourceError> {
    let id = adapter.source_id();
    if id != &config.source_id || adapter.adapter() != config.adapter.as_str() || id.mode() != config.data_mode {
        return Err(SourceError::Mismatch(format!("config names {} via {}, adapter is {} via {}", config.source_id.as_str(), config.adapter.as_str(), id.as_str(), adapter.adapter())));
    }
    let existing = store.get(id)?;
    if let Some(e) = &existing {
        if e.adapter != adapter.adapter() {
            return Err(SourceError::Mismatch(format!("{} is bound to adapter {}", id.as_str(), e.adapter)));
        }
    }
    let from = existing.as_ref().map_or_else(|| Watermark::origin_for(adapter.adapter()), |e| e.watermark.clone());
    let report = adapter.schema_check()?;
    if !report.ok() {
        return Err(SourceError::SchemaDrift(format!("{:?}", report.missing)));
    }
    let Batch { events, next: to, more } = adapter.read_events(&from, config.batch_cap)?;
    if events.is_empty() {
        return Ok(TickOutcome::Idle { watermark: from });
    }

    let (admitted, denied, unknown) = normalise(&events);
    let first_event_time = existing.as_ref().and_then(|e| e.first_event_time.clone()).into_iter().chain(events.iter().map(|e| e.event_time.clone()).filter(|t| !t.is_empty())).min();
    let last_event_time = events.iter().map(|e| e.event_time.as_str()).filter(|t| !t.is_empty()).max().unwrap_or("").to_owned();
    let batch_cases = admitted.iter().filter(|e| e.event_type == "case.opened").count() as u64;
    let cases_total = existing.as_ref().map_or(0, |e| e.cases_opened) + batch_cases;
    let days = match (first_event_time.as_deref().and_then(day_number), day_number(&last_event_time)) {
        (Some(a), Some(b)) if b >= a => (b - a) as u64,
        _ => 0,
    };

    let hex: String = Sha256::digest(format!("{}|{}|{}", id.as_str(), from.encode(), to.encode()).as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
    let (run_id, snap) = (format!("mon-{hex}"), format!("pkg-{hex}"));
    let work = &config.work_dir;
    write_package(work, &snap, config, adapter, &from, &to, &admitted)?;
    let sensor = match config.sensor {
        SensorKind::StandIn => run_sensor(config, &run_id, &snap, adapter.data_class(), &admitted)?,
        SensorKind::RustEvents => run_rust_events(config, &run_id, &snap, adapter.data_class(), &admitted)?,
    };

    let signals = signals(&admitted, days, cases_total, config);
    let label = match config.data_mode {
        DataMode::Dataset => "demo/replay data, not production; release and observation simulated",
        DataMode::Platform => "real platform signals; release and observation simulated until EXT-2",
    };
    let record = json!({
        "contract": "pulso-monitor-run/0",
        "run_id": run_id,
        "package": snap,
        "source_id": id.as_str(),
        "data_mode": config.data_mode.as_str(),
        "data_origin": config.data_mode.data_origin(),
        "adapter": adapter.adapter(),
        "data_class": adapter.data_class(),
        "label": label,
        "watermark_from": from.encode(),
        "watermark_to": to.encode(),
        "more": more,
        "events_read": events.len(),
        "events_admitted": admitted.len(),
        "quarantined": {"denied": denied, "unknown": unknown},
        "observed_until": last_event_time,
        "history": {"days": days, "cases": cases_total},
        "evidence": {"sensor": sensor["semantics"], "signals": "computed-local", "release": "simulated", "observation": "simulated", "model": "none"},
        "sensor": sensor,
        "signals": signals,
    });
    let runs = work.join("runs");
    std::fs::create_dir_all(&runs).map_err(io)?;
    atomic_write(&runs.join(format!("{run_id}.json")), &record.to_string())?;

    let new = WatermarkRecord {
        watermark: to.clone(),
        adapter: adapter.adapter().to_owned(),
        first_event_time,
        cases_opened: cases_total,
        batches: existing.as_ref().map_or(0, |e| e.batches) + 1,
    };
    store.commit(id, existing.as_ref().map(|e| &e.watermark), &new)?;
    Ok(TickOutcome::Processed { run_id, events: events.len(), admitted: admitted.len(), quarantined: denied + unknown, from, to, more, record })
}

fn normalise(events: &[PlatformEvent]) -> (Vec<&PlatformEvent>, usize, usize) {
    let (mut ok, mut denied, mut unknown) = (vec![], 0, 0);
    for e in events {
        if ADMITTED_EVENT_TYPES.contains(&e.event_type.as_str()) {
            ok.push(e);
        } else if DENIED_EVENT_TYPES.contains(&e.event_type.as_str()) {
            denied += 1;
        } else {
            unknown += 1;
        }
    }
    (ok, denied, unknown)
}

fn atomic_write(path: &Path, body: &str) -> Result<(), SourceError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

/// `<work>/packages/<snap>/{events.ndjson,manifest.json}`; identifiers and enums only, no payload, no text.
fn write_package(work: &Path, snap: &str, config: &Config, adapter: &dyn SourceAdapter, from: &Watermark, to: &Watermark, events: &[&PlatformEvent]) -> Result<(), SourceError> {
    let root = work.join("packages");
    let dir = root.join(snap);
    if dir.join("manifest.json").is_file() {
        return Ok(()); // replay of a batch whose package is already complete
    }
    let tmp = root.join(format!("{snap}.tmp"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(io)?;
    let mut lines = String::new();
    for (i, e) in events.iter().enumerate() {
        lines.push_str(&json!({"ordinal": i, "sequence": e.sequence, "event_id": e.event_id, "event_type": e.event_type, "entity": e.entity, "entity_id": e.entity_id, "case_id": e.case_id, "actor_role": e.actor_role, "actor_id": e.actor_id, "event_time": e.event_time}).to_string());
        lines.push('\n');
    }
    std::fs::write(tmp.join("events.ndjson"), lines).map_err(io)?;
    let dims = config.sensor == SensorKind::RustEvents;
    if dims {
        std::fs::write(tmp.join("cases.ndjson"), case_dimension(adapter, events)?).map_err(io)?;
    }
    let mut manifest = json!({"contract": "platform-events-package/0", "contract_version_source": "platform_live 1.1.0", "source_id": config.source_id.as_str(), "data_mode": config.data_mode.as_str(), "adapter": adapter.adapter(), "data_class": adapter.data_class(), "watermark_from": from.encode(), "watermark_to": to.encode(), "events": events.len()});
    if dims {
        manifest["dimensions"] = json!(["cases.ndjson"]);
    }
    std::fs::write(tmp.join("manifest.json"), manifest.to_string()).map_err(io)?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&tmp, &dir).map_err(io)
}

/// Allow-listed `cases` columns (`channel`, `language`, `priority`, `previous_case_id`) for the cases of this batch; opaque case
/// ids only, never `customer_id`, never text. The sensor needs the cell of each case; events alone do not carry it.
fn case_dimension(adapter: &dyn SourceAdapter, events: &[&PlatformEvent]) -> Result<String, SourceError> {
    let wanted: std::collections::BTreeSet<&str> = events.iter().filter_map(|e| e.case_id.as_deref()).collect();
    let mut rows = adapter.read_dimension("cases", crate::policy::HARD_CAP)?;
    rows.retain(|r| r.get("id").and_then(Option::as_deref).is_some_and(|id| wanted.contains(id)));
    rows.sort_by(|a, b| a.get("id").cmp(&b.get("id")));
    let col = |r: &crate::Row, k: &str| r.get(k).cloned().flatten();
    let mut out = String::new();
    for r in &rows {
        out.push_str(&json!({"case_id": col(r, "id"), "channel": col(r, "channel"), "language": col(r, "language"), "priority": col(r, "priority"), "previous_case_id": col(r, "previous_case_id")}).to_string());
        out.push('\n');
    }
    Ok(out)
}

/// R1G: the real Rust sensor over the cumulative event packages of this source (same `engine-steps/0` sensors input and
/// output as the stand-in; the superset report `sensor-events/1` rides along under `report`).
fn run_rust_events(config: &Config, run_id: &str, snap: &str, data_class: &str, events: &[&PlatformEvent]) -> Result<Value, SourceError> {
    let date = |f: Option<&&PlatformEvent>| f.and_then(|e| e.event_time.get(..10)).unwrap_or("1970-01-01").to_owned();
    let doc = json!({
        "contract_version": "engine-steps/0", "step": "sensors", "run_id": run_id, "data_class": data_class,
        "source_snapshot_ref": format!("snapshot:{snap}@1"), "discovery_config_ref": "discovery_config:monitor@1",
        "window": {"start": date(events.first()), "end": date(events.last())}, "metric_spec_refs": ["metric_spec:platform-events@1"],
    });
    let p = steps::events_sensor::Params {
        min_support: config.min_support,
        k_anon: config.k_anon,
        min_cell_cases: config.min_cell_cases,
        min_history_days: config.min_history_days,
        min_history_cases: config.min_history_cases,
        ..steps::events_sensor::Params::default()
    };
    let (out, report) = steps::events_sensor::run_package(&doc.to_string(), &config.work_dir.join("packages"), &p).map_err(|e| SourceError::Sensor(e.to_string()))?;
    let (output, report): (Value, Value) = (serde_json::from_str(&out).map_err(io)?, serde_json::from_str(&report).map_err(io)?);
    Ok(json!({
        "status": "ok", "semantics": "rust-events",
        "signals": output["signals"].as_array().map_or(0, Vec::len), "discards": output["discards"].as_array().map_or(0, Vec::len),
        "output": output, "report": report, "not_done": report["not_done"],
    }))
}

fn run_sensor(config: &Config, run_id: &str, snap: &str, data_class: &str, events: &[&PlatformEvent]) -> Result<Value, SourceError> {
    let date = |f: Option<&&PlatformEvent>| f.and_then(|e| e.event_time.get(..10)).unwrap_or("1970-01-01").to_owned();
    let doc = json!({
        "contract_version": "engine-steps/0", "step": "sensors", "run_id": run_id, "data_class": data_class,
        "source_snapshot_ref": format!("snapshot:{snap}@1"), "discovery_config_ref": "discovery_config:monitor@1",
        "window": {"start": date(events.first()), "end": date(events.last())}, "metric_spec_refs": ["metric_spec:platform-events@1"],
    });
    let initial = json!({"spec": {"sensors": doc}}).to_string();
    let env = StepEnv {
        runner_exe: config.runner_exe.clone(),
        snapshot_root: config.work_dir.join("packages"),
        lab_dir: config.work_dir.clone(),
        recompute_dir: config.work_dir.clone(),
        arranque: 30,
        min_support: config.min_support,
    };
    let handlers: Vec<_> = thread_handlers(env, None).into_iter().take(1).collect();
    let jobs = JobFileStore::open(config.work_dir.join("jobs").join(run_id)).map_err(io)?;
    let opts = Options { job_id: run_id.to_owned(), worker_id: "pulso-monitor".into(), crash_after: None, after_commit: None };
    let out = run_once(&jobs, &handlers, &initial, &opts).map_err(|e| SourceError::Sensor(format!("{e:?}")))?;
    let v: Value = serde_json::from_str(&out).map_err(io)?;
    let sensors = v.pointer("/out/sensors").cloned().ok_or_else(|| SourceError::Sensor("sensor produced no output".into()))?;
    Ok(json!({"status": "ok", "semantics": "claude-standin", "signals": sensors["signals"].as_array().map_or(0, Vec::len), "discards": sensors["discards"].as_array().map_or(0, Vec::len), "output": sensors}))
}

/// Days since 1970-01-01 of an ISO `YYYY-MM-DD...` prefix.
fn day_number(t: &str) -> Option<i64> {
    let b = t.get(..10)?;
    let (y, m, d) = (b.get(..4)?.parse::<i64>().ok()?, b.get(5..7)?.parse::<i64>().ok()?, b.get(8..10)?.parse::<i64>().ok()?);
    if b.as_bytes()[4] != b'-' || b.as_bytes()[7] != b'-' || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// History-free signals from the batch; history-dependent ones are gated (design section 7): below the minimum history they
/// report `insufficient_history` with what is missing, never a value and never a silent skip.
fn signals(events: &[&PlatformEvent], days: u64, cases: u64, c: &Config) -> Value {
    let count = |t: &str| events.iter().filter(|e| e.event_type == t).count() as u64;
    let free: Vec<Value> = [
        ("cases_opened", count("case.opened")),
        ("cases_closed", count("case.closed")),
        ("cases_assigned", count("case.assigned")),
        ("first_responses", count("case.first_responded")),
        ("turns_created", count("turn.created")),
    ]
    .into_iter()
    .map(|(name, n)| json!({"name": name, "value": n, "n": n, "weak": n < u64::from(c.min_support), "window": "batch"}))
    .collect();
    let (need_d, need_c) = (u64::from(c.min_history_days), u64::from(c.min_history_cases));
    let gated = days < need_d || cases < need_c;
    let dep: Vec<Value> = ["week_over_week_case_opened_change", "baseline_drift", "per_analyst_deviation"]
        .into_iter()
        .map(|name| {
            if gated {
                json!({"name": name, "status": "insufficient_history", "have": {"days": days, "cases": cases}, "need": {"days": need_d, "cases": need_c}, "missing": {"days": need_d.saturating_sub(days), "cases": need_c.saturating_sub(cases)}})
            } else if name == "week_over_week_case_opened_change" {
                wow(events, name)
            } else {
                json!({"name": name, "status": "not_implemented"})
            }
        })
        .collect();
    json!({"history_free": free, "history_dependent": dep})
}

fn wow(events: &[&PlatformEvent], name: &str) -> Value {
    let opened: Vec<i64> = events.iter().filter(|e| e.event_type == "case.opened").filter_map(|e| day_number(&e.event_time)).collect();
    let Some(&max) = opened.iter().max() else {
        return json!({"name": name, "status": "not_computable", "reason": "no_case_opened_in_batch"});
    };
    let last = opened.iter().filter(|d| **d > max - 7).count() as f64;
    let prev = opened.iter().filter(|d| **d <= max - 7 && **d > max - 14).count() as f64;
    if prev == 0.0 {
        return json!({"name": name, "status": "not_computable", "reason": "no_prior_window", "scope": "batch"});
    }
    json!({"name": name, "status": "computed", "value": (last - prev) / prev, "n": last + prev, "scope": "batch"})
}
