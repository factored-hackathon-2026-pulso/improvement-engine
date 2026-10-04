//! R1G: real Rust sensor over event packages (`sensor=rust-events`), no Core, no model, no payload text.
//!
//! Input: `events.ndjson` (identifier/enum/time columns of `event_log`, never `payload`) and the optional `cases.ndjson`
//! dimension (allow-listed `cases` columns: `case_id, channel, language, previous_case_id`). Cells are `language/channel`.
//!
//! Method (all documented in the report under `method`):
//! 1. Quarantine malformed or hostile rows (counted by reason, never echoed). Order by `sequence`, dedupe.
//! 2. Cold-start gate: history (days of event time, cases opened) below the gate => `insufficient_history`, no admission.
//! 3. Split by `event_log.sequence`: discovery = first `discovery_frac` of the sequence span, holdout = the rest. A case
//!    belongs to the window of its `case.opened` sequence.
//! 4. Per (family, cell): one-sided test "cell is WORSE than the rest of the population" (two-proportion z for rates,
//!    Welch z for delays). Families: `reassignment_rate` (case.assigned seen twice), `recurrence_rate`
//!    (`previous_case_id` set), `first_response_delay`, `resolution_delay`.
//! 5. Multiplicity: Bonferroni over every (family, cell) test of the discovery window, family-wise alpha = `alpha`.
//!    Candidates need nominal p <= alpha, a minimum effect size and p <= alpha/m.
//! 6. Replication: in the holdout the effect must have the same direction, reach the minimum support, and pass
//!    p <= alpha/K (Bonferroni over the K discovery survivors). Only then is a signal ADMITTED.
//! 7. Volume drift (cases per hour, discovery vs holdout) is reported as drift; load-sensitive (delay) candidates are then
//!    discarded as `drift_only` because load, not the cell, explains them.
//!
//! Every non-admitted candidate or untestable cell is a named discard: `insufficient_history`, `k_anonymity`,
//! `low_support`, `multiple_comparison`, `not_replicated`, `drift_only`. The frozen `engine-steps/0` output maps them to
//! its enum (`below_k`, `failed_holdout`, `low_coverage`, `other`); the exact names live in the `sensor-events/1` report.
use crate::StepError;
use crate::sensor::json::{self, Json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Params {
    /// Minimum count of positive cases (rates) in the holdout cell.
    pub min_support: u32,
    /// Minimum cases (or observed delays) per cell and window to run a test at all.
    pub min_cell_cases: u32,
    /// k-anonymity: a cell with fewer distinct cases is never reported.
    pub k_anon: u32,
    pub min_history_days: u32,
    pub min_history_cases: u32,
    pub alpha: f64,
    pub discovery_frac: f64,
    /// Minimum absolute rate difference (cell minus rest) for a candidate.
    pub min_rate_effect: f64,
    /// Minimum mean delay ratio (cell over rest) for a candidate.
    pub min_delay_ratio: f64,
    /// Volume drift: holdout/discovery arrival-rate ratio beyond this (either direction) with |z| >= 3.29.
    pub drift_ratio: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            min_support: 5,
            min_cell_cases: 30,
            k_anon: 10,
            min_history_days: 14,
            min_history_cases: 200,
            alpha: 0.05,
            discovery_frac: 0.6,
            min_rate_effect: 0.05,
            min_delay_ratio: 1.25,
            drift_ratio: 1.5,
        }
    }
}

