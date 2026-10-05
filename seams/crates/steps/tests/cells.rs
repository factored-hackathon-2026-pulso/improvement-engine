//! L1 `cells` sensor: deterministic recurring-problem signals over TREATED cell tables.
//! Test-first. Tables here are synthetic aggregates (no ids, no free text).
use steps::cells::{Config, Multiplicity, analyse, run, run_exploratory};
use steps::sensor::json::{Json, parse};

fn row(metric: &str, reason: &str, channel: &str, half: &str, num: i64, den: i64) -> String {
    format!(
        "{{\"metric\":\"{metric}\",\"dims\":{{\"reason_category\":\"{reason}\",\"channel\":\"{channel}\"}},\"half\":\"{half}\",\"numerator\":{num},\"denominator\":{den}}}"
    )
}

const REASONS: [&str; 5] = ["Queja", "Comercial", "Transaccional", "Tecnico", "Informativo"];
const CHANNELS: [&str; 2] = ["Phone", "Chat"];

/// 5x2 grid, baseline rate 20%; `planted(reason, channel, half)` may override the rate (permille).
fn grid(metric: &str, planted: &dyn Fn(&str, &str, &str) -> Option<i64>) -> Vec<String> {
    let mut rows = vec![];
    for r in REASONS {
        for c in CHANNELS {
            for (half, den) in [("discovery", 600i64), ("holdout", 400i64)] {
                let permille = planted(r, c, half).unwrap_or(200);
                rows.push(row(metric, r, c, half, den * permille / 1000, den));
            }
        }
    }
    rows
}

fn out(rows: &[String]) -> Json {
    parse(&run(&rows.join("\n")).expect("run")).expect("json")
}

fn signals(j: &Json) -> Vec<&Json> {
    j.get("signals").and_then(|s| s.as_arr()).unwrap().iter().collect()
}

fn find<'a>(j: &'a Json, reason: &str, channel: &str) -> Option<&'a Json> {
    signals(j).into_iter().find(|s| {
        let Some(d) = s.get("dims") else { return false };
        d.get("reason_category").and_then(|v| v.as_str()) == Some(reason)
            && d.get("channel").and_then(|v| v.as_str()) == Some(channel)
    })
}

fn status<'a>(j: &'a Json, reason: &str, channel: &str) -> Option<&'a str> {
    find(j, reason, channel).and_then(|s| s.get("status")).and_then(|v| v.as_str())
}

fn discard_count(j: &Json, kind: &str) -> i64 {
    j.get("discards")
        .and_then(|d| d.as_arr())
        .unwrap()
        .iter()
        .filter(|d| d.get("kind").and_then(|k| k.as_str()) == Some(kind))
        .map(|d| d.get("count").and_then(|c| c.as_i64()).unwrap())
        .sum()
}

fn planted_queja_phone(r: &str, c: &str, _h: &str) -> Option<i64> {
    (r == "Queja" && c == "Phone").then_some(300)
}

#[test]
fn planted_effect_is_corroborated_and_nothing_else() {
    let j = out(&grid("M1", &planted_queja_phone));
    assert_eq!(status(&j, "Queja", "Phone"), Some("corroborated"));
    let corroborated = signals(&j)
        .into_iter()
        .filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("corroborated"))
        .count();
    assert_eq!(corroborated, 1);
    assert_eq!(j.get("cells_explored").and_then(|v| v.as_i64()), Some(10));
}

#[test]
fn permuted_evidence_moves_the_finding_same_code() {
    // Same table with the Queja and Comercial labels exchanged: the finding follows the evidence.
    let swapped = |r: &str, c: &str, h: &str| {
        let r2 = match r {
            "Queja" => "Comercial",
            "Comercial" => "Queja",
            x => x,
        };
        planted_queja_phone(r2, c, h)
    };
    let j = out(&grid("M1", &swapped));
    assert_eq!(status(&j, "Comercial", "Phone"), Some("corroborated"));
    assert_ne!(status(&j, "Queja", "Phone"), Some("corroborated"));
    assert_ne!(status(&j, "Queja", "Phone"), Some("candidate"));
}

#[test]
fn evidence_permuted_across_halves_refutes_the_discovery() {
    // Effect present only in discovery for Queja/Phone; the holdout shows the opposite direction.
    let f = |r: &str, c: &str, h: &str| match (r, c, h) {
        ("Queja", "Phone", "discovery") => Some(300),
        ("Queja", "Phone", "holdout") => Some(150),
        _ => None,
    };
    let j = out(&grid("M1", &f));
    assert_eq!(status(&j, "Queja", "Phone"), Some("refuted"));
}

