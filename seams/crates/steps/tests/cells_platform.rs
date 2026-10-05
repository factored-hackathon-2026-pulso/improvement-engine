//! EVT1: the cells sensor over PLATFORM event cells (`P_*` metrics, `scripts/aggregate/platform_event_cells.py`).
//! Test-first. Tables are synthetic aggregates (no ids, no free text): the dimensions are case_type, channel, language, release, agent, tool.
use steps::cells::{Config, analyse, run, run_platform};
use steps::sensor::json::{Json, parse};

fn cell(metric: &str, dims: &[(&str, &str)], half: &str, period: &str, num: i64, den: i64) -> String {
    let d: Vec<String> = dims.iter().map(|(k, v)| format!("\"{k}\":\"{v}\"")).collect();
    format!("{{\"metric\":\"{metric}\",\"dims\":{{{}}},\"half\":\"{half}\",\"period\":\"{period}\",\"numerator\":{num},\"denominator\":{den}}}", d.join(","))
}

/// ALL rows for both halves plus W1/W2 rows (each window gets half of the half's count), the aggregator's contract.
fn full(metric: &str, dims: &[(&str, &str)], permille: i64, den_disc: i64, den_hold: i64) -> Vec<String> {
    let mut v = vec![];
    for (half, den) in [("discovery", den_disc), ("holdout", den_hold)] {
        v.push(cell(metric, dims, half, "ALL", den * permille / 1000, den));
        for w in ["W1", "W2"] {
            v.push(cell(metric, dims, half, w, den / 2 * permille / 1000, den / 2));
        }
    }
    v
}

const TYPES: [&str; 5] = ["unrecognized_charge", "undue_charge", "app_issue", "branch_service", "service_quality"];
const CHANNELS: [&str; 2] = ["app_chat", "web_chat"];

fn grid(metric: &str, planted: &dyn Fn(&str, &str) -> Option<i64>) -> Vec<String> {
    let mut rows = vec![];
    for t in TYPES {
        for c in CHANNELS {
            let p = planted(t, c).unwrap_or(400);
            rows.extend(full(metric, &[("case_type", t), ("channel", c)], p, 600, 400));
        }
    }
    rows
}

fn platform(rows: &[String]) -> Json {
    parse(&run_platform(&rows.join("\n")).expect("run_platform")).expect("json")
}

fn signals(j: &Json) -> Vec<&Json> {
    j.get("signals").and_then(|s| s.as_arr()).unwrap().iter().collect()
}

fn find<'a>(j: &'a Json, metric: &str, dims: &[(&str, &str)]) -> Option<&'a Json> {
    signals(j).into_iter().find(|s| {
        s.get("metric").and_then(|m| m.as_str()) == Some(metric)
            && s.get("dims").is_some_and(|d| dims.iter().all(|(k, v)| d.get(k).and_then(|x| x.as_str()) == Some(v)) && matches!(d, Json::Obj(o) if o.len() == dims.len()))
    })
}

fn status<'a>(j: &'a Json, metric: &str, dims: &[(&str, &str)]) -> Option<&'a str> {
    find(j, metric, dims).and_then(|s| s.get("status")).and_then(|v| v.as_str())
}

#[test]
fn platform_dimensions_are_accepted_and_ids_or_free_text_dimensions_still_rejected() {
    for dim in ["language", "release", "agent", "tool", "case_type", "channel"] {
        let row = cell("P_DRAFT_REJECT", &[(dim, "x")], "discovery", "ALL", 20, 100);
        assert!(analyse(&row, &Config::platform()).is_ok(), "{dim} must be an allowed dimension");
    }
    for bad in ["case_id", "customer_id", "analyst_id", "text", "note", "staff_id"] {
        let row = cell("P_DRAFT_REJECT", &[(bad, "x")], "discovery", "ALL", 20, 100);
        assert!(analyse(&row, &Config::platform()).is_err(), "{bad} must be rejected");
    }
}

#[test]
fn a_planted_case_type_is_corroborated_against_the_same_channel_baseline_only() {
    let rows = grid("P_DRAFT_REJECT", &|t, c| (t == "service_quality" && c == "app_chat").then_some(750));
    let j = platform(&rows);
    assert_eq!(status(&j, "P_DRAFT_REJECT", &[("case_type", "service_quality"), ("channel", "app_chat")]), Some("corroborated"));
    // the other channel's service_quality cell is at the base rate: not a finding
    assert_ne!(status(&j, "P_DRAFT_REJECT", &[("case_type", "service_quality"), ("channel", "web_chat")]), Some("corroborated"));
    let s = find(&j, "P_DRAFT_REJECT", &[("case_type", "service_quality"), ("channel", "app_chat")]).unwrap();
    let d = s.get("discovery").unwrap();
    // baseline = the 4 other case types of app_chat only (4 x 600 discovery, rate 0.40)
    assert_eq!(d.get("baseline_denominator").and_then(|v| v.as_i64()), Some(4 * 600));
    assert_eq!(d.get("baseline_numerator").and_then(|v| v.as_i64()), Some(4 * 240));
}

