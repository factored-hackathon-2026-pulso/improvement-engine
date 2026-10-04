//! Independent review (CL-review) of the rust-events sensor: adversarial null structure, privacy of counts, mutation guards.
mod ev_common;
use ev_common::Rng;
use steps::events_sensor::{Params, analyze};

fn p() -> Params {
    Params { min_history_days: 0, min_history_cases: 200, ..Params::default() }
}

#[derive(Clone, Copy)]
struct Cfg {
    cells: usize,
    /// share of the first cell (rest split evenly)
    big: f64,
    n: usize,
    days: usize,
    heavy: bool,
    /// sd of a per-(cell,day) multiplicative random effect on rates and delays (bursty days)
    burst: f64,
    /// reassignment rate planted in cell 0: (discovery, holdout) instead of the 0.08 base
    plant: Option<(f64, f64)>,
}

fn mk(c: Cfg, seed: u64) -> (String, String) {
    let mut r = Rng::new(seed);
    let norm = |r: &mut Rng| ((-2.0 * (1.0 - r.f()).ln()).sqrt()) * (6.283_185_307 * r.f()).cos();
    let langs = ["aa", "bb", "cc", "dd", "ee", "ff"];
    let chans = ["c1", "c2", "c3", "c4"];
    let cell_of = |i: usize| (langs[i % 6], chans[(i / 6) % 4]);
    let eff: Vec<f64> = (0..c.cells * c.days).map(|_| (norm(&mut r) * c.burst).exp()).collect();
    let (mut ev, mut cs) = (String::new(), String::new());
    let mut seq = 0;
    let span = (c.days as f64) * 86_400.0;
    let mut times: Vec<f64> = (0..c.n).map(|_| r.f() * span).collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut rows: Vec<(f64, &str, String)> = vec![];
    for (i, t0) in times.iter().enumerate() {
        let u = r.f();
        let cell = if u < c.big { 0 } else { 1 + ((u - c.big) / (1.0 - c.big) * (c.cells - 1) as f64) as usize % (c.cells - 1).max(1) };
        let day = ((t0 / 86_400.0) as usize).min(c.days - 1);
        let e = eff[cell * c.days + day];
        let (l, ch) = cell_of(cell);
        let prev = if r.f() < (0.08 * e).min(0.9) { "\"CAS-prev\"" } else { "null" };
        cs.push_str(&format!("{{\"case_id\":\"CAS-{i}\",\"channel\":\"{ch}\",\"language\":\"{l}\",\"previous_case_id\":{prev}}}\n"));
        let d = |r: &mut Rng, m: f64| if c.heavy { m * e * (norm(r) * 1.3).exp() / 2.3 } else { m * e * r.exp(1.0) };
        let mut t = *t0;
        rows.push((t, "case.opened", format!("CAS-{i}")));
        rows.push((t + 1.0, "case.assigned", format!("CAS-{i}")));
        t += d(&mut r, 60.0) + 2.0;
        rows.push((t, "case.first_responded", format!("CAS-{i}")));
        let pr = match (cell, c.plant) { (0, Some((a, b))) => if (i as f64) < 0.6 * c.n as f64 { a } else { b }, _ => 0.08 };
        if r.f() < (pr * e).min(0.9) {
            rows.push((t + 1.0, "case.assigned", format!("CAS-{i}")));
        }
        t += d(&mut r, 600.0) + 2.0;
        rows.push((t, "case.closed", format!("CAS-{i}")));
    }
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    for (t, ty, case) in rows {
        seq += 1;
        ev.push_str(&format!(
            "{{\"sequence\":{seq},\"event_id\":\"E{seq}\",\"event_type\":\"{ty}\",\"case_id\":\"{case}\",\"event_time\":\"{}\"}}\n",
            iso(1_788_249_600 + t as i64)
        ));
    }
    (ev, cs)
}