impl Params {
    /// STEPS_MIN_SUPPORT, STEPS_ARRANQUE (min cases per cell), STEPS_K_ANON, STEPS_MIN_HISTORY_DAYS, STEPS_MIN_HISTORY_CASES.
    pub fn from_env() -> Params {
        let mut p = Params::default();
        let get = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u32>().ok());
        if let Some(v) = get("STEPS_MIN_SUPPORT") {
            p.min_support = v;
        }
        if let Some(v) = get("STEPS_ARRANQUE") {
            p.min_cell_cases = v;
        }
        if let Some(v) = get("STEPS_K_ANON") {
            p.k_anon = v;
        }
        if let Some(v) = get("STEPS_MIN_HISTORY_DAYS") {
            p.min_history_days = v;
        }
        if let Some(v) = get("STEPS_MIN_HISTORY_CASES") {
            p.min_history_cases = v;
        }
        p
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub metric_id: String,
    pub family: String,
    pub cell: String,
    pub numerator: u64,
    pub denominator: u64,
    pub discovery_cell: f64,
    pub discovery_rest: f64,
    pub holdout_cell: f64,
    pub holdout_rest: f64,
    pub p_discovery: f64,
    pub p_holdout: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Discard {
    pub metric_id: String,
    pub family: String,
    pub cell: String,
    pub reason: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drift {
    pub metric: String,
    pub discovery_per_hour: f64,
    pub holdout_per_hour: f64,
    pub ratio: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub signals: Vec<Signal>,
    pub discards: Vec<Discard>,
    pub drift: Vec<Drift>,
    pub quarantined: u64,
    pub quarantine_reasons: BTreeMap<String, u64>,
    pub tests_run: u64,
    pub alpha_per_test: f64,
    pub events: u64,
    pub cases: u64,
    pub cases_without_dimension: u64,
    pub history_days: u64,
    pub windows: [u64; 2],
    pub packages: u64,
}

const FAMILIES: [Family; 4] = [Family::Reassign, Family::Recurrence, Family::FirstResponse, Family::Resolution];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    Reassign,
    Recurrence,
    FirstResponse,
    Resolution,
}
impl Family {
    fn name(self) -> &'static str {
        match self {
            Family::Reassign => "reassignment_rate",
            Family::Recurrence => "recurrence_rate",
            Family::FirstResponse => "first_response_delay",
            Family::Resolution => "resolution_delay",
        }
    }
    fn is_delay(self) -> bool {
        matches!(self, Family::FirstResponse | Family::Resolution)
    }
}

fn frozen_reason(r: &str) -> &'static str {
    match r {
        "not_replicated" => "failed_holdout",
        "low_support" | "k_anonymity" => "below_k",
        "insufficient_history" => "low_coverage",
        _ => "other",
    }
}

fn r6(x: f64) -> f64 {
    if x.is_finite() { (x * 1e6).round() / 1e6 } else { 0.0 }
}

impl Report {
    /// Frozen `engine-steps/0` sensors output (signals + discards only).
    pub fn steps_output(&self, run_id: &str, data_class: &str) -> String {
        let signals: Vec<Json> = self
            .signals
            .iter()
            .enumerate()
            .map(|(i, s)| {
                Json::obj(vec![
                    ("signal_id", Json::Str(format!("sig-{:04}", i + 1))),
                    ("metric_id", Json::s(&s.metric_id)),
                    ("population", Json::Str(s.cell.clone())),
                    ("numerator", Json::Int(s.numerator as i64)),
                    ("denominator", Json::Int(s.denominator.max(1) as i64)),
                    ("holdout_checked", Json::Bool(true)),
                    ("evidence_ref", Json::Str(format!("ev-{:04}", i + 1))),
                ])
            })
            .collect();
        let discards: Vec<Json> = self
            .discards
            .iter()
            .map(|d| Json::obj(vec![("metric_id", Json::s(&d.metric_id)), ("reason", Json::s(frozen_reason(&d.reason)))]))
            .collect();
        Json::obj(vec![
            ("contract_version", Json::s("engine-steps/0")),
            ("step", Json::s("sensors")),
            ("run_id", Json::s(run_id)),
            ("data_class", Json::s(data_class)),
            ("signals", Json::Arr(signals)),
            ("discards", Json::Arr(discards)),
        ])
        .write()
    }