#[test]
fn same_direction_but_weak_holdout_is_uncertain() {
    let f = |r: &str, c: &str, h: &str| match (r, c, h) {
        ("Queja", "Phone", "discovery") => Some(300),
        ("Queja", "Phone", "holdout") => Some(215),
        _ => None,
    };
    let j = out(&grid("M1", &f));
    assert_eq!(status(&j, "Queja", "Phone"), Some("uncertain"));
}

#[test]
fn missing_holdout_is_candidate() {
    let rows: Vec<String> =
        grid("M1", &planted_queja_phone).into_iter().filter(|r| !r.contains("\"holdout\"")).collect();
    let j = out(&rows);
    assert_eq!(status(&j, "Queja", "Phone"), Some("candidate"));
}

#[test]
fn flat_metric_is_refuted_as_no_differential() {
    let j = out(&grid("M5", &|_, _, _| None));
    let s = signals(&j);
    assert_eq!(s.len(), 1, "one metric-level verdict, no cell signals");
    assert_eq!(s[0].get("status").and_then(|v| v.as_str()), Some("refuted"));
    assert_eq!(s[0].get("reason").and_then(|v| v.as_str()), Some("no_differential"));
    assert_eq!(s[0].get("metric").and_then(|v| v.as_str()), Some("M5"));
}

fn explored_with_noise(f: &dyn Fn(&str, &str, &str) -> Option<i64>, extra: usize) -> Vec<String> {
    let mut rows = grid("M1", f);
    for i in 0..extra {
        for half in ["discovery", "holdout"] {
            for ch in CHANNELS {
                rows.push(row("M2", &format!("noise{i}"), ch, half, 120, 600));
            }
        }
    }
    rows
}

