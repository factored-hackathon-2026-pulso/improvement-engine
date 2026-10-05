#![allow(dead_code)]
//! Shared fixtures: a synthetic cells table run through the REAL L1 sensor, hand-built signals for the other mappings, and
//! scripted ports that answer from the request (so every scripted answer also proves the payload is TPS-clean).
use engine::models::{ModelError, ModelPort};
use reasoning::catalog::Catalog;
use reasoning::finding::{Finding, Source};
use reasoning::pipeline::{Opts, Ports};
use reasoning::testkit::{FnPort, menu_of};
use serde_json::{Value, json};
use std::cell::Cell;
use std::rc::Rc;

pub fn cat() -> Catalog {
    Catalog::bundled()
}

/// Synthetic M1 grid (baseline 20%), Tecnico/Phone planted at 45% in both halves. Run through the REAL L1 sensor.
pub fn synthetic_cells_report() -> Value {
    serde_json::from_str(&steps::cells::run(&reasoning::testkit::synthetic_cells_ndjson()).expect("the sensor runs")).expect("sensor output is JSON")
}

pub fn stage(num: i64, den: i64, base: f64) -> Value {
    let rate = num as f64 / den as f64;
    json!({"numerator": num, "denominator": den, "rate": rate, "baseline_rate": base, "diff": rate - base, "p": 0.0})
}

pub fn signal(metric: &str, dims: Value, disc: Value, hold: Value) -> Value {
    json!({"metric": metric, "dims": dims, "status": "corroborated", "reason": "replicated", "direction": "up", "claim": "association", "discovery": disc, "holdout": hold,
           "r2": {"status": "replicated"}, "p_adj": 0.0})
}

pub fn report_of(signals: Vec<Value>) -> Value {
    json!({"semantics": "claude-standin", "cells_explored": 12, "signals": signals, "discards": []})
}

pub fn finding_of(sig: Value, source: Source) -> Finding {
    let (mut f, _) = Finding::from_report(&report_of(vec![sig]), source).unwrap();
    f.remove(0)
}

/// Copilot E1 finding (E0-derived repeated recent-charges lookup): 77% of dispute cases vs a 40% reference.
pub fn copilot_finding(source: Source) -> Finding {
    finding_of(signal("E1", json!({"case_type": "dispute"}), stage(154, 200, 0.40), stage(122, 160, 0.40)), source)
}

pub fn pqr_finding() -> Finding {
    finding_of(signal("M4", json!({"category": "Technical"}), stage(5400, 6000, 0.70), stage(3600, 4000, 0.70)), Source::Synthetic)
}

pub fn tecnico_finding() -> Finding {
    let r = synthetic_cells_report();
    let (f, _) = Finding::from_report(&r, Source::Synthetic).unwrap();
    f.into_iter().find(|x| x.dims.get("reason_category").map(String::as_str) == Some("Tecnico")).expect("the planted cell is corroborated")
}

pub fn scout_ok(f: &Finding, target: &str, mech: &str) -> impl Fn(&engine::models::ModelRequest) -> Result<Value, ModelError> + 'static {
    let claimed = (f.discovery.numerator as f64 / f.discovery.denominator as f64 * 100.0).round() / 100.0;
    let target = target.to_string();
    let mech = mech.to_string();
    move |_| {
        Ok(json!({"opportunity": {"id": "h_1", "target_ref": target, "mechanism_class": mech, "claimed_rate": claimed,
            "hypothesis": "The artifact has no wording for this situation, so the person has to ask again and the contact stays open.",
            "falsifiers": ["The rate is the same in contacts that did receive the new wording."],
            "alternatives": [{"kind": "do_nothing", "why_not": "The gap is replicated and material."}, {"kind": "human_owned", "why_not": "Thresholds and policies are not part of this change."}]}}))
    }
}

pub fn verifier_ok(verdict: &'static str) -> impl Fn(&engine::models::ModelRequest) -> Result<Value, ModelError> + 'static {
    move |_| {
        let checks: Vec<Value> = ["recompute", "replication", "effect_size", "mechanism_fit", "dependency"].iter().map(|id| json!({"id": id, "result": "pass"})).collect();
        Ok(json!({"verdict": verdict, "checks": checks, "rationale": "The claim recomputes and replicates in the holdout."}))
    }
}

pub fn alts() -> Value {
    json!([{"kind": "do_nothing", "why_not": "The effect is replicated."}, {"kind": "other_target", "why_not": "No other artifact carries this wording."}])
}

pub fn ports(scout: FnPort, verifier: FnPort, builder: FnPort) -> Ports {
    Ports::new(Rc::new(scout), Rc::new(verifier), Rc::new(builder))
}

pub fn count_calls(counter: Rc<Cell<u32>>, inner: impl Fn(&engine::models::ModelRequest) -> Result<Value, ModelError> + 'static) -> impl Fn(&engine::models::ModelRequest) -> Result<Value, ModelError> + 'static {
    move |r| {
        counter.set(counter.get() + 1);
        inner(r)
    }
}

pub fn anchor_containing(req: &engine::models::ModelRequest, locale: &str, needle: &str) -> String {
    menu_of(req).into_iter().find(|(id, t)| id.starts_with(&format!("{locale}.")) && t.contains(needle)).unwrap_or_else(|| panic!("no {locale} anchor with {needle:?}")).0
}

pub fn opts() -> Opts {
    Opts::default()
}

pub fn dyn_port(p: FnPort) -> Rc<dyn ModelPort> {
    Rc::new(p)
}