    /// Versioned superset `sensor-events/1`: exact discard names, statistics, drift, quarantine counts, what was NOT done.
    pub fn to_json(&self) -> String {
        let f = |x: f64| Json::Float(r6(x));
        let signals: Vec<Json> = self
            .signals
            .iter()
            .enumerate()
            .map(|(i, s)| {
                Json::obj(vec![
                    ("signal_id", Json::Str(format!("sig-{:04}", i + 1))),
                    ("metric_id", Json::s(&s.metric_id)),
                    ("family", Json::s(&s.family)),
                    ("cell", Json::s(&s.cell)),
                    ("numerator", Json::Int(s.numerator as i64)),
                    ("denominator", Json::Int(s.denominator as i64)),
                    ("discovery", Json::obj(vec![("cell", f(s.discovery_cell)), ("rest", f(s.discovery_rest)), ("p", f(s.p_discovery))])),
                    ("holdout", Json::obj(vec![("cell", f(s.holdout_cell)), ("rest", f(s.holdout_rest)), ("p", f(s.p_holdout))])),
                ])
            })
            .collect();
        let discards: Vec<Json> = self
            .discards
            .iter()
            .map(|d| {
                Json::obj(vec![
                    ("metric_id", Json::s(&d.metric_id)),
                    ("family", Json::s(&d.family)),
                    ("cell", Json::s(&d.cell)),
                    ("reason", Json::s(&d.reason)),
                    ("frozen_reason", Json::s(frozen_reason(&d.reason))),
                    ("detail", Json::s(&d.detail)),
                ])
            })
            .collect();
        let drift: Vec<Json> = self
            .drift
            .iter()
            .map(|d| Json::obj(vec![("metric", Json::s(&d.metric)), ("discovery_per_hour", f(d.discovery_per_hour)), ("holdout_per_hour", f(d.holdout_per_hour)), ("ratio", f(d.ratio)), ("z", f(d.z))]))
            .collect();
        let q: Vec<(String, Json)> = self.quarantine_reasons.iter().map(|(k, v)| (k.clone(), Json::Int(*v as i64))).collect();
        let not_done = [
            "payload and free text are never read",
            "no per-staff or per-customer analysis; no identifiers in output",
            "cells without a cases dimension row are excluded from cell tests",
            "improvements (cell better than rest) are not candidates",
            "no causal claim: an admitted signal is a replicated deviation, not a proven root cause",
            "no seasonality or week-over-week model; release and observation stay simulated",
        ];
        Json::obj(vec![
            ("contract", Json::s("sensor-events/1")),
            ("sensor", Json::s("rust-events")),
            ("method", Json::obj(vec![
                ("multiple_comparison", Json::s("bonferroni over all (family, cell) tests in discovery; replication at alpha/K over the K survivors")),
                ("alpha", f(self.alpha_per_test)),
                ("test", Json::s("one-sided: cell worse than rest; two-proportion z (rates), Welch z (delays)")),
                ("split", Json::s("event_log.sequence: discovery first fraction of the span, holdout the rest")),
            ])),
            ("tests_run", Json::Int(self.tests_run as i64)),
            ("history", Json::obj(vec![("days", Json::Int(self.history_days as i64)), ("cases", Json::Int(self.cases as i64)), ("events", Json::Int(self.events as i64)), ("packages", Json::Int(self.packages as i64))])),
            ("windows_cases", Json::obj(vec![("discovery", Json::Int(self.windows[0] as i64)), ("holdout", Json::Int(self.windows[1] as i64))])),
            ("cases_without_dimension", Json::Int(self.cases_without_dimension as i64)),
            ("quarantined", Json::obj(vec![("total", Json::Int(self.quarantined as i64)), ("by_reason", Json::Obj(q))])),
            ("signals", Json::Arr(signals)),
            ("discards", Json::Arr(discards)),
            ("drift", Json::Arr(drift)),
            ("not_done", Json::Arr(not_done.iter().map(|s| Json::s(s)).collect())),
        ])
        .write()
    }
}

// ---------------------------------------------------------------------------------------------------------------- parsing

struct Ev {
    seq: i64,
    ty: String,
    case: Option<String>,
    t: i64,
}