#[test]
fn multiplicity_counts_all_explored_cells_across_metrics() {
    // Moderate effect: admitted with m=10, not after correcting over 10 + 390 explored cells.
    let f = |r: &str, c: &str, _h: &str| (r == "Queja" && c == "Phone").then_some(265);
    let small = Config { multiplicity: Multiplicity::Bonferroni, alpha: 0.05, ..Config::default() };
    let status_of = |rows: &[String], cfg: &Config| {
        let j = parse(&analyse(&rows.join("
"), cfg).unwrap().to_json().write()).unwrap();
        (status(&j, "Queja", "Phone").map(String::from), j.get("cells_explored").and_then(|v| v.as_i64()))
    };
    let (s10, m10) = status_of(&explored_with_noise(&f, 0), &small);
    assert_eq!((s10.as_deref(), m10), (Some("corroborated"), Some(10)));
    let (s400, m400) = status_of(&explored_with_noise(&f, 195), &small);
    assert_eq!((s400.as_deref(), m400), (Some("uncertain"), Some(400)));
    // BH is less strict than Bonferroni but still depends on the whole explored family.
    let bh = Config { alpha: 0.05, ..Config::default() };
    let (sbh, _) = status_of(&explored_with_noise(&f, 195), &bh);
    assert!(matches!(sbh.as_deref(), Some("corroborated") | Some("uncertain")));
}

#[test]
fn bh_adjusted_p_is_reported_and_monotone_with_raw_p() {
    let j = out(&grid("M1", &planted_queja_phone));
    let s = find(&j, "Queja", "Phone").unwrap();
    let p = s.get("discovery").and_then(|d| d.get("p")).and_then(|v| v.as_f64()).unwrap();
    let q = s.get("p_adj").and_then(|v| v.as_f64()).unwrap();
    assert!(q >= p && q <= 1.0);
}

#[test]
fn k_violations_are_named_discards_and_never_tested() {
    let mut rows = grid("M1", &planted_queja_phone);
    rows.push(row("M1", "Tiny", "Phone", "discovery", 5, 9)); // denominator < k
    rows.push(row("M1", "Leaky", "Chat", "discovery", 4, 200)); // 0 < numerator < k
    rows.push(row("M1", "Leaky2", "Chat", "discovery", 195, 200)); // 0 < complement < k
    let j = out(&rows);
    assert_eq!(discard_count(&j, "k_violation"), 3);
    assert_eq!(j.get("cells_explored").and_then(|v| v.as_i64()), Some(10));
    assert!(find(&j, "Tiny", "Phone").is_none());
    assert!(find(&j, "Leaky", "Chat").is_none());
    assert_eq!(status(&j, "Queja", "Phone"), Some("corroborated"));
}

#[test]
fn favourable_direction_is_a_named_discard_not_a_problem() {
    let f = |r: &str, c: &str, _h: &str| (r == "Tecnico" && c == "Chat").then_some(100);
    let j = out(&grid("M1", &f));
    assert!(find(&j, "Tecnico", "Chat").is_none());
    assert!(discard_count(&j, "favourable_direction") >= 1);
}

#[test]
fn output_is_order_invariant_and_deterministic() {
    let rows = grid("M1", &planted_queja_phone);
    let mut rev = rows.clone();
    rev.reverse();
    let a = run(&rows.join("\n")).unwrap();
    assert_eq!(a, run(&rev.join("\n")).unwrap());
    assert_eq!(a, run(&rows.join("\n")).unwrap());
}

#[test]
fn signals_make_associations_never_causes() {
    let j = out(&grid("M1", &planted_queja_phone));
    let s = find(&j, "Queja", "Phone").unwrap();
    assert_eq!(s.get("claim").and_then(|v| v.as_str()), Some("association"));
    assert!(s.get("cause").is_none());
    assert!(s.get("mechanism").is_none());
    assert_eq!(
        j.get("method").and_then(|m| m.get("multiplicity")).and_then(|v| v.as_str()),
        Some("benjamini_hochberg_all_explored_cells")
    );
}

#[test]
fn rejects_identifier_like_fields_and_bad_counts() {
    let bad_dim = "{\"metric\":\"M1\",\"dims\":{\"customer_id\":\"CLI-1\"},\"half\":\"discovery\",\"numerator\":20,\"denominator\":100}";
    assert!(run(bad_dim).is_err());
    let extra = "{\"metric\":\"M1\",\"dims\":{\"channel\":\"Phone\"},\"half\":\"discovery\",\"numerator\":20,\"denominator\":100,\"description\":\"free text\"}";
    assert!(run(extra).is_err());
    let over = row("M1", "Queja", "Phone", "discovery", 120, 100);
    assert!(run(&over).is_err());
    let half = row("M1", "Queja", "Phone", "other", 20, 100);
    assert!(run(&half).is_err());
}

#[test]
fn config_is_explicit_and_k_is_ten() {
    let c = Config::default();
    assert_eq!(c.k_min, 10);
    assert!((c.alpha - 0.01).abs() < 1e-12);
    assert_eq!(c.multiplicity, Multiplicity::Bh);
    assert_eq!(c.min_support, 500);
    let r = analyse(&grid("M1", &planted_queja_phone).join("\n"), &c).unwrap();
    assert_eq!(r.cells_explored, 10);
}

// ---- follow-up: same-channel baseline, R2 windows, dependency flag ----

fn prow(metric: &str, reason: &str, channel: &str, half: &str, period: &str, num: i64, den: i64) -> String {
    format!(
        "{{\"metric\":\"{metric}\",\"dims\":{{\"reason_category\":\"{reason}\",\"channel\":\"{channel}\"}},\"half\":\"{half}\",\"period\":\"{period}\",\"numerator\":{num},\"denominator\":{den}}}"
    )
}

fn find_m<'a>(j: &'a Json, metric: &str, reason: &str, channel: &str) -> Option<&'a Json> {
    signals(j).into_iter().find(|s| {
        s.get("metric").and_then(|v| v.as_str()) == Some(metric)
            && s.get("dims").and_then(|d| d.get("reason_category")).and_then(|v| v.as_str()) == Some(reason)
            && s.get("dims").and_then(|d| d.get("channel")).and_then(|v| v.as_str()) == Some(channel)
    })
}

#[test]
fn channel_mix_and_a_dominant_reason_do_not_fabricate_findings() {
    // No reason effect anywhere: Phone 10% and Chat 40% for EVERY reason; Chat/Queja is huge.
    let mut rows = vec![];
    for r in REASONS {
        for (c, permille) in [("Phone", 100i64), ("Chat", 400i64)] {
            for (half, base) in [("discovery", 600i64), ("holdout", 600i64)] {
                let den = if r == "Queja" && c == "Chat" { base * 8 } else { base };
                rows.push(row("M1", r, c, half, den * permille / 1000, den));
            }
        }
    }
    let j = out(&rows);
    let found = signals(&j)
        .into_iter()
        .filter(|s| matches!(s.get("status").and_then(|v| v.as_str()), Some("corroborated") | Some("candidate")))
        .count();
    assert_eq!(found, 0, "pooled-over-channels baselines would flag every Chat cell");
}

#[test]
fn baseline_excludes_own_reason_within_the_same_channel() {
    let j = out(&grid("M1", &planted_queja_phone));
    let d = find(&j, "Queja", "Phone").unwrap().get("discovery").unwrap();
    // Phone rest = the other four Phone cells at 20%, not the whole metric.
    let base = d.get("baseline_rate").and_then(|v| v.as_f64()).unwrap();
    assert!((base - 0.2).abs() < 1e-9);
}

fn r2_rows(effect_in: &dyn Fn(&str) -> bool) -> Vec<String> {
    let periods = ["2023-08", "2024-06", "2025-03", "2026-02"];
    let mut rows = vec![];
    for r in REASONS {
        for c in CHANNELS {
            for half in ["discovery", "holdout"] {
                for p in periods {
                    let permille = if r == "Queja" && c == "Phone" && effect_in(p) { 300 } else { 200 };
                    rows.push(prow("M1", r, c, half, p, 300 * permille / 1000, 300));
                }
            }
        }
    }
    rows
}

#[test]
fn r2_windows_confirm_a_persistent_effect() {
    let j = out(&r2_rows(&|_| true));
    let s = find(&j, "Queja", "Phone").unwrap();
    assert_eq!(s.get("status").and_then(|v| v.as_str()), Some("corroborated"));
    let r2 = s.get("r2").expect("r2 present");
    assert_eq!(r2.get("status").and_then(|v| v.as_str()), Some("replicated"));
    assert!(r2.get("w1").is_some() && r2.get("w2").is_some());
}

#[test]
fn r2_windows_flag_an_effect_that_exists_in_one_window_only() {
    let j = out(&r2_rows(&|p| p < "2025"));
    let s = find(&j, "Queja", "Phone").unwrap();
    let r2 = s.get("r2").unwrap();
    assert_ne!(r2.get("status").and_then(|v| v.as_str()), Some("replicated"));
}

#[test]
fn r2_is_not_evaluated_without_periods_and_partial_months_are_ignored() {
    let j = out(&grid("M1", &planted_queja_phone));
    let s = find(&j, "Queja", "Phone").unwrap();
    assert_eq!(s.get("r2").and_then(|r| r.get("status")).and_then(|v| v.as_str()), Some("not_evaluated"));
    let mut rows = r2_rows(&|_| true);
    rows.push(prow("M1", "Queja", "Phone", "discovery", "2026-06", 290, 300)); // partial month, outside both windows
    let j2 = out(&rows);
    assert_eq!(
        find(&j2, "Queja", "Phone").unwrap().get("r2").and_then(|r| r.get("status")).and_then(|v| v.as_str()),
        Some("replicated")
    );
    let bad = prow("M1", "Queja", "Phone", "discovery", "2024-13", 20, 300);
    assert!(run(&bad).is_err());
}

#[test]
fn dependent_metric_findings_are_flagged_not_dropped() {
    let mut rows = grid("M1", &planted_queja_phone);
    let f6 = |r: &str, c: &str, _h: &str| match (r, c) {
        ("Queja", "Phone") | ("Tecnico", "Chat") => Some(300),
        _ => None,
    };
    rows.extend(grid("M6", &f6));
    let j = out(&rows);
    let dep = find_m(&j, "M6", "Queja", "Phone").expect("kept");
    assert_eq!(dep.get("status").and_then(|v| v.as_str()), Some("corroborated"));
    assert_eq!(dep.get("depends_on").and_then(|v| v.as_str()), Some("M1"));
    // The flag comes from the dependency table, not from a parent finding on the same cell.
    let indep = find_m(&j, "M6", "Tecnico", "Chat").expect("kept");
    assert_eq!(indep.get("depends_on").and_then(|v| v.as_str()), Some("M1"));
    assert!(find_m(&j, "M1", "Queja", "Phone").unwrap().get("depends_on").is_none());
}

#[test]
fn handled_time_share_m10_is_flagged_as_a_re_expression_of_m1() {
    let mut rows = grid("M1", &planted_queja_phone);
    let f10 = |r: &str, c: &str, _h: &str| match (r, c) {
        ("Queja", "Phone") | ("Tecnico", "Chat") => Some(300),
        _ => None,
    };
    rows.extend(grid("M10", &f10));
    let j = out(&rows);
    let dep = find_m(&j, "M10", "Queja", "Phone").expect("kept");
    assert_eq!(dep.get("depends_on").and_then(|v| v.as_str()), Some("M1"));
    assert_eq!(find_m(&j, "M10", "Tecnico", "Chat").unwrap().get("depends_on").and_then(|v| v.as_str()), Some("M1"));
}

#[test]
fn ag2_dimensions_are_accepted_and_unknown_ones_still_rejected() {
    let ok = r#"{"metric":"M7","dims":{"action":"initiate_transfer","channel":"App"},"half":"discovery","period":"2024-01","numerator":0,"denominator":600}"#;
    assert!(analyse(ok, &Config::default()).is_ok());
    let ok2 = r#"{"metric":"M9","dims":{"customer_segment":"Plus","channel":"Web"},"half":"holdout","numerator":30,"denominator":600}"#;
    assert!(analyse(ok2, &Config::default()).is_ok());
    let ok3 = r#"{"metric":"M8","dims":{"campaign_type":"Push","channel":"Email"},"half":"holdout","numerator":300,"denominator":600}"#;
    assert!(analyse(ok3, &Config::default()).is_ok());
    let bad = r#"{"metric":"M7","dims":{"customer_id":"x"},"half":"discovery","numerator":1,"denominator":600}"#;
    assert!(analyse(bad, &Config::default()).is_err());
}

// ---- W1-4: `level_risk` finding type (a level against a pre-registered threshold, not a vs-rest contrast) ----

fn m_row(metric: &str, ctype: &str, channel: &str, half: &str, period: &str, num: i64, den: i64) -> String {
    format!(
        "{{\"metric\":\"{metric}\",\"dims\":{{\"campaign_type\":\"{ctype}\",\"channel\":\"{channel}\"}},\"half\":\"{half}\",\"period\":\"{period}\",\"numerator\":{num},\"denominator\":{den}}}"
    )
}

/// 3 campaign types x 2 channels x 2 halves x 4 periods (two per R2 window); `permille(half)` is the level.
fn level_grid(metric: &str, permille: &dyn Fn(&str, &str) -> i64) -> Vec<String> {
    let mut rows = vec![];
    for t in ["Push", "Promo", "Alert"] {
        for c in ["Email", "Sms"] {
            for half in ["discovery", "holdout"] {
                for p in ["2023-08", "2024-03", "2025-02", "2026-02"] {
                    let den = 400;
                    rows.push(m_row(metric, t, c, half, p, den * permille(half, p) / 1000, den));
                }
            }
        }
    }
    rows
}

fn level_signals(j: &Json) -> Vec<&Json> {
    signals(j).into_iter().filter(|s| s.get("type").and_then(|v| v.as_str()) == Some("level_risk")).collect()
}

fn level_of<'a>(j: &'a Json, metric: &str) -> Option<&'a Json> {
    level_signals(j).into_iter().find(|s| s.get("metric").and_then(|v| v.as_str()) == Some(metric))
}