#[test]
fn signatures_of_one_metric_do_not_contaminate_each_others_baseline() {
    // case_type x channel cells AND language x channel cells of the SAME metric describe the same population twice.
    let mut rows = grid("P_DRAFT_REJECT", &|t, c| (t == "service_quality" && c == "app_chat").then_some(750));
    for l in ["es", "pt"] {
        for c in CHANNELS {
            rows.extend(full("P_DRAFT_REJECT", &[("language", l), ("channel", c)], 400, 1500, 1000));
        }
    }
    let j = platform(&rows);
    let s = find(&j, "P_DRAFT_REJECT", &[("case_type", "service_quality"), ("channel", "app_chat")]).unwrap();
    assert_eq!(s.get("discovery").unwrap().get("baseline_denominator").and_then(|v| v.as_i64()), Some(4 * 600), "language cells must not enter the case_type baseline");
    // and the language x channel cells are tested against their own signature (flat: no finding)
    assert_ne!(status(&j, "P_DRAFT_REJECT", &[("language", "es"), ("channel", "app_chat")]), Some("corroborated"));
}

#[test]
fn a_contrast_dimension_is_dropped_by_priority_so_release_is_compared_within_the_agent() {
    let mut rows = vec![];
    for r in ["rel-a", "rel-b", "rel-c"] {
        for a in ["copiloto-asesor@1.0.0", "recepcion@1.0.0"] {
            let p = if r == "rel-b" && a == "copiloto-asesor@1.0.0" { 700 } else { 400 };
            rows.extend(full("P_DRAFT_REJECT", &[("release", r), ("agent", a)], p, 600, 400));
        }
    }
    let j = platform(&rows);
    let s = find(&j, "P_DRAFT_REJECT", &[("release", "rel-b"), ("agent", "copiloto-asesor@1.0.0")]).expect("signal");
    assert_eq!(s.get("status").and_then(|v| v.as_str()), Some("corroborated"));
    // baseline: the other releases of the SAME agent only
    assert_eq!(s.get("discovery").unwrap().get("baseline_denominator").and_then(|v| v.as_i64()), Some(2 * 600));
}

#[test]
fn a_tool_cell_is_compared_with_the_same_tool_in_other_case_types() {
    let mut rows = vec![];
    for t in TYPES {
        for tool in ["consultar_cargos", "estado_pqr"] {
            let p = if t == "undue_charge" && tool == "consultar_cargos" { 700 } else { 250 };
            rows.extend(full("P_TOOL_USE", &[("case_type", t), ("tool", tool)], p, 600, 400));
        }
    }
    let j = platform(&rows);
    assert_eq!(status(&j, "P_TOOL_USE", &[("case_type", "undue_charge"), ("tool", "consultar_cargos")]), Some("corroborated"));
    assert_eq!(find(&j, "P_TOOL_USE", &[("case_type", "undue_charge"), ("tool", "consultar_cargos")]).unwrap().get("discovery").unwrap().get("baseline_denominator").and_then(|v| v.as_i64()), Some(4 * 600));
    assert_ne!(status(&j, "P_TOOL_USE", &[("case_type", "undue_charge"), ("tool", "estado_pqr")]), Some("corroborated"));
}

#[test]
fn flat_platform_metrics_produce_no_finding_only_a_refuted_no_differential() {
    let j = platform(&grid("P_SUGG_NONE", &|_, _| Some(60)));
    assert!(signals(&j).iter().all(|s| s.get("status").and_then(|v| v.as_str()) == Some("refuted")), "{}", j.write());
}

#[test]
fn the_suggestion_failed_rate_is_a_pre_registered_level_risk_and_the_bank_config_is_untouched() {
    let mut rows = vec![];
    for l in ["es", "pt"] {
        for c in CHANNELS {
            rows.extend(full("P_SUGG_FAILED", &[("channel", c), ("language", l)], 90, 800, 600));
        }
    }
    let j = platform(&rows);
    assert_eq!(j.get("level_tests").and_then(|v| v.as_i64()), Some(1));
    let lvl = signals(&j).into_iter().find(|s| s.get("type").and_then(|t| t.as_str()) == Some("level_risk")).expect("level signal");
    assert_eq!(lvl.get("metric").and_then(|v| v.as_str()), Some("P_SUGG_FAILED"));
    assert_eq!(lvl.get("status").and_then(|v| v.as_str()), Some("corroborated"));
    assert_eq!(lvl.get("class").and_then(|v| v.as_str()), Some("risk"));
    // the default (bank) config does not know P_SUGG_FAILED as a level risk and keeps its single level test
    let bank: Json = parse(&run(&rows.join("\n")).unwrap()).unwrap();
    assert_eq!(bank.get("level_tests").and_then(|v| v.as_i64()), Some(1));
    assert!(signals(&bank).iter().all(|s| s.get("type").and_then(|t| t.as_str()) != Some("level_risk")));
    assert_eq!(Config::default().level_risks.len(), 1);
    assert_eq!(Config::platform().level_risks.len(), 1);
}

