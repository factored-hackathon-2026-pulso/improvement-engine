//! L1 `cells` sensor (`semantics: claude-standin`): deterministic recurring-problem signals over
//! TREATED cell tables (aggregates only, k >= 10), the dataset-mode sibling of the event sensor.
//!
//! Input (ndjson, one cell per line, produced by `scripts/aggregate/bank_cells.py`):
//! `{"metric":"M1","dims":{"reason_category":"Queja","channel":"Phone"},"half":"discovery","numerator":120,"denominator":600}`
//! `half` is `discovery` or `holdout` (a deterministic customer-hash 60/40 split made by the aggregator).
//!
//! Method (declared in the output): two-proportion pooled z-test of each cell against the rest of its
//! metric (same half), Benjamini-Hochberg (default) or Bonferroni over ALL explored cells of the package, discovery/holdout replication,
//! named discards, status vocabulary `candidate | corroborated | refuted | uncertain`.
//! Signals are associations (`claim: association`); the sensor never names a cause or mechanism.

use std::collections::BTreeMap;

use crate::StepError;
use crate::sensor::json::{Json, parse};

/// Dimension keys a treated cell table may carry. Anything else (ids, free text) is rejected.
pub const ALLOWED_DIMS: [&str; 6] = ["reason_category", "channel", "category", "case_type", "priority", "survey_type"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplicity {
    /// Benjamini-Hochberg FDR over ALL explored cells (default, per the bank data audit).
    Bh,
    /// Bonferroni over ALL explored cells.
    Bonferroni,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// (dependent metric, parent metric): a dependent finding on a cell where the parent also has a
    /// finding is flagged `depends_on`, never dropped.
    pub dependencies: Vec<(String, String)>,
    pub multiplicity: Multiplicity,
    /// Minimum count for every emitted number (cell numerator, complement, denominator).
    pub k_min: i64,
    /// Discovery level: BH q, or Bonferroni family-wise alpha; holdout replication uses the same level.
    pub alpha: f64,
    /// Minimum rate ratio vs the rest of the metric (discovery).
    pub min_ratio: f64,
    /// Minimum absolute rate difference vs the rest of the metric (discovery); holdout needs half of it.
    pub min_effect: f64,
    /// Minimum cell denominator in discovery.
    pub min_support: i64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            dependencies: [("M6", "M1"), ("M6R", "M1"), ("M6U", "M1")]
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            multiplicity: Multiplicity::Bh, k_min: 10, alpha: 0.01, min_ratio: 1.25, min_effect: 0.05, min_support: 500 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub numerator: i64,
    pub denominator: i64,
    pub rate: f64,
    pub baseline_rate: f64,
    pub diff: f64,
    pub p: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub metric: String,
    pub dims: Vec<(String, String)>,
    pub status: &'static str,
    pub reason: &'static str,
    pub direction: &'static str,
    pub discovery: Option<Stage>,
    pub holdout: Option<Stage>,
    pub p_adj: Option<f64>,
    /// Secondary replication over two long windows (R2), reported alongside the A/B split.
    pub r2: Option<R2>,
    /// Parent metric whose finding on the same cell explains this one (e.g. CSAT low depends on M1).
    pub depends_on: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct R2 {
    /// `replicated | not_replicated | reversed | not_evaluated`
    pub status: &'static str,
    pub w1: Option<Stage>,
    pub w2: Option<Stage>,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub cells_explored: usize,
    pub signals: Vec<Signal>,
    /// (kind, count), sorted by kind.
    pub discards: Vec<(String, i64)>,
    pub config: Config,
}

type Key = (String, Vec<(String, String)>);

struct Cell {
    num: i64,
    den: i64,
}

fn invalid<T>(m: impl Into<String>) -> Result<T, StepError> {
    Err(StepError::Invalid(m.into()))
}

/// Complementary error function (Numerical Recipes `erfcc`, relative error < 1.2e-7).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.26551223
        + t * (1.00002368
            + t * (0.37409196
                + t * (0.09678418
                    + t * (-0.18628806
                        + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277))))))));
    let ans = t * poly.exp();
    if x >= 0.0 { ans } else { 2.0 - ans }
}