fn st<'a>(s: &'a Json) -> &'a str {
    s.get("status").and_then(|v| v.as_str()).unwrap()
}

#[test]
fn level_risk_surfaces_m8_style_level_with_interval_support_and_replication() {
    let j = out(&level_grid("M8", &|_, _| 500));
    let s = level_of(&j, "M8").expect("level risk signal");
    assert_eq!(st(s), "corroborated");
    assert_eq!(s.get("class").and_then(|v| v.as_str()), Some("risk"));
    assert_eq!(s.get("claim").and_then(|v| v.as_str()), Some("association"));
    assert!(s.get("cause").is_none() && s.get("mechanism").is_none());
    let d = s.get("discovery").unwrap();
    assert_eq!(d.get("denominator").and_then(|v| v.as_i64()), Some(3 * 2 * 4 * 400));
    assert!((d.get("rate").and_then(|v| v.as_f64()).unwrap() - 0.5).abs() < 1e-9);
    assert!((d.get("baseline_rate").and_then(|v| v.as_f64()).unwrap() - 0.10).abs() < 1e-9, "threshold is the baseline");
    let (lo, hi) = (d.get("ci95_low").and_then(|v| v.as_f64()).unwrap(), d.get("ci95_high").and_then(|v| v.as_f64()).unwrap());
    assert!(lo < 0.5 && 0.5 < hi && lo > 0.10);
    assert!(s.get("holdout").is_some());
    assert_eq!(s.get("r2").and_then(|r| r.get("status")).and_then(|v| v.as_str()), Some("replicated"));
    let per = s.get("periods").unwrap();
    assert_eq!(per.get("total").and_then(|v| v.as_i64()), Some(4));
    assert_eq!(per.get("above_threshold").and_then(|v| v.as_i64()), Some(4));
    assert_eq!(s.get("level_cells").and_then(|c| c.get("above_threshold")).and_then(|v| v.as_i64()), Some(6));
}