#[test]
fn a_level_below_the_pre_registered_threshold_is_refuted() {
    let mut rows = vec![];
    for l in ["es", "pt"] {
        for c in CHANNELS {
            rows.extend(full("P_SUGG_FAILED", &[("channel", c), ("language", l)], 30, 800, 600));
        }
    }
    let j = platform(&rows);
    let lvl = signals(&j).into_iter().find(|s| s.get("type").and_then(|t| t.as_str()) == Some("level_risk")).expect("level signal");
    assert_eq!(lvl.get("status").and_then(|v| v.as_str()), Some("refuted"));
}

#[test]
fn pooled_support_applies_to_the_full_period_for_platform_cells_with_a_lower_floor() {
    // 120 discovery + 100 holdout = 220 pooled: below the bank floor (500) but above the platform floor (200)
    let mut rows = vec![];
    for t in TYPES {
        let p = if t == "service_quality" { 800 } else { 300 };
        rows.extend(full("P_DRAFT_REJECT", &[("case_type", t), ("channel", "web_chat")], p, 120, 100));
    }
    let j = platform(&rows);
    assert_eq!(status(&j, "P_DRAFT_REJECT", &[("case_type", "service_quality"), ("channel", "web_chat")]), Some("corroborated"));
    let bank = parse(&run(&rows.join("\n")).unwrap()).unwrap();
    assert_eq!(status(&bank, "P_DRAFT_REJECT", &[("case_type", "service_quality"), ("channel", "web_chat")]), None, "bank floor 500 discards it");
    assert_eq!(Config::platform().min_support, 200);
}

#[test]
fn k_violating_platform_rows_are_a_named_discard_with_a_suppressed_count_below_k() {
    let mut rows = grid("P_DRAFT_REJECT", &|_, _| None);
    rows.push(cell("P_DRAFT_REJECT", &[("case_type", "app_issue"), ("channel", "email")], "discovery", "ALL", 4, 90));
    let j = platform(&rows);
    let d = j.get("discards").and_then(|d| d.as_arr()).unwrap();
    let e = d.iter().find(|x| x.get("kind").and_then(|k| k.as_str()) == Some("k_violation")).expect("k_violation discard");
    assert_eq!(e.get("count"), Some(&Json::Null));
    assert_eq!(e.get("suppressed"), Some(&Json::Bool(true)));
}

#[test]
fn the_report_names_its_baseline_and_windows_and_carries_no_ids_or_text() {
    let j = platform(&grid("P_DRAFT_REJECT", &|t, c| (t == "service_quality" && c == "app_chat").then_some(750)));
    let text = j.write();
    for needle in ["CASE-", "CUS-", "analyst_id", "customer_id"] {
        assert!(!text.contains(needle), "{needle}");
    }
    let m = j.get("method").unwrap();
    assert_eq!(m.get("baseline").and_then(|v| v.as_str()), Some("same_channel_full_period_published_cells_minus_own_cell"));
    assert_eq!(m.get("profile").and_then(|v| v.as_str()), Some("platform"));
    let s = find(&j, "P_DRAFT_REJECT", &[("case_type", "service_quality"), ("channel", "app_chat")]).unwrap();
    assert_eq!(s.get("r2").and_then(|r| r.get("status")).and_then(|v| v.as_str()), Some("replicated"));
}

#[test]
fn a_channel_cell_of_a_tool_is_compared_with_the_same_tool_in_other_channels_never_with_other_tools() {
    // consultar_cargos is naturally used more than estado_pqr in EVERY channel (a tool-popularity difference, not a finding);
    // only email departs from the other channels for the same tool.
    let mut rows = vec![];
    for c in ["email", "app_chat", "web_chat", "phone_inbound"] {
        for (tool, p) in [("consultar_cargos", if c == "email" { 600 } else { 250 }), ("estado_pqr", 100)] {
            rows.extend(full("P_TOOL_USE", &[("channel", c), ("tool", tool)], p, 600, 400));
        }
    }
    let j = platform(&rows);
    assert_eq!(status(&j, "P_TOOL_USE", &[("channel", "email"), ("tool", "consultar_cargos")]), Some("corroborated"));
    let s = find(&j, "P_TOOL_USE", &[("channel", "email"), ("tool", "consultar_cargos")]).unwrap();
    assert_eq!(s.get("discovery").unwrap().get("baseline_denominator").and_then(|v| v.as_i64()), Some(3 * 600), "the same tool in the other channels");
    for c in ["app_chat", "web_chat", "phone_inbound"] {
        assert_ne!(status(&j, "P_TOOL_USE", &[("channel", c), ("tool", "consultar_cargos")]), Some("corroborated"), "{c}: popularity of a tool is not a finding");
    }
}
