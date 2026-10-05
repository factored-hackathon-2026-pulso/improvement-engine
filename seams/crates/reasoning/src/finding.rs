//! A corroborated cells signal as the reasoning roles see it, and its treated evidence rows.
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Where the aggregate came from. Decides the data class and whether the opt-in flag is needed for a real hosted model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Invented numbers (demo / tests). A real model may see them without any flag.
    Synthetic,
    /// Treated cell aggregates of the bank dataset.
    BankTreated,
    /// Treated aggregates derived from E0.
    E0Treated,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Synthetic => "synthetic",
            Source::BankTreated => "bank_treated",
            Source::E0Treated => "e0_treated",
        }
    }
    pub fn parse(s: &str) -> Option<Source> {
        match s {
            "synthetic" => Some(Source::Synthetic),
            "bank_treated" | "bank" => Some(Source::BankTreated),
            "e0_treated" | "e0" => Some(Source::E0Treated),
            _ => None,
        }
    }
    pub fn derived(self) -> bool {
        self != Source::Synthetic
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub numerator: i64,
    pub denominator: i64,
    pub rate: f64,
    pub baseline_rate: f64,
    pub diff: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// `finding_<n>`: a system-shaped id (the TPS scanner accepts it without registration).
    pub id: String,
    pub metric: String,
    pub dims: BTreeMap<String, String>,
    pub direction: String,
    pub discovery: Stage,
    pub holdout: Stage,
    pub p_adj: Option<f64>,
    pub r2: Option<String>,
    pub depends_on: Option<String>,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skipped {
    pub index: usize,
    pub metric: String,
    pub status: String,
    pub reason: String,
}

pub fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn stage(v: &Value) -> Option<Stage> {
    Some(Stage {
        numerator: v["numerator"].as_i64()?,
        denominator: v["denominator"].as_i64()?,
        rate: v["rate"].as_f64()?,
        baseline_rate: v["baseline_rate"].as_f64()?,
        diff: v["diff"].as_f64()?,
    })
}

impl Finding {
    /// Every `corroborated` signal of a `steps_cli cells` report becomes a finding; the others are listed as skipped with the
    /// sensor's own status and reason (the reasoning roles never see a candidate, refuted or uncertain signal).
    pub fn from_report(report: &Value, source: Source) -> Result<(Vec<Finding>, Vec<Skipped>), String> {
        let signals = report["signals"].as_array().ok_or("the cells report has no signals list")?;
        let (mut found, mut skipped) = (vec![], vec![]);
        for (i, s) in signals.iter().enumerate() {
            let status = s["status"].as_str().unwrap_or("").to_string();
            let metric = s["metric"].as_str().unwrap_or("").to_string();
            // A `level_risk` signal is a risk LEVEL against a threshold, not a vs-rest contrast: the contrast roles never see it.
            if s["type"].as_str() == Some("level_risk") {
                skipped.push(Skipped { index: i, metric, status: "level_risk".to_string(), reason: s["reason"].as_str().unwrap_or("").to_string() });
                continue;
            }
            if status != "corroborated" {
                skipped.push(Skipped { index: i, metric, status, reason: s["reason"].as_str().unwrap_or("").to_string() });
                continue;
            }
            let (Some(discovery), Some(holdout)) = (stage(&s["discovery"]), stage(&s["holdout"])) else {
                return Err(format!("signal {i} is corroborated but has no discovery or holdout stage"));
            };
            let dims = s["dims"].as_object().ok_or("signal without dims")?.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
            found.push(Finding {
                id: format!("finding_{}", i + 1),
                metric,
                dims,
                direction: s["direction"].as_str().unwrap_or("").to_string(),
                discovery,
                holdout,
                p_adj: s["p_adj"].as_f64(),
                r2: s["r2"]["status"].as_str().map(str::to_string),
                depends_on: s["depends_on"].as_str().map(str::to_string),
                source,
            });
        }
        Ok((found, skipped))
    }

    pub fn metric_token(&self) -> String {
        self.metric.to_ascii_lowercase()
    }

    /// Opaque evidence ref (`ev_` + 16 hex) over the exact numbers the model is shown. Resolves by recomputation.
    pub fn evidence_ref(&self) -> String {
        let dims: Vec<String> = self.dims.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let raw = format!("{}|{}|{}/{}|{}/{}", self.metric, dims.join(","), self.discovery.numerator, self.discovery.denominator, self.holdout.numerator, self.holdout.denominator);
        format!("ev_{}", &steps::compile::sha256_hex(raw.as_bytes())[..16])
    }

    /// The TPS observation: the two cell rows (`w1` discovery, `w2` holdout) as k-anonymous aggregate rows.
    pub fn observations(&self) -> Value {
        let ev = self.evidence_ref();
        let row = |w: &str, s: &Stage| json!({"metric_id": self.metric_token(), "window_id": w, "count": s.denominator, "rate": round2(s.rate), "evidence_ref": ev});
        json!([{"tool": "pulso/cell_rows@1.0.0", "args": {"metric_id": self.metric_token()}, "status": "ok", "error": null,
                "result": {"rows": [row("w1", &self.discovery), row("w2", &self.holdout)]}}])
    }

    /// Treated, opaque description of the finding (every string is an enum, a closed-vocabulary label or an id).
    pub fn inputs(&self) -> Value {
        let dims: serde_json::Map<String, Value> = self.dims.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        json!({
            "finding_id": self.id, "metric_id": self.metric_token(), "dims": dims, "direction": self.direction, "claim_kind": "association",
            "source": self.source.as_str(),
            "baseline_rate_discovery": round2(self.discovery.baseline_rate), "baseline_rate_holdout": round2(self.holdout.baseline_rate),
            "replication": "holdout_replicated", "r2_status": self.r2.clone().unwrap_or_else(|| "not_evaluated".into()),
            "depends_on": self.depends_on.as_ref().map_or_else(|| "none".to_string(), |d| d.to_ascii_lowercase()),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub id: &'static str,
    /// `pass | fail | na`
    pub result: &'static str,
}

/// Deterministic checks the model can never overrule: the recompute of the claimed rate from the evidence numbers and the
/// structural conditions of the sensor (k-anonymity, replication, effect size, dependency).
pub fn deterministic_checks(f: &Finding, claimed_rate: f64) -> Vec<Check> {
    let ok = |b: bool| if b { "pass" } else { "fail" };
    let observed = round2(f.discovery.numerator as f64 / f.discovery.denominator.max(1) as f64);
    let k = |s: &Stage| s.numerator >= 10 && s.denominator - s.numerator >= 10;
    vec![
        Check { id: "recompute", result: ok((round2(claimed_rate) - observed).abs() < 1e-9) },
        Check { id: "k_anonymity", result: ok(k(&f.discovery) && k(&f.holdout)) },
        Check { id: "replication", result: ok(f.holdout.diff > 0.0 && f.holdout.rate > f.holdout.baseline_rate) },
        Check { id: "effect_size", result: ok(f.discovery.diff >= 0.05 && f.discovery.rate >= 1.25 * f.discovery.baseline_rate) },
        Check { id: "dependency", result: ok(f.depends_on.is_none()) },
        Check { id: "r2_windows", result: match f.r2.as_deref() { Some("replicated") => "pass", Some("not_evaluated") | None => "na", _ => "fail" } },
    ]
}
