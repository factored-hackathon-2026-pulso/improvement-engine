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
//!
//! Second finding type, `level_risk` (W1-4): a metric whose VALUE is a risk against an explicit, pre-registered threshold
//! (`Config::level_risks`, e.g. M8: share of sends to non-consenting customers > 10%) rather than a vs-rest contrast. The
//! level is the pooled rate of the metric's valid cells; the test is one-sided (rate > threshold) with a Wilson 95% interval,
//! a minimum excess over the threshold, discovery/holdout replication, R2 windows and a per-month count. Level tests form a
//! separate pre-registered family: Bonferroni over `level_risks.len()` (fixed in advance, independent of which metrics are
//! present) and they do not enter `cells_explored` (the contrast BH family), so neither family dilutes the other. A level
//! signal is `class: risk`, `claim: association`, never a cause.

use std::collections::BTreeMap;

use crate::StepError;
use crate::sensor::json::{Json, parse};

/// Dimension keys a treated cell table may carry. Anything else (ids, free text) is rejected.
pub const ALLOWED_DIMS: [&str; 9] = [
    "reason_category", "channel", "category", "case_type", "priority", "survey_type",
    // AG2: digital action, campaign type, customer segment (closed vocabularies, aggregates only).
    "action", "campaign_type", "customer_segment",
];

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
    /// Minimum POOLED (discovery + holdout) cell denominator.
    pub min_support: i64,
    /// Pre-registered level-risk specs (the Bonferroni family of level tests).
    pub level_risks: Vec<LevelSpec>,
    /// `None` = the strict profile (default). `Some` = the exploratory profile: the strict tier runs unchanged and an
    /// additional, visibly tagged `candidate_exploratory` tier is produced with relaxed knobs (see [`Exploratory`]).
    pub exploratory: Option<Exploratory>,
}

/// Relaxed knobs of the exploratory tier. It adds VOLUME of candidates for downstream agents (Scout, Verifier, Builder,
/// regression proof); it never relaxes the privacy floor `k_min`, never emits `corroborated`, and stays an association.
/// Rationale and false-positive cost of each knob: docs/data/bank-cells-metrics.md ("Exploratory profile").
#[derive(Debug, Clone, PartialEq)]
pub struct Exploratory {
    /// BH q for the exploratory tier (strict: 0.01). Expected share of false discoveries among the exploratory tier <= q.
    pub alpha: f64,
    /// Absolute effect floor (strict: 0.05).
    pub min_effect: f64,
    /// Rate-ratio floor (strict: 1.25).
    pub min_ratio: f64,
    /// Minimum POOLED (discovery + holdout) support (strict: 500).
    pub min_support: i64,
}

impl Exploratory {
    pub fn standard() -> Self {
        Exploratory { alpha: 0.10, min_effect: 0.03, min_ratio: 1.15, min_support: 200 }
    }
}

impl Config {
    /// Strict tier unchanged plus the exploratory tier.
    pub fn exploratory() -> Self {
        Config { exploratory: Some(Exploratory::standard()), ..Config::default() }
    }
}

/// Note carried by every exploratory signal so the dossier and the Verifier see the weaker evidence.
pub const EXPLORATORY_NOTE: &str = "exploratory: weaker statistical evidence; the regression proof is the quality gate";

