//! STP1 intent (validation) step (`semantics: claude-standin`).
//! Resolves `recompute_ref` (`recompute:<id>@<rev>`) to `$STEPS_RECOMPUTE_DIR/<id>.json` (a recompute
//! step output) and derives the verdict. Only the `denominator` check is evaluated (from the
//! recompute match); every other requested check is reported `not_applicable`, never invented.
//! Independence: scout and verifier actors must differ.
use crate::StepError;
use crate::sensor::json::Json;
use crate::sensor::{env_path, envelope, json, ref_id, valid_id};

const CHECKS: [&str; 8] = [
    "schema_coverage",
    "denominator",
    "temporality",
    "mix",
    "duplicates",
    "multiple_testing",
    "window_sensitivity",
    "counterexamples",
];

pub fn run(input: &str) -> Result<String, StepError> {
    let v = json::parse(input)?;
    let (run_id, data_class) = envelope(&v, "validation")?;
    let inv = |m: &str| StepError::Invalid(m.to_string());
    let sid = v.get("signal_id").and_then(Json::as_str).filter(|s| valid_id(s)).ok_or_else(|| inv("signal_id"))?;
    let rid = ref_id(v.get("recompute_ref").and_then(Json::as_str).ok_or_else(|| inv("recompute_ref"))?)?;
    let scout = v.get("scout_actor").and_then(Json::as_str).filter(|s| valid_id(s)).ok_or_else(|| inv("scout_actor"))?;
    let verifier = v.get("verifier_actor").and_then(Json::as_str).filter(|s| valid_id(s)).ok_or_else(|| inv("verifier_actor"))?;
    if scout == verifier {
        return Err(inv("scout_actor and verifier_actor must differ"));
    }
    let checks: Vec<&str> = v
        .get("checks")
        .and_then(Json::as_arr)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| inv("checks"))?
        .iter()
        .map(|c| c.as_str().filter(|s| CHECKS.contains(s)).ok_or_else(|| inv("unknown check")))
        .collect::<Result<_, _>>()?;

    // The recompute row for this signal; absent file or row means no evidence: inconclusive.
    let path = env_path("STEPS_RECOMPUTE_DIR")?.join(format!("{rid}.json"));
    let row = match std::fs::read_to_string(&path) {
        Ok(t) => json::parse(&t)?
            .get("recomputes")
            .and_then(Json::as_arr)
            .and_then(|a| a.iter().find(|r| r.get("signal_id").and_then(Json::as_str) == Some(sid)).cloned()),
        Err(_) => None,
    };
    let matched = match row.as_ref().and_then(|r| r.get("match")) {
        Some(Json::Bool(b)) => Some(*b),
        _ => None,
    };
    let verdict = match matched {
        Some(true) => "corroborated",
        Some(false) => "refuted",
        None => "inconclusive",
    };
    let results: Vec<Json> = checks
        .iter()
        .map(|c| {
            let status = match (*c, matched) {
                ("denominator", Some(true)) => "pass",
                ("denominator", Some(false)) => "fail",
                _ => "not_applicable",
            };
            Json::obj(vec![("check", Json::s(c)), ("status", Json::s(status))])
        })
        .collect();
    let evidence: Vec<Json> =
        row.iter().filter_map(|r| r.get("evidence_ref").and_then(Json::as_str)).map(Json::s).collect();
    Ok(Json::obj(vec![
        ("contract_version", Json::s("engine-steps/0")),
        ("step", Json::s("validation")),
        ("run_id", Json::s(&run_id)),
        ("data_class", Json::s(&data_class)),
        ("signal_id", Json::s(sid)),
        ("verdict", Json::s(verdict)),
        ("check_results", Json::Arr(results)),
        ("verifier_actor", Json::s(verifier)),
        ("evidence_refs", Json::Arr(evidence)),
    ])
    .write())
}