#[test]
fn level_risk_does_not_replace_the_vs_rest_verdict_flat_m8_stays_no_differential() {
    let j = out(&level_grid("M8", &|_, _| 500));
    let contrast: Vec<&Json> = signals(&j).into_iter().filter(|s| s.get("type").is_none() && s.get("metric").and_then(|v| v.as_str()) == Some("M8")).collect();
    assert_eq!(contrast.len(), 1);
    assert_eq!(st(contrast[0]), "refuted");
    assert_eq!(contrast[0].get("reason").and_then(|v| v.as_str()), Some("no_differential"));
}

#[test]
fn level_at_or_below_the_threshold_is_refuted_not_a_risk() {
    let j = out(&level_grid("M8", &|_, _| 60));
    assert_eq!(st(level_of(&j, "M8").unwrap()), "refuted");
}

#[test]
fn level_above_threshold_but_under_the_excess_floor_is_not_a_risk() {
    // 12% vs the 10% threshold: significant on this support, but under the 5 pp excess floor.
    let j = out(&level_grid("M8", &|_, _| 120));
    let s = level_of(&j, "M8").unwrap();
    assert_eq!(st(s), "refuted");
    assert_eq!(s.get("reason").and_then(|v| v.as_str()), Some("excess_below_floor"));
}

