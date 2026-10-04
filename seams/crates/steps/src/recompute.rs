//! STP1 verifier recompute step (`semantics: claude-standin`).
//! Resolves `lab_ref` (`lab:<id>@<rev>`) to `$STEPS_LAB_DIR/<id>.json` holding
//! `{"rows":[{"signal_id","evidence_ref","numerator","count"}]}` and recomputes each scout
//! figure from numerator and count (rate at two decimals, round half even, as ed0_lab.rate_of).
use crate::StepError;
use crate::sensor::json::Json;
use crate::sensor::{env_path, envelope, json, ref_id, valid_id};

/// Round-half-even of numerator/count at two decimals, as an integer number of hundredths.
fn hundredths(num: i64, cnt: i64) -> i64 {
    let (n, c) = (num as i128 * 100, cnt as i128);
    let (q, r) = (n / c, n % c);
    let twice = r * 2;
    let up = twice > c || (twice == c && q % 2 == 1);
    (q + i128::from(up)) as i64
}

pub fn run(input: &str) -> Result<String, StepError> {
    let v = json::parse(input)?;
    let (run_id, data_class) = envelope(&v, "recompute")?;
    let inv = |m: &str| StepError::Invalid(m.to_string());
    let lab_id = ref_id(v.get("lab_ref").and_then(Json::as_str).ok_or_else(|| inv("lab_ref"))?)?;
    let ids = v.get("signal_ids").and_then(Json::as_arr).ok_or_else(|| inv("signal_ids"))?;
    let claims = v.get("scout_claims").and_then(Json::as_arr).ok_or_else(|| inv("scout_claims"))?;
    if ids.is_empty() || claims.is_empty() {
        return Err(inv("signal_ids and scout_claims must not be empty"));
    }
    for i in ids {
        if !i.as_str().is_some_and(valid_id) {
            return Err(inv("bad signal_id"));
        }
    }
    let lab_path = env_path("STEPS_LAB_DIR")?.join(format!("{lab_id}.json"));
    let lab_text = std::fs::read_to_string(&lab_path).map_err(|_| StepError::Io("cannot read lab file".into()))?;
    let lab = json::parse(&lab_text)?;
    let rows = lab.get("rows").and_then(Json::as_arr).ok_or_else(|| inv("lab rows"))?;

    let mut out = Vec::new();
    for c in claims {
        let sid = c.get("signal_id").and_then(Json::as_str).filter(|s| valid_id(s)).ok_or_else(|| inv("claim signal_id"))?;
        let claimed = c
            .get("claimed_rate")
            .and_then(Json::as_f64)
            .filter(|r| (0.0..=1.0).contains(r))
            .ok_or_else(|| inv("claimed_rate must be in [0,1]"))?;
        if !ids.iter().any(|i| i.as_str() == Some(sid)) {
            return Err(inv(&format!("claim for {sid} is not in signal_ids")));
        }
        let row = rows
            .iter()
            .find(|r| r.get("signal_id").and_then(Json::as_str) == Some(sid))
            .ok_or_else(|| inv(&format!("signal {sid} not in lab")))?;
        let num = row.get("numerator").and_then(Json::as_i64).filter(|n| *n >= 0).ok_or_else(|| inv("lab numerator"))?;
        let cnt = row.get("count").and_then(Json::as_i64).filter(|n| *n >= 1).ok_or_else(|| inv("lab count must be >= 1"))?;
        let ev = row.get("evidence_ref").and_then(Json::as_str).filter(|s| valid_id(s)).ok_or_else(|| inv("lab evidence_ref"))?;
        let h = hundredths(num, cnt);
        let recomputed = h as f64 / 100.0;
        out.push(Json::obj(vec![
            ("signal_id", Json::s(sid)),
            ("claimed_rate", Json::Float(claimed)),
            ("recomputed_rate", Json::Float(recomputed)),
            ("match", Json::Bool((claimed * 100.0).round() as i64 == h && (claimed - recomputed).abs() < 1e-9)),
            ("evidence_ref", Json::s(ev)),
        ]));
    }
    Ok(Json::obj(vec![
        ("contract_version", Json::s("engine-steps/0")),
        ("step", Json::s("recompute")),
        ("run_id", Json::s(&run_id)),
        ("data_class", Json::s(&data_class)),
        ("recomputes", Json::Arr(out)),
    ])
    .write())
}