/// A metric whose level is a risk above `threshold` (fixed before looking at the data, set by the analyst, not by the sensor).
#[derive(Debug, Clone, PartialEq)]
pub struct LevelSpec {
    pub metric: String,
    /// The pooled rate must exceed this level.
    pub threshold: f64,
    /// Minimum pooled rate minus threshold in discovery; holdout needs half of it.
    pub min_excess: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            dependencies: [("M6", "M1"), ("M6R", "M1"), ("M6U", "M1"), ("M10", "M1")]
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            multiplicity: Multiplicity::Bh, k_min: 10, alpha: 0.01, min_ratio: 1.25, min_effect: 0.05, min_support: 500,
            // M8: more than 10% of sends to customers flagged as not accepting marketing is a compliance risk.
            level_risks: vec![LevelSpec { metric: "M8".to_string(), threshold: 0.10, min_excess: 0.05 }],
            exploratory: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub numerator: i64,
    pub denominator: i64,
    pub rate: f64,
    pub baseline_numerator: i64,
    pub baseline_denominator: i64,
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
    /// Ranking score (effect x support x replication evidence), set on every reported cell signal; see `priority`.
    pub priority: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct R2 {
    /// `replicated | not_replicated | reversed | not_evaluated`
    pub status: &'static str,
    pub w1: Option<Stage>,
    pub w2: Option<Stage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelStage {
    pub numerator: i64,
    pub denominator: i64,
    pub rate: f64,
    pub ci_low: f64,
    pub ci_high: f64,
    pub threshold: f64,
    /// rate minus threshold
    pub excess: f64,
    /// One-sided p of H0: rate <= threshold.
    pub p: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelR2 {
    /// `replicated | not_replicated | reversed | not_evaluated`
    pub status: &'static str,
    pub w1: Option<LevelStage>,
    pub w2: Option<LevelStage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelSignal {
    pub metric: String,
    pub status: &'static str,
    pub reason: &'static str,
    pub discovery: LevelStage,
    pub holdout: Option<LevelStage>,
    /// Bonferroni over the pre-registered level family.
    pub p_adj: f64,
    pub r2: LevelR2,
    /// Months (all halves pooled, support >= min_support) / months above the threshold.
    pub periods_total: i64,
    pub periods_above: i64,
    /// Valid discovery cells of the metric / cells whose own rate is above the threshold (descriptive, untested).
    pub cells_total: i64,
    pub cells_above: i64,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub cells_explored: usize,
    /// Explored cells per multiplicity family: `main` (every cell except reason-only) and `reason_only` (BH of its own).
    pub families: Vec<(&'static str, usize)>,
    /// Size of the pre-registered level-test family (`Config::level_risks.len()`).
    pub level_tests: usize,
    pub level_signals: Vec<LevelSignal>,
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

/// Wilson score interval (95%).
fn wilson(x: i64, n: i64) -> (f64, f64) {
    let (p, n) = (x as f64 / n as f64, n as f64);
    let z = 1.959964f64;
    let d = 1.0 + z * z / n;
    let c = (p + z * z / (2.0 * n)) / d;
    let h = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / d;
    ((c - h).max(0.0), (c + h).min(1.0))
}

fn level_stage(num: i64, den: i64, thr: f64) -> LevelStage {
    let rate = num as f64 / den as f64;
    let se = (thr * (1.0 - thr) / den as f64).sqrt();
    let p = if se > 0.0 { (0.5 * erfc((rate - thr) / se / std::f64::consts::SQRT_2)).clamp(0.0, 1.0) } else { 1.0 };
    let (lo, hi) = wilson(num, den);
    LevelStage { numerator: num, denominator: den, rate: r6(rate), ci_low: r6(lo), ci_high: r6(hi), threshold: thr, excess: r6(rate - thr), p }
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
    if matches!(p, "ALL" | "W1" | "W2") {
        return true;
    }
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
            _ => return invalid(format!("line {}: period must be YYYY-MM, ALL, W1 or W2", ln + 1)),
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
    (
        Stage {
            numerator: cell.num,
            denominator: cell.den,
            rate: r6(rate),
            baseline_numerator: rest_num,
            baseline_denominator: rest_den,
            baseline_rate: r6(base),
            diff: r6(diff),
            p,
        },
        diff > 0.0,
    )
}

/// Level-risk findings over the valid cells (see the module doc for the family and multiplicity rules).
fn level_risks(
    valid: &BTreeMap<Key, Slots>,
    months: &BTreeMap<(String, String), (i64, i64)>,
    cfg: &Config,
    bump: &mut dyn FnMut(&str),
) -> Vec<LevelSignal> {
    let family = cfg.level_risks.len().max(1) as f64;
    let mut out = vec![];
    for spec in &cfg.level_risks {
        let mut pooled = [(0i64, 0i64); 4];
        let (mut cells_total, mut cells_above) = (0i64, 0i64);
        let mut present = false;
        for ((metric, _), slots) in valid {
            if *metric != spec.metric {
                continue;
            }
            present = true;
            for (i, c) in slots.iter().enumerate() {
                if let Some(c) = c {
                    pooled[i].0 += c.num;
                    pooled[i].1 += c.den;
                }
            }
            if let Some(d) = &slots[0] {
                cells_total += 1;
                cells_above += (d.num as f64 / d.den as f64 > spec.threshold) as i64;
            }
        }
        if !present {
            continue;
        }
        if pooled[0].1 < cfg.min_support {
            bump("level_below_min_support");
            continue;
        }
        let thr = spec.threshold;
        let disc = level_stage(pooled[0].0, pooled[0].1, thr);
        let p_adj = (disc.p * family).min(1.0);
        let hold = (pooled[1].1 >= cfg.k_min).then(|| level_stage(pooled[1].0, pooled[1].1, thr));
        let w = |i: usize| (pooled[i].1 >= cfg.min_support).then(|| level_stage(pooled[i].0, pooled[i].1, thr));
        let (w1, w2) = (w(2), w(3));
        let r2 = match (&w1, &w2) {
            (Some(a), Some(b)) => {
                let ok = |x: &LevelStage| x.ci_low > thr;
                let status = if ok(a) && ok(b) {
                    "replicated"
                } else if a.excess <= 0.0 || b.excess <= 0.0 {
                    "reversed"
                } else {
                    "not_replicated"
                };
                LevelR2 { status, w1: w1.clone(), w2: w2.clone() }
            }
            _ => LevelR2 { status: "not_evaluated", w1: None, w2: None },
        };
        let (status, reason) = if disc.excess <= 0.0 || p_adj >= cfg.alpha {
            ("refuted", "level_not_above_threshold")
        } else if disc.excess < spec.min_excess {
            ("refuted", "excess_below_floor")
        } else {
            match &hold {
                None => ("candidate", "holdout_unavailable"),
                Some(h) if h.excess <= 0.0 => ("refuted", "holdout_level_not_above_threshold"),
                Some(h) if h.p * family < 0.05 && h.excess >= spec.min_excess / 2.0 => ("corroborated", "replicated_in_holdout"),
                Some(_) => ("uncertain", "holdout_not_significant"),
            }
        };
        let (mut total, mut above) = (0i64, 0i64);
        for ((metric, _), (n, d)) in months {
            if *metric == spec.metric && *d >= cfg.min_support {
                total += 1;
                above += (*n as f64 / *d as f64 > thr) as i64;
            }
        }
        out.push(LevelSignal {
            metric: spec.metric.clone(),
            status,
            reason,
            discovery: disc,
            holdout: hold,
            p_adj,
            r2,
            periods_total: total,
            periods_above: above,
            cells_total,
            cells_above,
        });
    }
    out
}

/// Multiplicity family of a cell: `reason_only` (a sole `reason_category` dimension) has its own BH; everything else is `main`.
const FAMILIES: [&str; 2] = ["main", "reason_only"];

fn family_of(dims: &[(String, String)]) -> usize {
    usize::from(dims.len() == 1 && dims[0].0 == "reason_category")
}

/// Adjusted p-values for the tests of ONE family; `m` is the number of explored cells of the family (untested ones count as p = 1).
fn adjust_p(ps: &[f64], keys: &[&Key], m: usize, mult: Multiplicity) -> Vec<f64> {
    let mut order: Vec<usize> = (0..ps.len()).collect();
    order.sort_by(|&a, &b| ps[a].partial_cmp(&ps[b]).unwrap_or(std::cmp::Ordering::Equal).then(keys[a].cmp(keys[b])));
    let mut adj = vec![1.0f64; ps.len()];
    match mult {
        Multiplicity::Bonferroni => {
            for &i in &order {
                adj[i] = (ps[i] * m as f64).min(1.0);
            }
        }
        Multiplicity::Bh => {
            let mut running = 1.0f64;
            for (rank, &i) in order.iter().enumerate().rev() {
                running = running.min(ps[i] * m as f64 / (rank + 1) as f64).min(1.0);
                adj[i] = running;
            }
        }
    }
    adj
}

/// Ranking score: effect x support x replication evidence. `evidence` = 0.4 (discovery) + 0.3 (holdout same direction)
/// + 0.2 (holdout significant: strict corroboration) + 0.1 (R2 windows replicated). Ranking only; never a test.
fn priority(diff: f64, pooled: i64, hold_up: bool, hold_sig: bool, r2_ok: bool) -> f64 {
    let evidence = 0.4 + 0.3 * f64::from(u8::from(hold_up)) + 0.2 * f64::from(u8::from(hold_sig)) + 0.1 * f64::from(u8::from(r2_ok));
    r6(diff * (1.0 + pooled as f64).ln() * evidence)
}

struct Tested {
    key: Key,
    disc: Stage,
    up: bool,
    pooled: i64,
    fam: usize,
    strict_ok: bool,
}

const PERIOD_ALL: &str = "ALL";

pub fn analyse(input: &str, cfg: &Config) -> Result<Report, StepError> {
    let rows = parse_rows(input)?;
    let mut discards: BTreeMap<String, i64> = BTreeMap::new();
    let mut bump = |k: &str| *discards.entry(k.to_string()).or_insert(0) += 1;

    // Metrics with FULL-PERIOD rows (`ALL`, per half) take their half totals from those rows; month rows then only feed the
    // per-month counts. Metrics with window rows (`W1`/`W2`) take R2 from them. Tables without those rows (older exports,
    // level-risk metrics) keep the original month-sum behaviour.
    let full_metrics: std::collections::BTreeSet<String> =
        rows.iter().filter(|r| r.period.as_deref() == Some(PERIOD_ALL)).map(|r| r.key.0.clone()).collect();
    let win_metrics: std::collections::BTreeSet<String> =
        rows.iter().filter(|r| matches!(r.period.as_deref(), Some("W1" | "W2"))).map(|r| r.key.0.clone()).collect();

    // k rule: a violating row is a named discard and is dropped everywhere (never tested, not in baselines).
    let mut valid: BTreeMap<Key, Slots> = BTreeMap::new();
    let add = |slot: &mut Option<Cell>, c: &Cell| match slot {
        Some(x) => {
            x.num += c.num;
            x.den += c.den;
        }
        None => *slot = Some(Cell { num: c.num, den: c.den }),
    };
    let mut months: BTreeMap<(String, String), (i64, i64)> = BTreeMap::new();
    for r in rows {
        if !k_ok(&r.cell, cfg.k_min) {
            bump("k_violation");
            continue;
        }
        let metric = r.key.0.clone();
        let per = r.period.clone();
        let month = per.as_deref().filter(|p| !matches!(*p, PERIOD_ALL | "W1" | "W2"));
        if let Some(p) = month {
            if cfg.level_risks.iter().any(|l| l.metric == metric) {
                let t = months.entry((metric.clone(), p.to_string())).or_insert((0, 0));
                t.0 += r.cell.num;
                t.1 += r.cell.den;
            }
        }
        let slots = valid.entry(r.key).or_insert([None, None, None, None]);
        match per.as_deref() {
            Some(PERIOD_ALL) => add(&mut slots[r.half], &r.cell),
            Some("W1") => add(&mut slots[2], &r.cell),
            Some("W2") => add(&mut slots[3], &r.cell),
            Some(p) => {
                if !full_metrics.contains(&metric) {
                    add(&mut slots[r.half], &r.cell);
                }
                if !win_metrics.contains(&metric) {
                    if let Some(w) = window_of(p) {
                        add(&mut slots[2 + w], &r.cell);
                    }
                }
            }
            None => add(&mut slots[r.half], &r.cell),
        }
    }
    let before = valid.len();
    valid.retain(|_, s| s[0].is_some());
    for _ in 0..(before - valid.len()) {
        bump("holdout_without_discovery");
    }

    let level_signals = level_risks(&valid, &months, cfg, &mut bump);

    // Baseline = the same-channel stratum total (all PUBLISHED full-period cells of the metric/stratum in this slot) minus the
    // cell itself. Cells suppressed by k are absent from both sides; the aggregator publishes the full-period cells so the
    // share of the stratum that survives is high (see docs/data/bank-cells-metrics.md).
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
        k_ok(&Cell { num: rn, den: rd }, cfg.k_min).then_some((rn, rd))
    };

    // Explored cells: every valid discovery cell of the whole package; BH is applied per family (`main`, `reason_only`).
    let mut m_fam = [0usize; 2];
    for key in valid.keys() {
        m_fam[family_of(&key.1)] += 1;
    }
    let m = valid.len();
    let expl = cfg.exploratory.as_ref();
    let mut tested: Vec<Tested> = vec![];
    for (key, halves) in &valid {
        let Some(c) = &halves[0] else { continue };
        let Some((rn, rd)) = rest(key, 0, c) else {
            bump("no_baseline");
            continue;
        };
        // Support floor on the POOLED discovery + holdout support (the full period), not on the discovery half alone.
        let pooled = c.den + halves[1].as_ref().map_or(0, |h| h.den);
        let strict_ok = pooled >= cfg.min_support;
        if !strict_ok && !expl.is_some_and(|e| pooled >= e.min_support) {
            bump("below_min_support");
            continue;
        }
        let (disc, up) = stage(c, rn, rd);
        tested.push(Tested { key: key.clone(), disc, up, pooled, fam: family_of(&key.1), strict_ok });
    }
    // Adjusted p per family: strict tier over the strict-eligible tests only (the exploratory tier never changes it),
    // exploratory tier over every eligible test.
    let mut adj_strict = vec![1.0f64; tested.len()];
    let mut adj_expl = vec![1.0f64; tested.len()];
    for f in 0..2 {
        for (strict_only, out) in [(true, &mut adj_strict), (false, &mut adj_expl)] {
            let idx: Vec<usize> = (0..tested.len()).filter(|&i| tested[i].fam == f && (!strict_only || tested[i].strict_ok)).collect();
            let ps: Vec<f64> = idx.iter().map(|&i| tested[i].disc.p).collect();
            let ks: Vec<&Key> = idx.iter().map(|&i| &tested[i].key).collect();
            for (j, a) in adjust_p(&ps, &ks, m_fam[f], cfg.multiplicity).into_iter().enumerate() {
                out[idx[j]] = a;
            }
        }
    }

    let ratio_of = |d: &Stage| if d.baseline_rate > 0.0 { d.rate / d.baseline_rate } else { f64::INFINITY };
    let mut passed: Vec<usize> = vec![];
    let mut signals: Vec<Signal> = vec![];
    let mut exploratory_idx: Vec<usize> = vec![];
    for (i, t) in tested.iter().enumerate() {
        let ratio = ratio_of(&t.disc);
        let big = t.disc.diff.abs() >= cfg.min_effect && (ratio >= cfg.min_ratio || ratio <= 1.0 / cfg.min_ratio);
        if !t.up {
            if t.strict_ok && adj_strict[i] < cfg.alpha && big {
                bump("favourable_direction");
            }
            continue;
        }
        if t.strict_ok && adj_strict[i] < cfg.alpha && big {
            passed.push(i);
            continue;
        }
        let expl_pass = expl.is_some_and(|e| adj_expl[i] < e.alpha && t.disc.diff >= e.min_effect && ratio >= e.min_ratio);
        if expl_pass {
            exploratory_idx.push(i);
        } else if t.strict_ok && t.disc.p < 0.05 && big {
            signals.push(Signal {
                metric: t.key.0.clone(),
                dims: t.key.1.clone(),
                status: "uncertain",
                reason: "not_significant_after_correction",
                direction: "up",
                discovery: Some(t.disc.clone()),
                holdout: None,
                p_adj: Some(adj_strict[i]),
                r2: None,
                depends_on: None,
                priority: None,
            });
        }
    }

    let r2_of = |key: &Key, halves: &Slots| -> R2 {
        let w = |i: usize| halves[i].as_ref().and_then(|c| rest(key, i, c).map(|(rn, rd)| stage(c, rn, rd).0));
        let (w1, w2) = (w(2), w(3));
        match (&w1, &w2) {
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
        }
    };
    let hold_of = |key: &Key, halves: &Slots| halves[1].as_ref().and_then(|h| rest(key, 1, h).map(|(rn, rd)| stage(h, rn, rd).0));

    // Replication on the holdout half; the holdout test is corrected over the number of strict candidates.
    let ncand = passed.len().max(1) as f64;
    let mut metrics_with_finding: BTreeMap<String, bool> = BTreeMap::new();
    for key in valid.keys() {
        metrics_with_finding.entry(key.0.clone()).or_insert(false);
    }
    for &i in &passed {
        let t = &tested[i];
        metrics_with_finding.insert(t.key.0.clone(), true);
        let halves = &valid[&t.key];
        let hold = hold_of(&t.key, halves);
        let r2 = r2_of(&t.key, halves);
        let (mut status, mut reason) = match &hold {
            None => ("candidate", "holdout_unavailable"),
            Some(h) if h.diff <= 0.0 => ("refuted", "holdout_direction_reversed"),
            Some(h) if h.denominator < cfg.min_support / 2 => ("uncertain", "replication_underpowered"),
            Some(h) if h.p * ncand < 0.05 && h.diff >= cfg.min_effect / 2.0 => ("corroborated", "replicated_in_holdout"),
            Some(_) => ("uncertain", "holdout_not_significant"),
        };
        if expl.is_some() && status == "uncertain" {
            // The strict discovery passed; weaker replication is a LABEL in the exploratory profile, not a gate.
            reason = if reason == "replication_underpowered" { "exploratory_replication_underpowered" } else { "exploratory_holdout_weak" };
            status = "candidate_exploratory";
        }
        let pr = (status != "refuted").then(|| {
            priority(t.disc.diff, t.pooled, hold.as_ref().is_some_and(|h| h.diff > 0.0), status == "corroborated", r2.status == "replicated")
        });
        signals.push(Signal {
            metric: t.key.0.clone(),
            dims: t.key.1.clone(),
            status,
            reason,
            direction: "up",
            discovery: Some(t.disc.clone()),
            holdout: hold,
            p_adj: Some(adj_strict[i]),
            r2: Some(r2),
            depends_on: None,
            priority: pr,
        });
    }
    // Exploratory tier: discovery passes the relaxed knobs, replication is a label.
    if let Some(e) = expl {
        for &i in &exploratory_idx {
            let t = &tested[i];
            let halves = &valid[&t.key];
            let hold = hold_of(&t.key, halves);
            let r2 = r2_of(&t.key, halves);
            let reason = match &hold {
                None => "exploratory_discovery_only",
                Some(h) if h.diff <= 0.0 => {
                    bump("exploratory_holdout_reversed");
                    continue;
                }
                Some(h) if h.denominator < e.min_support / 2 => "exploratory_replication_underpowered",
                Some(h) if h.p < 0.05 && h.diff >= e.min_effect / 2.0 => "exploratory_holdout_replicated",
                Some(_) => "exploratory_holdout_weak",
            };
            metrics_with_finding.insert(t.key.0.clone(), true);
            let pr = priority(t.disc.diff, t.pooled, hold.as_ref().is_some_and(|h| h.diff > 0.0), false, r2.status == "replicated");
            signals.push(Signal {
                metric: t.key.0.clone(),
                dims: t.key.1.clone(),
                status: "candidate_exploratory",
                reason,
                direction: "up",
                discovery: Some(t.disc.clone()),
                holdout: hold,
                p_adj: Some(adj_expl[i]),
                r2: Some(r2),
                depends_on: None,
                priority: Some(pr),
            });
        }
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
                priority: None,
            });
        }
    }
    // Dependency flag from the metric dependency table, NOT from the presence of a parent finding: a dependent metric
    // (CSAT low, handle-time share) is a re-expression of its parent, so any cell of it is flagged even when the parent
    // has no finding on the same cell.
    for s in signals.iter_mut().filter(|s| !s.dims.is_empty()) {
        if let Some((_, parent)) = cfg.dependencies.iter().find(|(dep, _)| *dep == s.metric) {
            s.depends_on = Some(parent.clone());
        }
    }
    let rank = |s: &str| match s {
        "corroborated" => 0,
        "candidate" => 1,
        "candidate_exploratory" => 2,
        "uncertain" => 3,
        _ => 4,
    };
    signals.sort_by(|a, b| {
        let ord = rank(a.status).cmp(&rank(b.status));
        let ord = if ord == std::cmp::Ordering::Equal && a.status == "candidate_exploratory" {
            // best first: the reasoning cap takes the highest priority
            b.priority.unwrap_or(0.0).partial_cmp(&a.priority.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            ord.then(a.p_adj.unwrap_or(2.0).partial_cmp(&b.p_adj.unwrap_or(2.0)).unwrap_or(std::cmp::Ordering::Equal))
        };
        ord.then(a.metric.cmp(&b.metric)).then(a.dims.cmp(&b.dims))
    });
    Ok(Report {
        cells_explored: m,
        families: FAMILIES.iter().copied().zip(m_fam).collect(),
        level_tests: cfg.level_risks.len(),
        level_signals,
        signals,
        discards: discards.into_iter().collect(),
        config: cfg.clone(),
    })
}

fn level_stage_json(s: &LevelStage) -> Json {
    Json::obj(vec![
        ("numerator", Json::Int(s.numerator)),
        ("denominator", Json::Int(s.denominator)),
        ("rate", Json::Float(s.rate)),
        ("baseline_rate", Json::Float(s.threshold)),
        ("diff", Json::Float(s.excess)),
        ("ci95_low", Json::Float(s.ci_low)),
        ("ci95_high", Json::Float(s.ci_high)),
        ("p", Json::Float(s.p)),
    ])
}

fn level_json(s: &LevelSignal) -> Json {
    let mut r2 = vec![("status", Json::s(s.r2.status))];
    if let Some(w) = &s.r2.w1 {
        r2.push(("w1", level_stage_json(w)));
    }
    if let Some(w) = &s.r2.w2 {
        r2.push(("w2", level_stage_json(w)));
    }
    let mut kv = vec![
        ("metric", Json::s(&s.metric)),
        ("type", Json::s("level_risk")),
        ("class", Json::s("risk")),
        ("dims", Json::Obj(vec![])),
        ("status", Json::s(s.status)),
        ("reason", Json::s(s.reason)),
        ("direction", Json::s("up")),
        ("claim", Json::s("association")),
        ("discovery", level_stage_json(&s.discovery)),
    ];
    if let Some(h) = &s.holdout {
        kv.push(("holdout", level_stage_json(h)));
    }
    kv.push(("r2", Json::obj(r2)));
    kv.push(("periods", Json::obj(vec![("total", Json::Int(s.periods_total)), ("above_threshold", Json::Int(s.periods_above))])));
    kv.push(("level_cells", Json::obj(vec![("total", Json::Int(s.cells_total)), ("above_threshold", Json::Int(s.cells_above))])));
    kv.push(("p_adj", Json::Float(s.p_adj)));
    Json::obj(kv)
}

fn stage_json(s: &Stage) -> Json {
    Json::obj(vec![
        ("numerator", Json::Int(s.numerator)),
        ("denominator", Json::Int(s.denominator)),
        ("rate", Json::Float(s.rate)),
        ("baseline_numerator", Json::Int(s.baseline_numerator)),
        ("baseline_denominator", Json::Int(s.baseline_denominator)),
        ("baseline_rate", Json::Float(s.baseline_rate)),
        ("diff", Json::Float(s.diff)),
        ("p", Json::Float(s.p)),
    ])
}

impl Report {
    pub fn to_json(&self) -> Json {
        let c = &self.config;
        let signals: Vec<Json> = self
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
                if let Some(p) = s.priority {
                    kv.push(("priority", Json::Float(p)));
                }
                if s.status == "candidate_exploratory" {
                    kv.push(("exploratory_note", Json::s(EXPLORATORY_NOTE)));
                }
                Json::obj(kv)
            })
            .chain(self.level_signals.iter().map(level_json))
            .collect();
        let discards = self
            .discards
            .iter()
            .map(|(k, n)| {
                // A discard count is itself a published count: below k_min it is suppressed by the sensor.
                if *n < c.k_min {
                    Json::obj(vec![("kind", Json::s(k)), ("count", Json::Null), ("suppressed", Json::Bool(true))])
                } else {
                    Json::obj(vec![("kind", Json::s(k)), ("count", Json::Int(*n))])
                }
            })
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
                    ("profile", Json::s(if c.exploratory.is_some() { "exploratory" } else { "strict" })),
                    ("support_basis", Json::s("pooled_discovery_plus_holdout")),
                    ("baseline", Json::s("same_channel_full_period_published_cells_minus_own_cell")),
                    ("families", Json::Obj(self.families.iter().map(|(f, n)| (f.to_string(), Json::Int(*n as i64))).collect())),
                    (
                        "exploratory",
                        match &c.exploratory {
                            None => Json::Null,
                            Some(e) => Json::obj(vec![
                                ("alpha", Json::Float(e.alpha)),
                                ("min_effect", Json::Float(e.min_effect)),
                                ("min_ratio", Json::Float(e.min_ratio)),
                                ("min_support", Json::Int(e.min_support)),
                                ("explored", Json::Int(self.cells_explored as i64)),
                                ("never_corroborated", Json::Bool(true)),
                            ]),
                        },
                    ),
                    (
                        "level_risk",
                        Json::obj(vec![
                            ("test", Json::s("one_sided_z_vs_pre_registered_threshold_wilson_95_interval")),
                            ("multiplicity", Json::s("bonferroni_over_pre_registered_level_family_separate_from_cells_explored")),
                            ("specs", Json::Arr(c.level_risks.iter().map(|l| Json::obj(vec![("metric", Json::s(&l.metric)), ("threshold", Json::Float(l.threshold)), ("min_excess", Json::Float(l.min_excess))])).collect())),
                        ]),
                    ),
                ]),
            ),
            ("cells_explored", Json::Int(self.cells_explored as i64)),
            ("level_tests", Json::Int(self.level_tests as i64)),
            ("signals", Json::Arr(signals)),
            ("discards", Json::Arr(discards)),
        ])
    }
}

/// Step entry point: ndjson cell table in, report JSON out (default config).
pub fn run(input: &str) -> Result<String, StepError> {
    Ok(analyse(input, &Config::default())?.to_json().write())
}

/// Same, with the exploratory profile (strict tier unchanged plus the tagged `candidate_exploratory` tier).
pub fn run_exploratory(input: &str) -> Result<String, StepError> {
    Ok(analyse(input, &Config::exploratory())?.to_json().write())
}