#[test]
fn level_not_confirmed_in_the_holdout_half_is_refuted_or_uncertain_never_corroborated() {
    let j = out(&level_grid("M8", &|h, _| if h == "discovery" { 500 } else { 50 }));
    assert_eq!(st(level_of(&j, "M8").unwrap()), "refuted");
    let j = out(&level_grid("M8", &|h, _| if h == "discovery" { 500 } else { 110 }));
    assert_ne!(st(level_of(&j, "M8").unwrap()), "corroborated");
}

#[test]
fn level_without_holdout_is_candidate() {
    let rows: Vec<String> = level_grid("M8", &|_, _| 500).into_iter().filter(|r| r.contains("\"discovery\"")).collect();
    assert_eq!(st(level_of(&out(&rows), "M8").unwrap()), "candidate");
}

#[test]
fn level_in_one_window_only_is_flagged_by_r2_and_periods() {
    let j = out(&level_grid("M8", &|_, p| if p < "2025" { 500 } else { 40 }));
    let s = level_of(&j, "M8").unwrap();
    assert_eq!(s.get("periods").and_then(|p| p.get("above_threshold")).and_then(|v| v.as_i64()), Some(2));
    assert_eq!(s.get("r2").and_then(|r| r.get("status")).and_then(|v| v.as_str()), Some("reversed"));
}

#[test]
fn only_pre_registered_metrics_get_level_tests_m7_stays_descriptive_and_m9_refuted() {
    // M7 6.0% (above 4.5%) is a descriptive gap: never a level risk, never a contrast finding at the 5 pp floor.
    let mut rows = level_grid("M7", &|_, _| 60);
    rows.extend(level_grid("M9", &|_, _| 50));
    let j = out(&rows);
    assert!(level_signals(&j).is_empty());
    assert!(signals(&j).iter().all(|s| matches!(st(s), "refuted")));
    assert_eq!(j.get("level_tests").and_then(|v| v.as_i64()), Some(1), "the registered family size, fixed in advance");
}