/// Two-sided p-value of the pooled two-proportion z-test (cell vs rest).
fn two_prop(x1: i64, n1: i64, x0: i64, n0: i64) -> (f64, f64) {
    let (p1, p0) = (x1 as f64 / n1 as f64, x0 as f64 / n0 as f64);
    let pp = (x1 + x0) as f64 / (n1 + n0) as f64;
    let se = (pp * (1.0 - pp) * (1.0 / n1 as f64 + 1.0 / n0 as f64)).sqrt();
    if se == 0.0 {
        return (p1 - p0, 1.0);
    }
    let z = (p1 - p0) / se;
    (p1 - p0, erfc(z.abs() / std::f64::consts::SQRT_2).clamp(0.0, 1.0))
}

fn r6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

fn k_ok(c: &Cell, k: i64) -> bool {
    c.den >= k && (c.num == 0 || c.num >= k) && (c.den - c.num == 0 || c.den - c.num >= k)
}

struct Row {
    key: Key,
    half: usize,
    period: Option<String>,
    cell: Cell,
}

/// Window of a `YYYY-MM` period for the secondary R2 replication (partial months fall outside both).
fn window_of(period: &str) -> Option<usize> {
    if ("2023-07"..="2024-12").contains(&period) {
        Some(0)
    } else if ("2025-01"..="2026-05").contains(&period) {
        Some(1)
    } else {
        None
    }
}

fn valid_period(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b[..4].iter().all(|c| c.is_ascii_digit())
        && b[5..].iter().all(|c| c.is_ascii_digit())
        && (1..=12).contains(&p[5..].parse::<u32>().unwrap_or(0))
}

fn parse_rows(input: &str) -> Result<Vec<Row>, StepError> {
    let mut rows = vec![];
    let mut seen = std::collections::BTreeSet::new();
    for (ln, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v = parse(line).map_err(|e| StepError::Invalid(format!("line {}: {e}", ln + 1)))?;
        let Json::Obj(kv) = &v else { return invalid(format!("line {}: not an object", ln + 1)) };
        for (k, _) in kv {
            if !["metric", "dims", "half", "period", "numerator", "denominator"].contains(&k.as_str()) {
                return invalid(format!("line {}: field `{k}` is not allowed in a treated cell table", ln + 1));
            }
        }
        let metric = v.get("metric").and_then(|m| m.as_str()).ok_or_else(|| StepError::Invalid(format!("line {}: metric", ln + 1)))?;
        let Some(Json::Obj(dkv)) = v.get("dims") else { return invalid(format!("line {}: dims", ln + 1)) };
        let mut dims = vec![];
        for (k, dv) in dkv {
            if !ALLOWED_DIMS.contains(&k.as_str()) {
                return invalid(format!("line {}: dimension `{k}` is not allowed (aggregates only)", ln + 1));
            }
            let Some(s) = dv.as_str() else { return invalid(format!("line {}: dimension value", ln + 1)) };
            dims.push((k.clone(), s.to_string()));
        }
        dims.sort();
        let half = match v.get("half").and_then(|h| h.as_str()) {
            Some("discovery") => 0,
            Some("holdout") => 1,
            _ => return invalid(format!("line {}: half must be discovery or holdout", ln + 1)),
        };
        let period = match v.get("period") {
            None => None,
            Some(Json::Str(p)) if valid_period(p) => Some(p.clone()),
            _ => return invalid(format!("line {}: period must be YYYY-MM", ln + 1)),
        };
        let num = v.get("numerator").and_then(|x| x.as_i64()).ok_or_else(|| StepError::Invalid(format!("line {}: numerator", ln + 1)))?;
        let den = v.get("denominator").and_then(|x| x.as_i64()).ok_or_else(|| StepError::Invalid(format!("line {}: denominator", ln + 1)))?;
        if num < 0 || den < 0 || num > den {
            return invalid(format!("line {}: need 0 <= numerator <= denominator", ln + 1));
        }
        let key: Key = (metric.to_string(), dims);
        if !seen.insert((key.clone(), half, period.clone())) {
            return invalid(format!("line {}: duplicate cell row", ln + 1));
        }
        rows.push(Row { key, half, period, cell: Cell { num, den } });
    }
    Ok(rows)
}

/// Slots: 0 discovery, 1 holdout, 2 window W1, 3 window W2.
type Slots = [Option<Cell>; 4];