fn clean(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

fn label(s: &str) -> bool {
    !s.is_empty() && s.len() <= 16 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn etype(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.' || b == b'-')
}

/// `YYYY-MM-DDTHH:MM:SS[.fff]Z` (or `+00:00`) to epoch seconds.
fn epoch(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if s.len() < 20 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |a: usize, z: usize| s.get(a..z).and_then(|x| x.parse::<i64>().ok()).filter(|v| *v >= 0);
    let (y, m, d, hh, mm, ss) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 || y < 1970 {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hh * 3600 + mm * 60 + ss)
}

fn quarantine(r: &mut Report, why: &str) {
    r.quarantined += 1;
    *r.quarantine_reasons.entry(why.to_string()).or_insert(0) += 1;
}

const MAX_LINE: usize = 8192;

fn parse_event(line: &str) -> Result<Ev, &'static str> {
    if line.len() > MAX_LINE {
        return Err("oversized_row");
    }
    let v = json::parse(line).map_err(|_| "malformed_json")?;
    if !matches!(v, Json::Obj(_)) {
        return Err("not_an_object");
    }
    let seq = v.get("sequence").and_then(Json::as_i64).filter(|s| *s >= 0).ok_or("bad_sequence")?;
    v.get("event_id").and_then(Json::as_str).filter(|s| clean(s, 128)).ok_or("bad_event_id")?;
    let ty = v.get("event_type").and_then(Json::as_str).filter(|s| etype(s)).ok_or("bad_event_type")?;
    let t = v.get("event_time").and_then(Json::as_str).and_then(epoch).ok_or("bad_event_time")?;
    let case = match v.get("case_id") {
        None | Some(Json::Null) => None,
        Some(Json::Str(s)) if clean(s, 128) => Some(s.clone()),
        Some(_) => return Err("bad_case_id"),
    };
    Ok(Ev { seq, ty: ty.to_string(), case, t })
}

struct Dim {
    cell: String,
    prev: bool,
}

fn parse_dim(line: &str) -> Result<(String, Dim), &'static str> {
    if line.len() > MAX_LINE {
        return Err("oversized_row");
    }
    let v = json::parse(line).map_err(|_| "malformed_json")?;
    let id = v.get("case_id").and_then(Json::as_str).filter(|s| clean(s, 128)).ok_or("bad_case_id")?;
    let lang = v.get("language").and_then(Json::as_str).filter(|s| label(s)).ok_or("bad_dimension_label")?;
    let chan = v.get("channel").and_then(Json::as_str).filter(|s| label(s)).ok_or("bad_dimension_label")?;
    let prev = match v.get("previous_case_id") {
        None | Some(Json::Null) => false,
        Some(Json::Str(s)) if clean(s, 128) => true,
        Some(_) => return Err("bad_previous_case_id"),
    };
    Ok((id.to_string(), Dim { cell: format!("{lang}/{chan}"), prev }))
}

// ---------------------------------------------------------------------------------------------------------------- stats

fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let r = t
        * (-z * z - 1.265_512_23
            + t * (1.000_023_68 + t * (0.374_091_96 + t * (0.096_784_18 + t * (-0.186_288_06 + t * (0.278_868_07 + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    if x >= 0.0 { r } else { 2.0 - r }
}

/// One-sided upper-tail p of a standard normal z.
fn p_upper(z: f64) -> f64 {
    0.5 * erfc(z / std::f64::consts::SQRT_2)
}

#[derive(Clone, Copy, Default)]
struct G {
    n: u64,
    s: f64,
    ss: f64,
}
impl G {
    fn add(&mut self, v: f64) {
        self.n += 1;
        self.s += v;
        self.ss += v * v;
    }
    fn minus(self, o: G) -> G {
        G { n: self.n - o.n, s: self.s - o.s, ss: self.ss - o.ss }
    }
    fn mean(self) -> f64 {
        if self.n == 0 { 0.0 } else { self.s / self.n as f64 }
    }
    fn var(self) -> f64 {
        if self.n < 2 { 0.0 } else { ((self.ss - self.n as f64 * self.mean().powi(2)) / (self.n - 1) as f64).max(0.0) }
    }
}

/// (z, effect) of cell versus rest. Rates: pooled two-proportion z, effect = difference. Delays: Welch z, effect = mean ratio.
fn compare(f: Family, c: G, r: G) -> (f64, f64) {
    if c.n == 0 || r.n == 0 {
        return (0.0, 0.0);
    }
    if f.is_delay() {
        let se = (c.var() / c.n as f64 + r.var() / r.n as f64).sqrt();
        let z = if se > 0.0 { (c.mean() - r.mean()) / se } else { 0.0 };
        (z, if r.mean() > 0.0 { c.mean() / r.mean() } else { 0.0 })
    } else {
        let (n1, n2) = (c.n as f64, r.n as f64);
        let p = (c.s + r.s) / (n1 + n2);
        let se = (p * (1.0 - p) * (1.0 / n1 + 1.0 / n2)).sqrt();
        let d = c.mean() - r.mean();
        (if se > 0.0 { d / se } else { 0.0 }, d)
    }
}

// ---------------------------------------------------------------------------------------------------------------- analysis

struct CaseAgg {
    opened_seq: i64,
    opened_t: i64,
    assigns: u32,
    first_resp: Option<i64>,
    closed: Option<i64>,
}

struct Sample {
    w: usize,
    cell: String,
    vals: [Option<f64>; 4],
}

fn metric_id(f: Family, cell: &str) -> String {
    format!("{}.{}", f.name(), cell.replace('/', "."))
}

/// Analyse concatenated `events.ndjson` and `cases.ndjson` text. Pure and deterministic: rows are ordered by `sequence`.
pub fn analyze(events: &str, cases: &str, p: &Params) -> Report {
    let mut rep = Report { alpha_per_test: p.alpha, ..Report::default() };
    let mut evs: BTreeMap<i64, Ev> = BTreeMap::new();
    let mut ids: HashMap<String, ()> = HashMap::new();
    for line in events.lines().filter(|l| !l.trim().is_empty()) {
        match parse_event(line) {
            Err(why) => quarantine(&mut rep, why),
            Ok(e) => {
                let id = json::parse(line).ok().and_then(|v| v.get("event_id").and_then(Json::as_str).map(str::to_string)).unwrap_or_default();
                if evs.contains_key(&e.seq) {
                    quarantine(&mut rep, "duplicate_sequence");
                } else if ids.insert(id, ()).is_some() {
                    quarantine(&mut rep, "duplicate_event_id");
                } else {
                    evs.insert(e.seq, e);
                }
            }
        }
    }
    let mut dims: HashMap<String, Dim> = HashMap::new();
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        match parse_dim(line) {
            Err(why) => quarantine(&mut rep, why),
            Ok((id, d)) => {
                dims.entry(id).or_insert(d);
            }
        }
    }
    rep.events = evs.len() as u64;

    let mut agg: BTreeMap<&str, CaseAgg> = BTreeMap::new();
    for e in evs.values() {
        let Some(c) = e.case.as_deref() else { continue };
        match e.ty.as_str() {
            "case.opened" => {
                agg.entry(c).or_insert(CaseAgg { opened_seq: e.seq, opened_t: e.t, assigns: 0, first_resp: None, closed: None });
            }
            _ => {}
        }
    }
    for e in evs.values() {
        let Some(a) = e.case.as_deref().and_then(|c| agg.get_mut(c)) else { continue };
        match e.ty.as_str() {
            "case.assigned" => a.assigns += 1,
            "case.first_responded" => a.first_resp = a.first_resp.or(Some(e.t)),
            "case.closed" => a.closed = a.closed.or(Some(e.t)),
            _ => {}
        }
    }
    rep.cases = agg.len() as u64;
    let (Some((&seq_min, _)), Some((&seq_max, _))) = (evs.iter().next(), evs.iter().next_back()) else {
        gate_all(&mut rep, "no events");
        return rep;
    };
    let (t_min, t_max) = (evs.values().map(|e| e.t).min().unwrap_or(0), evs.values().map(|e| e.t).max().unwrap_or(0));
    rep.history_days = ((t_max - t_min).max(0) / 86_400) as u64;
    if rep.history_days < u64::from(p.min_history_days) || rep.cases < u64::from(p.min_history_cases) {
        let detail = format!("have {} days and {} cases; need {} days and {} cases", rep.history_days, rep.cases, p.min_history_days, p.min_history_cases);
        gate_all(&mut rep, &detail);
        return rep;
    }

    let split = seq_min + ((seq_max - seq_min) as f64 * p.discovery_frac) as i64;
    let t_split = evs.range(..=split).next_back().map_or(t_min, |(_, e)| e.t);
    let mut samples: Vec<Sample> = vec![];
    let mut n_win = [0u64; 2];
    for (id, a) in &agg {
        let w = usize::from(a.opened_seq > split);
        n_win[w] += 1;
        let Some(d) = dims.get(*id) else {
            rep.cases_without_dimension += 1;
            continue;
        };
        let delay = |t: Option<i64>| t.filter(|t| *t >= a.opened_t).map(|t| (t - a.opened_t) as f64);
        samples.push(Sample {
            w,
            cell: d.cell.clone(),
            vals: [Some(f64::from(a.assigns >= 2)), Some(f64::from(d.prev)), delay(a.first_resp), delay(a.closed)],
        });
    }
    rep.windows = n_win;

    // volume drift
    let (d1, d2) = ((t_split - t_min) as f64, (t_max - t_split) as f64);
    let mut drifted = false;
    if d1 > 0.0 && d2 > 0.0 && n_win[0] > 0 && n_win[1] > 0 {
        let n = (n_win[0] + n_win[1]) as f64;
        let p0 = d2 / (d1 + d2);
        let z = (n_win[1] as f64 - n * p0) / (n * p0 * (1.0 - p0)).sqrt();
        let (r1, r2) = (n_win[0] as f64 / d1 * 3600.0, n_win[1] as f64 / d2 * 3600.0);
        let ratio = r2 / r1;
        if z.abs() >= 3.29 && (ratio >= p.drift_ratio || ratio <= 1.0 / p.drift_ratio) {
            drifted = true;
            rep.drift.push(Drift { metric: "volume_level".into(), discovery_per_hour: r1, holdout_per_hour: r2, ratio, z });
            rep.discards.push(Discard {
                metric_id: "volume_level.all".into(),
                family: "volume_level".into(),
                cell: "all".into(),
                reason: "drift_only".into(),
                detail: format!("arrival rate x{ratio:.2} between windows; reported as drift, not an opportunity"),
            });
        }
    }

    // group sums per (family, window): total, and per cell
    let mut cell_n: BTreeMap<&str, u64> = BTreeMap::new();
    for s in &samples {
        *cell_n.entry(s.cell.as_str()).or_insert(0) += 1;
    }
    let mut tot = [[G::default(); 2]; 4];
    let mut by_cell: BTreeMap<&str, [[G; 2]; 4]> = BTreeMap::new();
    for s in &samples {
        let cg = by_cell.entry(s.cell.as_str()).or_insert([[G::default(); 2]; 4]);
        for (fi, v) in s.vals.iter().enumerate() {
            if let Some(v) = v {
                tot[fi][s.w].add(*v);
                cg[fi][s.w].add(*v);
            }
        }
    }

    struct Cand {
        fi: usize,
        cell: String,
        pd: f64,
    }
    let mut testable: Vec<(usize, &str)> = vec![];
    for (cell, g) in &by_cell {
        for (fi, f) in FAMILIES.iter().enumerate() {
            if cell_n[cell] < u64::from(p.k_anon) {
                rep.discards.push(Discard {
                    metric_id: metric_id(*f, cell),
                    family: f.name().into(),
                    cell: (*cell).into(),
                    reason: "k_anonymity".into(),
                    detail: format!("cell has fewer than k={} cases; never reported", p.k_anon),
                });
                continue;
            }
            let min = u64::from(p.min_cell_cases);
            let ok = |w: usize| g[fi][w].n >= min && tot[fi][w].n - g[fi][w].n >= min;
            if !(ok(0) && ok(1)) {
                rep.discards.push(Discard {
                    metric_id: metric_id(*f, cell),
                    family: f.name().into(),
                    cell: (*cell).into(),
                    reason: "low_support".into(),
                    detail: format!("fewer than {min} observations in a window"),
                });
                continue;
            }
            testable.push((fi, cell));
        }
    }
    let m = testable.len().max(1) as f64;
    rep.tests_run = testable.len() as u64;
    let mut survivors: Vec<Cand> = vec![];
    for (fi, cell) in &testable {
        let f = FAMILIES[*fi];
        let cg = by_cell[cell][*fi][0];
        let (z, eff) = compare(f, cg, tot[*fi][0].minus(cg));
        let pd = p_upper(z);
        let min_eff = if f.is_delay() { p.min_delay_ratio } else { p.min_rate_effect };
        if pd > p.alpha || eff < min_eff {
            continue; // not a candidate: not a discard (would list every healthy cell)
        }
        if pd > p.alpha / m {
            rep.discards.push(Discard {
                metric_id: metric_id(f, cell),
                family: f.name().into(),
                cell: (*cell).to_string(),
                reason: "multiple_comparison".into(),
                detail: format!("p={pd:.4} does not survive Bonferroni over {m} tests"),
            });
            continue;
        }
        if f.is_delay() && drifted {
            rep.discards.push(Discard {
                metric_id: metric_id(f, cell),
                family: f.name().into(),
                cell: (*cell).to_string(),
                reason: "drift_only".into(),
                detail: "load-sensitive delay deviates while arrival volume drifts; not attributed to the cell".into(),
            });
            continue;
        }
        survivors.push(Cand { fi: *fi, cell: (*cell).to_string(), pd });
    }
    let k = survivors.len().max(1) as f64;
    for c in survivors {
        let f = FAMILIES[c.fi];
        let (cg, tg) = (by_cell[c.cell.as_str()][c.fi][1], tot[c.fi][1]);
        let rg = tg.minus(cg);
        let (zh, eh) = compare(f, cg, rg);
        let ph = p_upper(zh);
        let support = if f.is_delay() { cg.n >= u64::from(p.min_cell_cases) } else { cg.s as u64 >= u64::from(p.min_support) };
        let min_eff = if f.is_delay() { p.min_delay_ratio } else { p.min_rate_effect };
        let weak = if f.is_delay() { eh < 1.0 + (min_eff - 1.0) / 2.0 } else { eh < min_eff / 2.0 };
        let (reason, detail) = if !support {
            ("low_support", format!("holdout support below minimum ({} positives)", cg.s as u64))
        } else if ph > p.alpha / k || weak {
            ("not_replicated", format!("holdout p={ph:.4} (needed <= {:.4}) or direction/effect lost", p.alpha / k))
        } else {
            ("", String::new())
        };
        if !reason.is_empty() {
            rep.discards.push(Discard { metric_id: metric_id(f, &c.cell), family: f.name().into(), cell: c.cell.clone(), reason: reason.into(), detail });
            continue;
        }
        let cd = by_cell[c.cell.as_str()][c.fi][0];
        let rest_all = (tot[c.fi][0].minus(cd).s + rg.s) / ((tot[c.fi][0].minus(cd).n + rg.n).max(1) as f64);
        let numerator = if f.is_delay() { delay_numerator(&samples, &c.cell, c.fi, rest_all) } else { (cg.s + cd.s) as u64 };
        rep.signals.push(Signal {
            metric_id: metric_id(f, &c.cell),
            family: f.name().into(),
            cell: c.cell.clone(),
            numerator,
            denominator: cg.n + cd.n,
            discovery_cell: cd.mean(),
            discovery_rest: tot[c.fi][0].minus(cd).mean(),
            holdout_cell: cg.mean(),
            holdout_rest: rg.mean(),
            p_discovery: c.pd,
            p_holdout: ph,
        });
    }
    rep.signals.sort_by(|a, b| (a.family.as_str(), a.cell.as_str()).cmp(&(b.family.as_str(), b.cell.as_str())));
    rep.discards.sort_by(|a, b| (a.metric_id.as_str(), a.reason.as_str()).cmp(&(b.metric_id.as_str(), b.reason.as_str())));
    rep
}

/// Delay metrics: numerator = cell cases slower than the rest's mean delay (aggregate count, no identifiers).
fn delay_numerator(samples: &[Sample], cell: &str, fi: usize, rest_mean: f64) -> u64 {
    samples.iter().filter(|s| s.cell == cell && s.vals[fi].is_some_and(|v| v > rest_mean)).count() as u64
}

fn gate_all(rep: &mut Report, detail: &str) {
    for f in ["reassignment_rate", "recurrence_rate", "first_response_delay", "resolution_delay", "volume_level"] {
        rep.discards.push(Discard { metric_id: format!("{f}.all"), family: f.into(), cell: "all".into(), reason: "insufficient_history".into(), detail: detail.into() });
    }
}

// ---------------------------------------------------------------------------------------------------------------- packages

const MAX_BYTES: u64 = 96 << 20;

fn seq_of(wm: &str) -> Option<i64> {
    wm.strip_prefix("seq:").and_then(|n| n.parse().ok())
}

/// Analyse the package `root/pkg` together with the earlier packages of the same source (same `source_id`, watermark_to
/// not beyond this one), newest first until the size budget. Replay of an old batch therefore sees only its own past.
pub fn analyze_package(root: &Path, pkg: &str, p: &Params) -> Result<Report, StepError> {
    let io = |what: &str| StepError::Io(what.to_string());
    let manifest = |dir: &Path| -> Option<(String, i64, i64)> {
        let v = json::parse(&std::fs::read_to_string(dir.join("manifest.json")).ok()?).ok()?;
        let src = v.get("source_id").and_then(Json::as_str)?.to_string();
        let (a, b) = (seq_of(v.get("watermark_from").and_then(Json::as_str)?)?, seq_of(v.get("watermark_to").and_then(Json::as_str)?)?);
        Some((src, a, b))
    };
    if pkg.is_empty() || pkg.contains(['/', '\\']) || pkg.starts_with('.') {
        return Err(StepError::Invalid("bad package name".into()));
    }
    let cur = root.join(pkg);
    let (src, _, to) = manifest(&cur).ok_or_else(|| io("package manifest.json missing or unusable"))?;
    let mut found: Vec<(i64, PathBuf)> = vec![];
    for e in std::fs::read_dir(root).map_err(|_| io("cannot list packages"))?.filter_map(Result::ok) {
        let d = e.path();
        if let Some((s, a, b)) = manifest(&d).filter(|(s, _, b)| *s == src && *b <= to) {
            let _ = (s, b);
            found.push((a, d));
        }
    }
    found.sort();
    let (mut events, mut cases, mut size, mut used) = (String::new(), String::new(), 0u64, 0u64);
    for (_, d) in found.iter().rev() {
        let (ef, cf) = (d.join("events.ndjson"), d.join("cases.ndjson"));
        let len = std::fs::metadata(&ef).map_err(|_| io("events.ndjson missing"))?.len() + std::fs::metadata(&cf).map_or(0, |m| m.len());
        if size + len > MAX_BYTES && used > 0 {
            break;
        }
        size += len;
        used += 1;
        events.push_str(&String::from_utf8_lossy(&std::fs::read(&ef).map_err(|_| io("cannot read events.ndjson"))?));
        if let Ok(b) = std::fs::read(&cf) {
            cases.push_str(&String::from_utf8_lossy(&b));
        }
    }
    let mut rep = analyze(&events, &cases, p);
    rep.packages = used;
    Ok(rep)
}

/// Engine `sensors` step over a package: validates the frozen input envelope, returns (frozen output, superset report).
pub fn run_package(input: &str, root: &Path, p: &Params) -> Result<(String, String), StepError> {
    let v = json::parse(input)?;
    let (run_id, data_class) = crate::sensor::envelope(&v, "sensors")?;
    let inv = |m: &str| StepError::Invalid(m.to_string());
    let snap = crate::sensor::ref_id(v.get("source_snapshot_ref").and_then(Json::as_str).ok_or_else(|| inv("source_snapshot_ref"))?)?;
    crate::sensor::ref_id(v.get("discovery_config_ref").and_then(Json::as_str).ok_or_else(|| inv("discovery_config_ref"))?)?;
    v.get("window").and_then(|w| w.get("start")).and_then(Json::as_str).ok_or_else(|| inv("window.start"))?;
    v.get("window").and_then(|w| w.get("end")).and_then(Json::as_str).ok_or_else(|| inv("window.end"))?;
    if v.get("metric_spec_refs").and_then(Json::as_arr).is_none_or(<[Json]>::is_empty) {
        return Err(inv("metric_spec_refs must not be empty"));
    }
    let rep = analyze_package(root, snap, p)?;
    Ok((rep.steps_output(&run_id, &data_class), rep.to_json()))
}

/// Used by `sensor::run` when `STEPS_SENSOR=rust-events`: package root from STEPS_SNAPSHOT_ROOT, params from the env.
pub fn run_env(input: &str) -> Result<String, StepError> {
    let root = crate::sensor::env_path("STEPS_SNAPSHOT_ROOT")?;
    run_package(input, &root, &Params::from_env()).map(|(out, _)| out)
}
