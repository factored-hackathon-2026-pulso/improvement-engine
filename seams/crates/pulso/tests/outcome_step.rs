//! OUT1: the outcome step. A `release.*` trigger -> the finding the proposal was announced for -> the treated cell and its controls
//! (pre months from the pre tables, post months from the post tables) -> the estimator over the JSON CLI contract (scripted double,
//! a subprocess) -> a verdict card that never claims success without `improved`. Offline: a Python double stands in for the estimator.
use debug_api::{NewEvent, RunEventSink, Store};
use pg::repo::{JobRepository, MemRepo};
use pulso::config::RunConfig;
use pulso::run::engine_job::EngineRunner;
use pulso::run::outcome::{OutcomeStep, Trigger, build_table, parse_verdict};
use pulso::run::supervisor::StopToken;
use pulso::run::tasks::{JobCtx, JobRunner};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const T: &str = "tenant-local";
const KEY: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pulso-out1-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("value-loop")).unwrap();
    d
}

fn scripts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/out1")
}

fn dims(reason: &str, channel: &str) -> Value {
    json!({"reason_category": reason, "channel": channel})
}

/// 2024-01..2024-12, both halves, treated Queja/Phone plus Tecnico/Phone and Comercial/Phone (controls) and noise cells that must NOT reach the estimator.
fn cells(path: &Path) {
    let mut out = String::new();
    for m in 1..=12 {
        for half in ["discovery", "holdout"] {
            for (reason, channel) in [("Queja", "Phone"), ("Tecnico", "Phone"), ("Comercial", "Phone"), ("Queja", "App"), ("Tecnico", "App")] {
                out += &format!("{}\n", json!({"metric": "M1", "dims": dims(reason, channel), "half": half, "period": format!("2024-{m:02}"), "numerator": 100 + m, "denominator": 1000}));
            }
            out += &format!("{}\n", json!({"metric": "M2", "dims": {"channel": "Phone"}, "half": half, "period": format!("2024-{m:02}"), "numerator": 50, "denominator": 900}));
        }
    }
    std::fs::write(path, out).unwrap();
}

fn canned_cell(status: &str, reason: Option<&str>, effect: Option<f64>, interval: Option<[f64; 2]>) -> Value {
    json!({"metric": "M1", "dims": dims("Queja", "Phone"), "effect_pp": effect, "interval_family_adjusted_pp": interval, "n_pre": 6000, "n_post": 6000,
        "control": "same_metric_published_siblings", "control_dimension": "reason_category", "control_siblings": [dims("Comercial", "Phone"), dims("Tecnico", "Phone")],
        "status": status, "reason": reason, "uncertainty_method": "naive_independent_binomial_not_customer_cluster_adjusted", "interpretation": "descriptive_association_not_causation"})
}

fn canned(work: &Path, cell: Value) -> PathBuf {
    let p = work.join("canned.json");
    std::fs::write(&p, json!({"contract": "pulso.outcome.v1", "release_period": "2024-07", "window_months": 3, "cells": [cell]}).to_string()).unwrap();
    p
}

fn step(work: &Path, canned: &Path, extra: &[(&str, String)]) -> OutcomeStep {
    let pre = work.join("cells.ndjson");
    cells(&pre);
    let mut m: HashMap<String, String> = HashMap::new();
    m.insert("PULSO_OUTCOME_PRE_CELLS".into(), pre.display().to_string());
    m.insert("PULSO_OUTCOME_PSEUDO_RELEASE".into(), "2024-07".into());
    m.insert("PULSO_OUTCOME_CMD".into(), format!("python {} --canned {} --log {}", scripts().join("outcome_double.py").display(), canned.display(), work.join("calls.log").display()));
    for (k, v) in extra {
        m.insert((*k).into(), v.clone());
    }
    OutcomeStep::from_lookup(&|k| m.get(k).cloned(), Some(work)).unwrap().expect("configured")
}

/// The record the value loop leaves for an announced proposal.
fn announced(work: &Path) {
    let doc = json!({"contract": "value-loop/b3-0", "findings": [
        {"finding_id": "finding_1", "evidence_ref": "ev:abc", "metric": "M1", "dims": dims("Queja", "Phone"), "status": "proposed", "delivery": {"status": "delivered", "proposal_id": "prop-1"}},
        {"finding_id": "finding_2", "evidence_ref": "ev:def", "metric": "M1", "dims": dims("Queja", "App"), "status": "proposed", "delivery": {"status": "delivered", "proposal_id": "prop-2"}}]});
    std::fs::write(work.join("value-loop").join("job-1.json"), doc.to_string()).unwrap();
}