/// Comparison group of a cell: the same metric and the same non-reason dimensions (same channel),
/// excluding the cell's own reason. Cells without a reason or with only a reason dimension compare
/// against the rest of the metric.
fn stratum(dims: &[(String, String)]) -> Vec<(String, String)> {
    if dims.len() > 1 && dims.iter().any(|(k, _)| k == "reason_category") {
        dims.iter().filter(|(k, _)| k != "reason_category").cloned().collect()
    } else {
        vec![]
    }
}

fn stage(cell: &Cell, rest_num: i64, rest_den: i64) -> (Stage, bool) {
    let (diff, p) = two_prop(cell.num, cell.den, rest_num, rest_den);
    let rate = cell.num as f64 / cell.den as f64;
    let base = rest_num as f64 / rest_den as f64;
    (Stage { numerator: cell.num, denominator: cell.den, rate: r6(rate), baseline_rate: r6(base), diff: r6(diff), p }, diff > 0.0)
}

pub fn analyse(input: &str, cfg: &Config) -> Result<Report, StepError> {
    let rows = parse_rows(input)?;
    let mut discards: BTreeMap<String, i64> = BTreeMap::new();
    let mut bump = |k: &str| *discards.entry(k.to_string()).or_insert(0) += 1;

    // k rule: a violating row is a named discard and is dropped everywhere (never tested, not in baselines).
    let mut valid: BTreeMap<Key, Slots> = BTreeMap::new();
    let add = |slot: &mut Option<Cell>, c: &Cell| match slot {
        Some(x) => {
            x.num += c.num;
            x.den += c.den;
        }
        None => *slot = Some(Cell { num: c.num, den: c.den }),
    };
    for r in rows {
        if !k_ok(&r.cell, cfg.k_min) {
            bump("k_violation");
            continue;
        }
        let slots = valid.entry(r.key).or_insert([None, None, None, None]);
        add(&mut slots[r.half], &r.cell);
        if let Some(w) = r.period.as_deref().and_then(window_of) {
            add(&mut slots[2 + w], &r.cell);
        }
    }
    let before = valid.len();
    valid.retain(|_, s| s[0].is_some());
    for _ in 0..(before - valid.len()) {
        bump("holdout_without_discovery");
    }

    // Totals per (metric, comparison stratum, slot); baseline = stratum total minus the cell itself.
    let mut totals: BTreeMap<(String, Vec<(String, String)>, usize), (i64, i64)> = BTreeMap::new();
    for ((metric, dims), slots) in &valid {
        let st = stratum(dims);
        for (i, c) in slots.iter().enumerate() {
            if let Some(c) = c {
                let t = totals.entry((metric.clone(), st.clone(), i)).or_insert((0, 0));
                t.0 += c.num;
                t.1 += c.den;
            }
        }
    }
    let rest = |key: &Key, slot: usize, c: &Cell| -> Option<(i64, i64)> {
        let t = totals.get(&(key.0.clone(), stratum(&key.1), slot))?;
        let (rn, rd) = (t.0 - c.num, t.1 - c.den);
        (rd >= cfg.k_min).then_some((rn, rd))
    };

    // Explored cells: every valid discovery cell of the whole package (Bonferroni family).
    let m = valid.len();
    struct Pass {
        key: Key,
        disc: Stage,
        p_adj: f64,
    }
    let mut passed: Vec<Pass> = vec![];
    let mut signals: Vec<Signal> = vec![];
    // Discovery statistics for every testable cell.
    let mut tested: Vec<(Key, Stage, bool)> = vec![];
    for (key, halves) in &valid {
        let Some(c) = &halves[0] else { continue };
        let Some((rn, rd)) = rest(key, 0, c) else {
            bump("no_baseline");
            continue;
        };
        if c.den < cfg.min_support {
            bump("below_min_support");
            continue;
        }
        let (disc, up) = stage(c, rn, rd);
        tested.push((key.clone(), disc, up));
    }
    // Adjusted p over ALL explored cells (m), computed on the tested ones (the rest count as p = 1).
    let mut order: Vec<usize> = (0..tested.len()).collect();
    order.sort_by(|&a, &b| tested[a].1.p.partial_cmp(&tested[b].1.p).unwrap_or(std::cmp::Ordering::Equal).then(tested[a].0.cmp(&tested[b].0)));
    let mut adj = vec![1.0f64; tested.len()];
    match cfg.multiplicity {
        Multiplicity::Bonferroni => {
            for &i in &order {
                adj[i] = (tested[i].1.p * m as f64).min(1.0);
            }
        }
        Multiplicity::Bh => {
            let mut running = 1.0f64;
            for (rank, &i) in order.iter().enumerate().rev() {
                running = running.min(tested[i].1.p * m as f64 / (rank + 1) as f64).min(1.0);
                adj[i] = running;
            }
        }
    }
    for (i, (key, disc, up)) in tested.into_iter().enumerate() {
        let p_adj = adj[i];
        let ratio = if disc.baseline_rate > 0.0 { disc.rate / disc.baseline_rate } else { f64::INFINITY };
        let big = disc.diff.abs() >= cfg.min_effect && (ratio >= cfg.min_ratio || ratio <= 1.0 / cfg.min_ratio);
        if !up {
            if p_adj < cfg.alpha && big {
                bump("favourable_direction");
            }
            continue;
        }
        if p_adj < cfg.alpha && big {
            passed.push(Pass { key, disc, p_adj });
        } else if disc.p < 0.05 && big {
            signals.push(Signal {
                metric: key.0,
                dims: key.1,
                status: "uncertain",
                reason: "not_significant_after_correction",
                direction: "up",
                discovery: Some(disc),
                holdout: None,
                p_adj: Some(p_adj),
                r2: None,
                depends_on: None,
            });
        }
    }

    // Replication on the holdout half; the holdout test is corrected over the number of candidates.
    let ncand = passed.len().max(1) as f64;
    let mut metrics_with_finding: BTreeMap<String, bool> = BTreeMap::new();
    for (key, _) in valid.keys().map(|k| (k, ())) {
        metrics_with_finding.entry(key.0.clone()).or_insert(false);
    }
    for pa in passed {
        metrics_with_finding.insert(pa.key.0.clone(), true);
        let halves = &valid[&pa.key];
        let hold = halves[1].as_ref().and_then(|h| rest(&pa.key, 1, h).map(|(rn, rd)| stage(h, rn, rd).0));
        let w = |i: usize| halves[i].as_ref().and_then(|c| rest(&pa.key, i, c).map(|(rn, rd)| stage(c, rn, rd).0));
        let (w1, w2) = (w(2), w(3));
        let r2 = match (&w1, &w2) {
            (Some(a), Some(b)) => {
                let ok = |x: &Stage| x.diff >= cfg.min_effect / 2.0 && x.p < 0.05;
                let status = if ok(a) && ok(b) {
                    "replicated"
                } else if a.diff <= 0.0 || b.diff <= 0.0 {
                    "reversed"
                } else {
                    "not_replicated"
                };
                R2 { status, w1: w1.clone(), w2: w2.clone() }
            }
            _ => R2 { status: "not_evaluated", w1: None, w2: None },
        };
        let (status, reason) = match &hold {
            None => ("candidate", "holdout_unavailable"),
            Some(h) if h.diff <= 0.0 => ("refuted", "holdout_direction_reversed"),
            Some(h) if h.p * ncand < 0.05 && h.diff >= cfg.min_effect / 2.0 => ("corroborated", "replicated_in_holdout"),
            Some(_) => ("uncertain", "holdout_not_significant"),
        };
        signals.push(Signal {
            metric: pa.key.0,
            dims: pa.key.1,
            status,
            reason,
            direction: "up",
            discovery: Some(pa.disc),
            holdout: hold,
            p_adj: Some(pa.p_adj),
            r2: Some(r2),
            depends_on: None,
        });
    }
    // A metric with no cell passing discovery is reported once as refuted / no differential.
    for (metric, found) in metrics_with_finding {
        if !found {
            signals.push(Signal {
                metric,
                dims: vec![],
                status: "refuted",
                reason: "no_differential",
                direction: "none",
                discovery: None,
                holdout: None,
                p_adj: None,
                r2: None,
                depends_on: None,
            });
        }
    }
    let findings: Vec<(String, Vec<(String, String)>)> = signals
        .iter()
        .filter(|s| !s.dims.is_empty() && matches!(s.status, "corroborated" | "candidate"))
        .map(|s| (s.metric.clone(), s.dims.clone()))
        .collect();
    for s in signals.iter_mut().filter(|s| !s.dims.is_empty()) {
        for (dep, parent) in &cfg.dependencies {
            if *dep == s.metric && findings.iter().any(|(m, d)| m == parent && d.iter().all(|x| s.dims.contains(x))) {
                s.depends_on = Some(parent.clone());
            }
        }
    }
    let rank = |s: &str| match s {
        "corroborated" => 0,
        "candidate" => 1,
        "uncertain" => 2,
        _ => 3,
    };
    signals.sort_by(|a, b| {
        rank(a.status)
            .cmp(&rank(b.status))
            .then(a.p_adj.unwrap_or(2.0).partial_cmp(&b.p_adj.unwrap_or(2.0)).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.metric.cmp(&b.metric))
            .then(a.dims.cmp(&b.dims))
    });
    Ok(Report { cells_explored: m, signals, discards: discards.into_iter().collect(), config: cfg.clone() })
}