#[test]
fn level_tests_are_a_separate_pre_registered_family_with_their_own_bonferroni() {
    let rows = level_grid("M8", &|_, _| 500);
    let r = analyse(&rows.join("
"), &Config::default()).unwrap();
    assert_eq!(r.level_tests, 1);
    assert_eq!(r.cells_explored, 6, "level tests do not dilute or inflate the contrast family");
    // A weak level (11.5% vs 10% on n = 800) so p is not saturated; two registered specs double the adjusted p.
    let weak: Vec<String> = ["2024-03", "2024-04"].iter().map(|p| m_row("M8", "Push", "Email", "discovery", p, 46, 400)).collect();
    let mut one = Config::default();
    one.alpha = 0.5;
    one.level_risks[0].min_excess = 0.0;
    let mut two = one.clone();
    let spec = two.level_risks[0].clone();
    two.level_risks.push(steps::cells::LevelSpec { metric: "M8B".into(), ..spec });
    let (a, b) = (analyse(&weak.join("
"), &one).unwrap(), analyse(&weak.join("
"), &two).unwrap());
    assert_eq!((a.level_tests, b.level_tests), (1, 2));
    let (p1, p2) = (a.level_signals[0].p_adj, b.level_signals.iter().find(|s| s.metric == "M8").unwrap().p_adj);
    assert!(p1 > 0.0 && p1 < 0.5);
    assert!((p2 - (p1 * 2.0).min(1.0)).abs() < 1e-12);
    let j = parse(&run(&rows.join("
")).unwrap()).unwrap();
    assert!(j.get("method").and_then(|m| m.get("level_risk")).is_some());
}

#[test]
fn level_rows_obey_the_k_rule_and_min_support_with_named_discards() {
    let mut rows = level_grid("M8", &|_, _| 500);
    rows.push(m_row("M8", "Push", "Chat", "discovery", "2024-03", 5, 100)); // k violation
    let j = out(&rows);
    assert_eq!(discard_count(&j, "k_violation"), 1);
    let small: Vec<String> = vec![m_row("M8", "Push", "Email", "discovery", "2024-03", 200, 400), m_row("M8", "Push", "Email", "holdout", "2024-03", 200, 400)];
    let j = out(&small);
    assert!(level_of(&j, "M8").is_none());
    assert_eq!(discard_count(&j, "level_below_min_support"), 1);
}

// ---------------------------------------------------------------- DET1: full-period cells, pooled support, exploratory

fn dims1(reason: &str, channel: Option<&str>) -> String {
    match channel {
        Some(c) => format!("{{\"reason_category\":\"{reason}\",\"channel\":\"{c}\"}}"),
        None => format!("{{\"reason_category\":\"{reason}\"}}"),
    }
}

fn arow(metric: &str, reason: &str, channel: Option<&str>, half: &str, period: &str, num: i64, den: i64) -> String {
    format!("{{\"metric\":\"{metric}\",\"dims\":{},\"half\":\"{half}\",\"period\":\"{period}\",\"numerator\":{num},\"denominator\":{den}}}", dims1(reason, channel))
}

/// Full-period table for one channel: a big filler reason at 20% and a planted reason with the given (disc den, disc num, hold den, hold num).
fn full_table(planted: (i64, i64, i64, i64)) -> Vec<String> {
    let mut v = vec![];
    for (r, d, h) in [("Informativo", (3000, 600, 3000, 600), 0), ("Transaccional", (3000, 600, 3000, 600), 0), ("Comercial", planted, 0)] {
        let _ = h;
        v.push(arow("M1", r, Some("WhatsApp"), "discovery", "ALL", d.1, d.0));
        v.push(arow("M1", r, Some("WhatsApp"), "holdout", "ALL", d.3, d.2));
    }
    v
}

fn dsig<'a>(j: &'a Json, reason: &str) -> Option<&'a Json> {
    signals(j).into_iter().find(|s| s.get("dims").and_then(|d| d.get("reason_category")).and_then(|v| v.as_str()) == Some(reason))
}

fn dst<'a>(s: Option<&'a Json>) -> (&'a str, &'a str) {
    let s = s.expect("signal");
    (s.get("status").and_then(|v| v.as_str()).unwrap(), s.get("reason").and_then(|v| v.as_str()).unwrap())
}

#[test]
fn support_floor_applies_to_the_pooled_period_not_the_discovery_half() {
    // 240 + 260 = 500 pooled: the old discovery-half floor (500) dropped it; both halves are enough to replicate (>= 250 / 2).
    let j = out(&full_table((240, 96, 260, 104))); // 40% vs 20%
    assert_eq!(dst(dsig(&j, "Comercial")), ("corroborated", "replicated_in_holdout"));
    assert_eq!(discard_count(&j, "below_min_support"), 0);
    // below the pooled floor it is still a named discard
    let j = out(&full_table((200, 80, 200, 80)));
    assert!(dsig(&j, "Comercial").is_none());
    assert_eq!(discard_count(&j, "below_min_support"), 1);
}

#[test]
fn underpowered_holdout_is_uncertain_not_dropped() {
    let j = out(&full_table((500, 200, 60, 24)));
    assert_eq!(dst(dsig(&j, "Comercial")), ("uncertain", "replication_underpowered"));
}

#[test]
fn full_period_rows_set_the_half_totals_and_month_rows_are_not_double_counted() {
    let mut rows = full_table((300, 120, 300, 120));
    for m in ["2024-01", "2024-02", "2025-03"] {
        rows.push(arow("M1", "Comercial", Some("WhatsApp"), "discovery", m, 40, 100));
    }
    let j = out(&rows);
    let d = dsig(&j, "Comercial").unwrap().get("discovery").unwrap();
    assert_eq!(d.get("denominator").and_then(|v| v.as_i64()), Some(300));
    assert_eq!(d.get("baseline_denominator").and_then(|v| v.as_i64()), Some(6000));
}

#[test]
fn window_rows_feed_r2() {
    let mut rows = full_table((300, 120, 300, 120));
    for (w, h) in [("W1", "discovery"), ("W1", "holdout"), ("W2", "discovery"), ("W2", "holdout")] {
        rows.push(arow("M1", "Comercial", Some("WhatsApp"), h, w, 60, 150));
        for r in ["Informativo", "Transaccional"] {
            rows.push(arow("M1", r, Some("WhatsApp"), h, w, 300, 1500));
        }
    }
    let j = out(&rows);
    assert_eq!(dsig(&j, "Comercial").unwrap().get("r2").and_then(|r| r.get("status")).and_then(|v| v.as_str()), Some("replicated"));
}

#[test]
fn reason_only_cells_are_their_own_family_with_their_own_multiplicity() {
    let mut rows = full_table((300, 120, 300, 120));
    for (r, d, h) in [("Informativo", (3000, 600), (3000, 600)), ("Transaccional", (3000, 600), (3000, 600)), ("Comercial", (300, 120), (300, 120))] {
        rows.push(arow("M1", r, None, "discovery", "ALL", d.1, d.0));
        rows.push(arow("M1", r, None, "holdout", "ALL", h.1, h.0));
    }
    let j = out(&rows);
    let fam = j.get("method").and_then(|m| m.get("families")).unwrap();
    assert_eq!(fam.get("main").and_then(|v| v.as_i64()), Some(3));
    assert_eq!(fam.get("reason_only").and_then(|v| v.as_i64()), Some(3));
    assert_eq!(j.get("cells_explored").and_then(|v| v.as_i64()), Some(6));
    assert!(signals(&j).iter().any(|s| s.get("dims").and_then(|d| d.get("channel")).is_none() && s.get("status").and_then(|v| v.as_str()) == Some("corroborated")));
}

#[test]
fn exploratory_tier_relaxes_knobs_but_never_k_and_never_corroborates() {
    // +4 pp (below the 5 pp strict floor), well powered: strict ignores it, exploratory labels it.
    let rows = full_table((3000, 720, 3000, 720)); // 24% vs 20%
    let strict = out(&rows);
    assert!(dsig(&strict, "Comercial").is_none());
    let j = parse(&run_exploratory(&rows.join("
")).unwrap()).unwrap();
    let s = dsig(&j, "Comercial").expect("exploratory candidate");
    assert_eq!(dst(Some(s)).0, "candidate_exploratory");
    assert!(s.get("priority").and_then(|v| v.as_f64()).unwrap() > 0.0);
    assert!(s.get("exploratory_note").and_then(|v| v.as_str()).unwrap().starts_with("exploratory:"));
    assert_eq!(j.get("method").and_then(|m| m.get("profile")).and_then(|v| v.as_str()), Some("exploratory"));
    assert!(signals(&j).iter().all(|s| s.get("status").and_then(|v| v.as_str()) != Some("corroborated")));
    // k stays: a row with a numerator of 5 is still a named discard
    let mut bad = rows.clone();
    bad.push(arow("M1", "Queja", Some("WhatsApp"), "discovery", "ALL", 5, 900));
    let j = parse(&run_exploratory(&bad.join("
")).unwrap()).unwrap();
    assert_eq!(discard_count(&j, "k_violation"), 1);
    assert!(dsig(&j, "Queja").is_none());
}

#[test]
fn exploratory_profile_leaves_the_strict_tier_untouched() {
    let rows = full_table((300, 120, 300, 120));
    let a = out(&rows);
    let b = parse(&run_exploratory(&rows.join("
")).unwrap()).unwrap();
    assert_eq!(dst(dsig(&a, "Comercial")), dst(dsig(&b, "Comercial")));
    assert_eq!(analyse(&rows.join("
"), &Config::default()).unwrap().signals.len(), analyse(&rows.join("
"), &Config::default()).unwrap().signals.len());
}