fn release(kind: &str, release_id: &str, proposal: Option<&str>) -> Trigger {
    Trigger { event_type: kind.into(), release_id: Some(release_id.into()), proposal_id: proposal.map(str::to_string), agent: Some("disputas".into()), event_at: Some("2026-10-05T10:00:00Z".into()) }
}

fn calls(work: &Path) -> usize {
    std::fs::read_to_string(work.join("calls.log")).map_or(0, |s| s.lines().count())
}

const SUCCESS_WORDS: [&str; 6] = ["mejor", "melhor", "exito", "sucesso", "funcion", "caus"];

#[test]
fn an_improved_verdict_becomes_a_card_with_numbers_controls_caveats_and_es_pt_text() {
    let work = temp("improved");
    announced(&work);
    let s = step(&work, &canned(&work, canned_cell("improved", None, Some(-5.2), Some([-7.0, -3.4]))), &[]);
    let out = s.run(&release("release.published", "rel-1", Some("prop-1"))).unwrap();
    assert_eq!(out["state"], "measured");
    let c = &out["cards"][0];
    assert_eq!(c["verdict"], "improved");
    assert_eq!(c["success_claimed"], true);
    assert_eq!(out["success_claimed"], true);
    assert_eq!((c["effect_pp"].as_f64(), c["n_pre"].as_u64(), c["n_post"].as_u64()), (Some(-5.2), Some(6000), Some(6000)));
    assert_eq!(c["interval_pp"], json!([-7.0, -3.4]));
    assert_eq!(c["controls"]["dimension"], "reason_category");
    assert_eq!(c["controls"]["cells"].as_array().unwrap().len(), 2);
    assert_eq!(c["period_kind"], "pseudo_release_historical");
    let caveats = c["caveats"].to_string();
    for needle in ["association, not cause", "PSEUDO-release", "independent"] {
        assert!(caveats.contains(needle), "{needle}: {caveats}");
    }
    let (es, pt) = (c["dossier"]["es"].as_str().unwrap(), c["dossier"]["pt"].as_str().unwrap());
    assert!(es.contains("asociacion") && es.contains("no una prueba de causa") && es.contains("pseudo-lanzamiento"), "{es}");
    assert!(pt.contains("associacao") && pt.contains("nao uma prova de causa") && pt.contains("pseudo-lancamento"), "{pt}");
    // the card is on the finding record, and on disk
    let rec: Value = serde_json::from_str(&std::fs::read_to_string(work.join("value-loop/job-1.json")).unwrap()).unwrap();
    assert_eq!(rec["findings"][0]["outcome_card"]["verdict"], "improved");
    assert!(rec["findings"][1]["outcome_card"].is_null(), "the other finding is untouched");
    assert!(work.join("outcome/rel-1.json").is_file());
}

#[test]
fn only_the_estimator_saying_improved_ever_produces_a_success_claim() {
    for (status, reason, effect, interval) in [
        ("no_detectable_change", None, Some(-0.4), Some([-1.5, 0.7])),
        ("worsened", None, Some(3.1), Some([1.2, 5.0])),
        ("inconclusive", Some("underpowered_minimum_support"), None, None),
        ("inconclusive", Some("half_directions_or_intervals_disagree"), Some(-1.0), Some([-4.0, 2.0])),
    ] {
        let work = temp(&format!("nosuccess-{status}-{}", reason.unwrap_or("x")));
        announced(&work);
        let s = step(&work, &canned(&work, canned_cell(status, reason, effect, interval)), &[]);
        let out = s.run(&release("release.published", "rel-2", Some("prop-1"))).unwrap();
        let c = &out["cards"][0];
        assert_eq!(c["verdict"], status);
        assert_eq!(c["success_claimed"], false, "{status}");
        assert_eq!(out["success_claimed"], false);
        let text = format!("{} {}", c["dossier"]["es"], c["dossier"]["pt"]).to_lowercase();
        if status != "worsened" {
            for w in SUCCESS_WORDS {
                assert!(!text.contains(w), "{status}: success word {w:?} in {text}");
            }
        }
        if reason == Some("underpowered_minimum_support") {
            assert_eq!(c["underpowered"], true);
            assert!(c["power_note"].as_str().unwrap().contains("underpowered"));
            assert!(c["caveats"].to_string().contains("valid and expected"));
        }
    }
}