fn iso(t: i64) -> String {
    let days = t.div_euclid(86_400);
    let s = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

fn fpr(c: Cfg, seeds: u64) -> (u64, u64) {
    let (mut adm, mut cand) = (0, 0);
    for s in 1..=seeds {
        let (e, k) = mk(c, 777_000 + s);
        let r = analyze(&e, &k, &p());
        adm += u64::from(!r.signals.is_empty());
        cand += u64::from(r.discards.iter().any(|d| d.reason == "not_replicated"));
    }
    (adm, cand)
}

#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn measure_adversarial_null_fpr() {
    let base = Cfg { cells: 8, big: 0.4, n: 3000, days: 30, heavy: false, burst: 0.0, plant: None };
    let cfgs = [
        ("balanced 8 cells exp", base),
        ("unbalanced one huge 0.9, 12 cells", Cfg { cells: 12, big: 0.9, ..base }),
        ("24 cells heavy-tail lognormal", Cfg { cells: 24, big: 0.3, heavy: true, ..base }),
        ("24 cells heavy-tail n=8000", Cfg { cells: 24, big: 0.3, heavy: true, n: 8000, ..base }),
        ("12 cells bursty days sd 0.3, 30d", Cfg { cells: 12, burst: 0.3, ..base }),
        ("12 cells bursty days sd 0.5, 10d", Cfg { cells: 12, burst: 0.5, days: 10, ..base }),
        ("12 cells bursty heavy sd 0.3", Cfg { cells: 12, burst: 0.3, heavy: true, ..base }),
    ];
    for (name, c) in cfgs {
        let (a, b) = fpr(c, 60);
        eprintln!("REVIEW-FPR {name}: admitted {a}/60, runs with not_replicated candidate {b}/60");
    }
}

fn planted(n: usize, hold: f64) -> steps::events_sensor::Report {
    let c = Cfg { cells: 4, big: 0.4, n, days: 30, heavy: false, burst: 0.0, plant: Some((0.20, hold)) };
    let (e, k) = mk(c, 4242);
    analyze(&e, &k, &p())
}

#[test]
fn positive_control_effect_that_persists_is_admitted() {
    let r = planted(4000, 0.20);
    assert!(r.signals.iter().any(|s| s.family == "reassignment_rate" && s.cell == "aa/c1"), "{r:?}");
}

// Mutation guards. Each isolates ONE gate: the holdout effect keeps the right direction and size, so only the holdout
// p-value (replication at alpha/K) can reject; or it is significant but too small, so only the effect/direction gate can.
#[test]
fn replication_p_value_alone_rejects_a_holdout_that_is_not_significant() {
    let r = planted(1500, 0.12);
    assert!(r.signals.is_empty(), "holdout diff ~0.04 is not significant at alpha/K: {r:?}");
    assert!(r.discards.iter().any(|d| d.reason == "not_replicated" && d.cell == "aa/c1"), "{:?}", r.discards);
}

#[test]
fn replication_direction_effect_alone_rejects_a_significant_but_collapsed_holdout() {
    let r = planted(20_000, 0.10);
    assert!(r.signals.is_empty(), "holdout effect shrank below half the minimum: {r:?}");
    assert!(r.discards.iter().any(|d| d.reason == "not_replicated" && d.cell == "aa/c1"), "{:?}", r.discards);
}

#[test]
fn counts_in_discards_never_disclose_cells_below_k() {
    // support fails in the holdout: the detail must not state the (small) positive count
    let c = Cfg { cells: 4, big: 0.4, n: 4000, days: 30, heavy: false, burst: 0.0, plant: Some((0.20, 0.20)) };
    let (e, k) = mk(c, 4242);
    let r = analyze(&e, &k, &Params { min_support: 1_000_000, ..p() });
    let ds: Vec<_> = r.discards.iter().filter(|d| d.reason == "low_support").collect();
    assert!(!ds.is_empty());
    for d in ds {
        assert!(!d.detail.contains("positives"), "{}", d.detail);
    }
}

#[test]
fn rereading_a_package_after_a_crash_does_not_double_count() {
    let c = Cfg { cells: 4, big: 0.4, n: 4000, days: 30, heavy: false, burst: 0.0, plant: Some((0.20, 0.20)) };
    let (e, k) = mk(c, 4242);
    let once = analyze(&e, &k, &p());
    let twice = analyze(&format!("{e}{e}"), &format!("{k}{k}"), &p());
    assert_eq!(once.steps_output("r", "treated"), twice.steps_output("r", "treated"));
    assert_eq!((once.events, once.cases), (twice.events, twice.cases));
}
