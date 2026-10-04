//! L1 `cells` sensor: deterministic recurring-problem signals over TREATED cell tables.
//! Test-first. Tables here are synthetic aggregates (no ids, no free text).
use steps::cells::{Config, Multiplicity, analyse, run};
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
