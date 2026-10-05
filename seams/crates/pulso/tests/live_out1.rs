//! OUT1 LIVE (ignored): the outcome step with a REAL estimator on REAL bank cell tables (aggregates only).
//!
//! Needs, outside git: `OUT1_LIVE_SCRATCH` = a directory holding a copy of Codex's estimator (`scripts/aggregate/outcome/*.py`,
//! `docs/data/opbench/opbench.py`, with `__init__.py` files) and `cells.ndjson` (`scripts/aggregate/bank_cells.py` output);
//! `OUT1_LIVE_OUT` = where the cards are written. Run: `cargo test -j 1 -p pulso --test live_out1 -- --ignored --nocapture`.
//! The estimator is called through the repository's reference adapter `scripts/out1/outcome_cli_adapter.py`.
use pulso::run::outcome::{OutcomeStep, Trigger};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn env(k: &str) -> PathBuf {
    PathBuf::from(std::env::var(k).unwrap_or_else(|_| panic!("{k} is not set")))
}

fn finding_record(work: &Path, metric: &str, dims: Value, proposal: &str) {
    std::fs::create_dir_all(work.join("value-loop")).unwrap();
    let doc = json!({"contract": "value-loop/b3-0", "findings": [{"finding_id": "finding_1", "evidence_ref": "ev:live-out1", "metric": metric, "dims": dims, "status": "proposed", "delivery": {"status": "delivered", "proposal_id": proposal}}]});
    std::fs::write(work.join("value-loop/job-1.json"), doc.to_string()).unwrap();
}

fn step(work: &Path, scratch: &Path, pre: &Path, post: Option<&Path>, pseudo: &str, label: Option<&str>) -> OutcomeStep {
    let adapter = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/out1/outcome_cli_adapter.py");
    let mut m: HashMap<String, String> = HashMap::new();
    m.insert("PULSO_OUTCOME_PRE_CELLS".into(), pre.display().to_string());
    if let Some(p) = post {
        m.insert("PULSO_OUTCOME_POST_CELLS".into(), p.display().to_string());
    }
    m.insert("PULSO_OUTCOME_PSEUDO_RELEASE".into(), pseudo.into());
    if let Some(l) = label {
        m.insert("PULSO_OUTCOME_DATA_LABEL".into(), l.into());
    }
    m.insert("PULSO_OUTCOME_CMD".into(), format!("python {}", adapter.display()));
    m.insert("PULSO_OUTCOME_CWD".into(), scratch.display().to_string());
    OutcomeStep::from_lookup(&|k| m.get(k).cloned(), Some(work)).unwrap().unwrap()
}

fn trig(release: &str, proposal: &str) -> Trigger {
    Trigger { event_type: "release.published".into(), release_id: Some(release.into()), proposal_id: Some(proposal.into()), agent: Some("disputas".into()), event_at: Some("2026-10-05T10:00:00Z".into()) }
}

#[test]
#[ignore = "needs OUT1_LIVE_SCRATCH (estimator copy + cells.ndjson) and OUT1_LIVE_OUT"]
fn real_estimator_on_real_bank_cells_and_on_a_planted_synthetic_effect() {
    let (scratch, out) = (env("OUT1_LIVE_SCRATCH"), env("OUT1_LIVE_OUT"));
    let cells = scratch.join("cells.ndjson");
    let dims = json!({"reason_category": "Queja", "channel": "Phone"});
    let mut summary = vec![];

    // (1) REAL cells, Queja/Phone, M1, several PSEUDO-releases (no release happened: the post months are just the months after the date).
    for (i, release) in ["2024-06", "2024-12", "2025-06", "2025-12", "2026-02"].iter().enumerate() {
        let work = out.join(format!("real-{release}"));
        let _ = std::fs::remove_dir_all(&work);
        finding_record(&work, "M1", dims.clone(), "prop-live");
        let s = step(&work, &scratch, &cells, None, release, None);
        let doc = s.run(&trig(&format!("pseudo-{release}"), "prop-live")).unwrap();
        let c = &doc["cards"][0];
        println!("REAL pseudo-release {release}: verdict={} reason={} effect_pp={} interval={} n={}/{} success_claimed={}", c["verdict"], c["reason"], c["effect_pp"], c["interval_pp"], c["n_pre"], c["n_post"], c["success_claimed"]);
        if i == 2 {
            println!("--- full card (real, {release}) ---\n{}", serde_json::to_string_pretty(&doc).unwrap());
        }
        assert_eq!(c["period_kind"], "pseudo_release_historical");
        assert_ne!(c["verdict"], "improved", "a stationary pseudo-release must not be called an improvement ({release})");
        summary.push((release.to_string(), c["verdict"].clone()));
    }

    // (2) SYNTHETIC planted effect: the same real tables, but the post tables have the treated cell's numerator cut by 8 pp of its
    // denominator from the release on. LABELLED SYNTHETIC: this shows the `improved` path, it is not a result about any release.
    let rows: Vec<Value> = std::fs::read_to_string(&cells).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let release = "2025-06";
    let planted: String = rows
        .iter()
        .map(|r| {
            let mut r = r.clone();
            if r["metric"] == "M1" && r["dims"] == dims && r["period"].as_str().unwrap() > release {
                let den = r["denominator"].as_i64().unwrap();
                let num = r["numerator"].as_i64().unwrap();
                r["numerator"] = json!((num - (den * 8 + 50) / 100).max(0));
            }
            format!("{r}\n")
        })
        .collect();
    let post = out.join("synthetic-post-cells.ndjson");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(&post, planted).unwrap();
    let work = out.join("synthetic-planted");
    let _ = std::fs::remove_dir_all(&work);
    finding_record(&work, "M1", dims.clone(), "prop-synth");
    let s = step(&work, &scratch, &cells, Some(&post), release, Some("synthetic-planted-effect"));
    let doc = s.run(&trig("synthetic-planted-8pp", "prop-synth")).unwrap();
    let c = &doc["cards"][0];
    println!("SYNTHETIC planted -8pp after {release}: verdict={} effect_pp={} interval={} success_claimed={}", c["verdict"], c["effect_pp"], c["interval_pp"], c["success_claimed"]);
    println!("--- full card (SYNTHETIC planted effect) ---\n{}", serde_json::to_string_pretty(&doc).unwrap());
    assert_eq!(c["verdict"], "improved");
    assert_eq!(c["success_claimed"], true);
    assert!(c["table"]["post_from"].as_str().unwrap().contains("synthetic-post-cells"));
    println!("SUMMARY real pseudo-releases: {summary:?}");
}