#[test]
fn an_improved_claim_the_interval_does_not_support_is_downgraded_to_inconclusive() {
    let d: BTreeMap<String, String> = [("reason_category", "Queja"), ("channel", "Phone")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let wrap = |cell: Value| json!({"contract": "pulso.outcome.v1", "cells": [cell]});
    for (effect, interval) in [(Some(-5.0), Some([-7.0, 0.5])), (Some(2.0), Some([-9.0, -1.0])), (None, None), (Some(-5.0), None)] {
        let v = parse_verdict(&wrap(canned_cell("improved", None, effect, interval)), "M1", &d);
        assert_eq!(v.status, "inconclusive", "{effect:?} {interval:?}");
        assert_eq!(v.downgraded_from.as_deref(), Some("improved"));
    }
    let v = parse_verdict(&wrap(canned_cell("worsened", None, Some(-2.0), Some([-3.0, -1.0]))), "M1", &d);
    assert_eq!(v.status, "inconclusive", "a worsened claim needs a positive interval too");
    // vocabulary and contract are closed
    assert_eq!(parse_verdict(&wrap(canned_cell("caused_improvement", None, Some(-5.0), Some([-7.0, -3.0]))), "M1", &d).reason.as_deref(), Some("estimator_status_outside_vocabulary"));
    assert_eq!(parse_verdict(&json!({"contract": "other", "cells": []}), "M1", &d).reason.as_deref(), Some("estimator_contract_mismatch"));
    assert_eq!(parse_verdict(&json!({"contract": "pulso.outcome.v1", "cells": []}), "M1", &d).reason.as_deref(), Some("treated_cell_not_in_estimator_output"));
}

#[test]
fn the_table_has_the_treated_cell_and_its_sibling_controls_with_pre_months_before_and_post_months_after_the_release() {
    let work = temp("table");
    announced(&work);
    let s = step(&work, &canned(&work, canned_cell("inconclusive", Some("underpowered_minimum_support"), None, None)), &[]);
    s.run(&release("release.published", "rel-3", Some("prop-1"))).unwrap();
    let rows: Vec<Value> = std::fs::read_to_string(work.join("outcome/rel-3.0.cells.ndjson")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    // 3 cells x 2 halves x (3 pre + 3 post) months
    assert_eq!(rows.len(), 36);
    let periods: std::collections::BTreeSet<&str> = rows.iter().map(|r| r["period"].as_str().unwrap()).collect();
    assert_eq!(periods.into_iter().collect::<Vec<_>>(), ["2024-04", "2024-05", "2024-06", "2024-08", "2024-09", "2024-10"], "the release month 2024-07 is in neither window");
    assert!(rows.iter().all(|r| r["metric"] == "M1" && r["dims"]["channel"] == "Phone"), "same channel, same metric only: no App cells, no M2");
    let reasons: std::collections::BTreeSet<&str> = rows.iter().map(|r| r["dims"]["reason_category"].as_str().unwrap()).collect();
    assert_eq!(reasons.into_iter().collect::<Vec<_>>(), ["Comercial", "Queja", "Tecnico"]);
    assert!(rows.iter().all(|r| r.as_object().unwrap().len() == 6), "aggregate rows only");
    let t: Value = serde_json::from_str(&std::fs::read_to_string(work.join("outcome/rel-3.0.treated.json")).unwrap()).unwrap();
    assert_eq!(t, json!({"metric": "M1", "dims": dims("Queja", "Phone")}));
    // the estimator got the contract's arguments
    let log: Value = serde_json::from_str(std::fs::read_to_string(work.join("calls.log")).unwrap().lines().next().unwrap()).unwrap();
    assert_eq!((log["release_date"].as_str(), log["window_months"].as_u64()), (Some("2024-07"), Some(3)));
}

#[test]
fn post_months_come_from_the_post_tables_when_they_are_given() {
    let pre: Vec<Value> = (1..=12).map(|m| json!({"metric": "M1", "dims": dims("Queja", "Phone"), "half": "discovery", "period": format!("2024-{m:02}"), "numerator": 1, "denominator": 100})).collect();
    let post: Vec<Value> = (1..=12).map(|m| json!({"metric": "M1", "dims": dims("Queja", "Phone"), "half": "discovery", "period": format!("2024-{m:02}"), "numerator": 2, "denominator": 200})).collect();
    let d: BTreeMap<String, String> = [("reason_category", "Queja"), ("channel", "Phone")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let rows = build_table(&pre, &post, "M1", &d, "2024-07", 2);
    let by: Vec<(&str, u64)> = rows.iter().map(|r| (r["period"].as_str().unwrap(), r["numerator"].as_u64().unwrap())).collect();
    assert_eq!(by, [("2024-05", 1), ("2024-06", 1), ("2024-08", 2), ("2024-09", 2)]);
    // the year boundary
    let rows = build_table(&pre, &post, "M1", &d, "2024-01", 1);
    assert_eq!(rows.len(), 1, "2023-12 is not in the tables: only the post month is there");
}

#[test]
fn a_second_event_for_the_same_release_makes_no_second_estimator_call() {
    let work = temp("idem");
    announced(&work);
    let s = step(&work, &canned(&work, canned_cell("improved", None, Some(-5.2), Some([-7.0, -3.4]))), &[]);
    let a = s.run(&release("release.published", "rel-4", Some("prop-1"))).unwrap();
    assert_eq!((a["replay"].clone(), calls(&work)), (json!(false), 1));
    let b = s.run(&release("release.published", "rel-4", Some("prop-1"))).unwrap();
    let c = s.run(&release("release.promoted", "rel-4", Some("prop-1"))).unwrap();
    assert_eq!((b["replay"].clone(), c["replay"].clone(), calls(&work)), (json!(true), json!(true), 1), "per release id, whatever the event");
    assert_eq!(b["cards"], a["cards"]);
    let r = s.run(&release("release.revoked", "rel-4", Some("prop-1"))).unwrap();
    assert_eq!((r["revoked"].clone(), calls(&work)), (json!(true), 1), "a revoke measures nothing");
    let other = s.run(&release("release.published", "rel-5", Some("prop-1"))).unwrap();
    assert_eq!((other["replay"].clone(), calls(&work)), (json!(false), 2), "another release id is measured");
}

#[test]
fn a_revoked_release_that_was_never_measured_is_recorded_not_measured() {
    let work = temp("revoked");
    announced(&work);
    let s = step(&work, &canned(&work, canned_cell("improved", None, Some(-5.2), Some([-7.0, -3.4]))), &[]);
    let r = s.run(&release("release.revoked", "rel-6", Some("prop-1"))).unwrap();
    assert_eq!((r["state"].clone(), r["success_claimed"].clone(), calls(&work)), (json!("revoked_not_measured"), json!(false), 0));
}

#[test]
fn a_proposal_with_no_announced_finding_is_unlinked_and_the_estimator_is_not_called() {
    let work = temp("unlinked");
    announced(&work);
    let s = step(&work, &canned(&work, canned_cell("improved", None, Some(-5.2), Some([-7.0, -3.4]))), &[]);
    let a = s.run(&release("release.published", "rel-7", Some("prop-unknown"))).unwrap();
    assert_eq!((a["state"].clone(), a["reason"].clone()), (json!("unlinked"), json!("no_finding_announced_for_proposal")));
    let b = s.run(&release("release.published", "rel-8", None)).unwrap();
    assert_eq!(b["reason"], "proposal_id_missing");
    assert_eq!(calls(&work), 0);
    assert_eq!((a["success_claimed"].clone(), b["success_claimed"].clone()), (json!(false), json!(false)));
}

#[test]
fn estimator_failures_are_closed_inconclusive_cards_never_a_verdict_or_a_job_failure() {
    for (tag, cmd) in [
        ("crash", format!("python {} --canned x --fail 3", scripts().join("outcome_double.py").display())),
        ("missing", "no-such-estimator-binary-out1".to_string()),
        ("notjson", "python -c print('hello')".to_string()),
    ] {
        let work = temp(tag);
        announced(&work);
        let s = step(&work, &canned(&work, canned_cell("improved", None, Some(-5.2), Some([-7.0, -3.4]))), &[("PULSO_OUTCOME_CMD", cmd)]);
        let out = s.run(&release("release.published", "rel-9", Some("prop-1"))).unwrap();
        let c = &out["cards"][0];
        assert_eq!((c["verdict"].clone(), c["success_claimed"].clone()), (json!("inconclusive"), json!(false)), "{tag}");
        assert!(c["reason"].as_str().unwrap().starts_with("estimator_failed:"), "{tag}: {c}");
    }
}

#[test]
fn a_timeout_kills_the_estimator() {
    let work = temp("timeout");
    announced(&work);
    let sleeper = work.join("sleeper.py");
    std::fs::write(&sleeper, "import time
time.sleep(30)
").unwrap();
    let s = step(&work, &canned(&work, canned_cell("improved", None, Some(-5.2), Some([-7.0, -3.4]))), &[("PULSO_OUTCOME_CMD", format!("python {}", sleeper.display())), ("PULSO_OUTCOME_TIMEOUT_SECS", "1".into())]);
    let t = std::time::Instant::now();
    let out = s.run(&release("release.published", "rel-10", Some("prop-1"))).unwrap();
    assert!(t.elapsed().as_secs() < 15);
    assert_eq!(out["cards"][0]["reason"], "estimator_failed:estimator_timeout");
}

#[test]
fn a_live_release_takes_its_period_from_the_event_and_says_so() {
    let work = temp("live");
    announced(&work);
    let c = canned(&work, canned_cell("inconclusive", Some("incomplete_global_month_window"), None, None));
    // no pseudo-release: the period is the month of the event (2026-10), which the tables do not cover
    let pre = work.join("cells.ndjson");
    cells(&pre);
    let cmd = format!("python {} --canned {} --log {}", scripts().join("outcome_double.py").display(), c.display(), work.join("calls.log").display());
    let m: HashMap<String, String> = [("PULSO_OUTCOME_PRE_CELLS", pre.display().to_string()), ("PULSO_OUTCOME_CMD", cmd)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    let s = OutcomeStep::from_lookup(&|k| m.get(k).cloned(), Some(&work)).unwrap().unwrap();
    let out = s.run(&release("release.published", "rel-11", Some("prop-1"))).unwrap();
    let card = &out["cards"][0];
    assert_eq!(card["period_kind"], "live_release_event");
    assert_eq!(card["window"]["release_period"], "2026-10");
    assert!(!card["caveats"].to_string().contains("PSEUDO"));
    assert_eq!(card["verdict"], "inconclusive");
}

#[test]
fn configuration_is_validated_and_off_by_default() {
    let w = temp("cfg");
    let get = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(a, _)| *a == k).map(|(_, v)| v.to_string());
    assert!(OutcomeStep::from_lookup(&get(&[]), Some(&w)).unwrap().is_none());
    assert!(OutcomeStep::from_lookup(&get(&[("PULSO_CELLS_NDJSON", "c.ndjson")]), Some(&w)).unwrap().is_some(), "the value-loop tables are the default pre tables");
    assert!(OutcomeStep::from_lookup(&get(&[("PULSO_CELLS_NDJSON", "c.ndjson"), ("PULSO_OUTCOME", "off")]), Some(&w)).unwrap().is_none());
    assert!(OutcomeStep::from_lookup(&get(&[("PULSO_CELLS_NDJSON", "c"), ("PULSO_OUTCOME_PSEUDO_RELEASE", "2024-13")]), Some(&w)).err().unwrap().contains("PULSO_OUTCOME_PSEUDO_RELEASE"));
    assert!(OutcomeStep::from_lookup(&get(&[("PULSO_CELLS_NDJSON", "c"), ("PULSO_OUTCOME_WINDOW_MONTHS", "0")]), Some(&w)).err().unwrap().contains("PULSO_OUTCOME_WINDOW_MONTHS"));
}

#[test]
fn the_trigger_view_is_an_outcome_release_event_or_nothing() {
    let v = json!({"kind": "outcome", "event_type": "release.published", "subject": {"release_id": "rel-1", "proposal_id": "prop-1", "agent": "disputas"}, "event_at": "2026-10-05T10:00:00Z"});
    let t = Trigger::from_view(&v).unwrap();
    assert_eq!((t.release_id.as_deref(), t.proposal_id.as_deref(), t.agent.as_deref()), (Some("rel-1"), Some("prop-1"), Some("disputas")));
    assert!(Trigger::from_view(&json!({"kind": "outcome", "event_type": "run.closed", "subject": {}})).is_none());
    assert!(Trigger::from_view(&json!({"kind": "explicit", "event_type": "release.published", "subject": {}})).is_none());
}

fn cfg(work: &Path) -> RunConfig {
    let m: HashMap<String, String> = [("PULSO_STORAGE", "memory"), ("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_ADAPTER", "product-sqlite"), ("PULSO_SOURCE_ID", "platform:sim"), ("PULSO_WORK_DIR", work.to_str().unwrap()), ("PULSO_SOURCE_SQLITE", "unused.db")]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    RunConfig::from_lookup(&|k| m.get(k).cloned()).unwrap()
}

/// The runner reads the trigger back from the automation audit run, runs the outcome step (not the value loop), keeps the card in the job
/// summary and in the console store, and a second job for the same release id calls the estimator no more.
#[test]
fn the_runner_executes_a_release_trigger_job_as_the_outcome_step_with_idempotency_per_release_id() {
    let work = temp("runner");
    announced(&work);
    let s = step(&work, &canned(&work, canned_cell("inconclusive", Some("underpowered_minimum_support"), None, None)), &[]);
    let store = Arc::new(Store::memory());
    let audit = |key: &str, release: &str| {
        let view = json!({"trigger_key": key, "kind": "outcome", "event_type": "release.published", "ref": release, "subject": {"release_id": release, "proposal_id": "prop-1"}, "event_at": "2026-10-05T10:00:00Z"});
        store.emit("automation-audit", NewEvent::new("run_started", "run", "automation-audit", json!({"title": "audit", "state": "completed", "origin": "system"}))).ok();
        store.emit("automation-audit", NewEvent::new("automation_trigger_received", "automation", key, view)).unwrap();
    };
    let k2 = format!("sha256:{}", "f".repeat(64));
    audit(KEY, "rel-12");
    audit(&k2, "rel-12");
    let runner = EngineRunner::new(&cfg(&work), &work, Path::new(env!("CARGO_BIN_EXE_pulso-synth-runner")), store.clone()).unwrap().with_outcome(Some(Arc::new(s)));
    let repo = Arc::new(MemRepo::new());
    let mut jobs = vec![];
    for key in [KEY, k2.as_str()] {
        let id = repo.admit_keyed(T, &format!("trigger:{key}")).unwrap();
        let c = repo.claim_next(T, "w", 100, 60).unwrap().unwrap();
        assert_eq!(c.job, id);
        let stop = StopToken::new();
        let ctx = JobCtx { repo: repo.as_ref(), tenant: T, worker: "w", stop: &stop, now: 100, lease_seconds: 60 };
        runner.run(&c, &ctx).unwrap();
        jobs.push(repo.output(T, &id, 0).unwrap().unwrap());
    }
    let first: Value = serde_json::from_str(&jobs[0]).unwrap();
    assert_eq!(first["kind"], "outcome");
    assert_eq!(first["outcome"]["cards"][0]["verdict"], "inconclusive");
    assert!(first.get("value_loop").is_none(), "the release trigger does not run the value loop");
    let second: Value = serde_json::from_str(&jobs[1]).unwrap();
    assert_eq!(second["outcome"]["replay"], true);
    assert_eq!(calls(&work), 1, "one estimator call for the release id");
    let run = store.runs().into_iter().find(|r| r.starts_with("outcome-")).expect("an outcome run in the console store");
    let title = store.state(&run).unwrap()["run"]["title"].as_str().unwrap().to_string();
    assert!(title.contains("not cause") && title.contains("success claimed: false"), "{title}");
}

#[test]
fn a_release_trigger_without_the_outcome_step_is_recorded_as_skipped() {
    let work = temp("skipped");
    let store = Arc::new(Store::memory());
    let view = json!({"trigger_key": KEY, "kind": "outcome", "event_type": "release.published", "ref": "r", "subject": {"release_id": "rel-13"}});
    store.emit("automation-audit", NewEvent::new("run_started", "run", "automation-audit", json!({"title": "audit", "state": "completed", "origin": "system"}))).ok();
    store.emit("automation-audit", NewEvent::new("automation_trigger_received", "automation", KEY, view)).unwrap();
    let runner = EngineRunner::new(&cfg(&work), &work, Path::new(env!("CARGO_BIN_EXE_pulso-synth-runner")), store).unwrap();
    let repo = Arc::new(MemRepo::new());
    let id = repo.admit_keyed(T, &format!("trigger:{KEY}")).unwrap();
    let c = repo.claim_next(T, "w", 100, 60).unwrap().unwrap();
    let stop = StopToken::new();
    runner.run(&c, &JobCtx { repo: repo.as_ref(), tenant: T, worker: "w", stop: &stop, now: 100, lease_seconds: 60 }).unwrap();
    let out: Value = serde_json::from_str(&repo.output(T, &id, 0).unwrap().unwrap()).unwrap();
    assert!(out["outcome"]["skipped"].as_str().unwrap().contains("not configured"));
}