fn stage_json(s: &Stage) -> Json {
    Json::obj(vec![
        ("numerator", Json::Int(s.numerator)),
        ("denominator", Json::Int(s.denominator)),
        ("rate", Json::Float(s.rate)),
        ("baseline_rate", Json::Float(s.baseline_rate)),
        ("diff", Json::Float(s.diff)),
        ("p", Json::Float(s.p)),
    ])
}

impl Report {
    pub fn to_json(&self) -> Json {
        let c = &self.config;
        let signals = self
            .signals
            .iter()
            .map(|s| {
                let mut kv = vec![
                    ("metric", Json::s(&s.metric)),
                    ("dims", Json::Obj(s.dims.iter().map(|(k, v)| (k.clone(), Json::s(v))).collect())),
                    ("status", Json::s(s.status)),
                    ("reason", Json::s(s.reason)),
                    ("direction", Json::s(s.direction)),
                    ("claim", Json::s("association")),
                ];
                if let Some(d) = &s.discovery {
                    kv.push(("discovery", stage_json(d)));
                }
                if let Some(h) = &s.holdout {
                    kv.push(("holdout", stage_json(h)));
                }
                if let Some(r) = &s.r2 {
                    let mut o = vec![("status", Json::s(r.status))];
                    if let Some(w) = &r.w1 {
                        o.push(("w1", stage_json(w)));
                    }
                    if let Some(w) = &r.w2 {
                        o.push(("w2", stage_json(w)));
                    }
                    kv.push(("r2", Json::obj(o)));
                }
                if let Some(d) = &s.depends_on {
                    kv.push(("depends_on", Json::s(d)));
                }
                if let Some(p) = s.p_adj {
                    kv.push(("p_adj", Json::Float(p)));
                }
                Json::obj(kv)
            })
            .collect();
        let discards = self
            .discards
            .iter()
            .map(|(k, n)| Json::obj(vec![("kind", Json::s(k)), ("count", Json::Int(*n))]))
            .collect();
        Json::obj(vec![
            ("semantics", Json::s(crate::SEMANTICS)),
            (
                "method",
                Json::obj(vec![
                    ("test", Json::s("two_proportion_z_pooled_vs_same_channel_excluding_own_reason")),
                    ("multiplicity", Json::s(match c.multiplicity { Multiplicity::Bh => "benjamini_hochberg_all_explored_cells", Multiplicity::Bonferroni => "bonferroni_all_explored_cells" })),
                    ("min_ratio", Json::Float(c.min_ratio)),
                    ("replication", Json::s("discovery_holdout_hash_split")),
                    ("secondary_replication", Json::s("r2_windows_2023-07..2024-12_vs_2025-01..2026-05")),
                    ("alpha", Json::Float(c.alpha)),
                    ("min_effect", Json::Float(c.min_effect)),
                    ("min_support", Json::Int(c.min_support)),
                    ("k_min", Json::Int(c.k_min)),
                ]),
            ),
            ("cells_explored", Json::Int(self.cells_explored as i64)),
            ("signals", Json::Arr(signals)),
            ("discards", Json::Arr(discards)),
        ])
    }
}

/// Step entry point: ndjson cell table in, report JSON out (default config).
pub fn run(input: &str) -> Result<String, StepError> {
    Ok(analyse(input, &Config::default())?.to_json().write())
}
