//! doubles[] (what is NOT real) derived from the report, same rule as the G1 `generate_doubles`, and the human summary
//! that lists them FIRST (same layout as demo/run-demo0.ps1).
use pulso::doubles::{generate, summary};
use serde_json::json;

fn report() -> serde_json::Value {
    json!({"label": "DEMO-0", "host": "rust", "target": "local", "contract_revision": "engine-run/c2-1", "sha": "0123456789abcdef0123456789abcdef01234567",
        "quality_claims": "forbidden", "gate": {"verdict": "fail", "judge": "claude-gsipy"},
        "steps": [
            {"n": 1, "id": "trigger", "status": "stand-in", "data_class": "generated_sample", "receipt": {"provider": "manual-command"}},
            {"n": 3, "id": "recompute", "status": "real-narrow", "data_class": "generated_sample", "receipt": {"provider": "claude-standin"}},
            {"n": 7, "id": "revision", "status": "not_exercised", "data_class": "generated_sample", "receipt": {"provider": "claude-standin"}},
            {"n": 99, "id": "pretend", "status": "real", "data_class": "generated_sample", "receipt": {"provider": "scripted"}}],
        "ports": [{"port": "core", "provenance": "offline-double(thread10::DoublePort)", "price_source": "n/a"}],
        "overrides": [{"step": "approval", "of": "gate", "verdict": "fail", "by": "human", "label": "human_override", "simulated": true, "reason": "exercise"}]})
}

#[test]
fn every_non_real_step_every_port_and_every_override_is_a_double() {
    let d = generate(&report());
    let parts: Vec<(String, String)> = d.iter().map(|x| (x["part"].as_str().unwrap().into(), x["status"].as_str().unwrap().into())).collect();
    assert!(parts.contains(&("trigger".into(), "stand-in".into())));
    assert!(parts.contains(&("recompute".into(), "real-narrow".into())), "real-narrow is NOT real");
    assert!(parts.contains(&("revision".into(), "not_exercised".into())));
    assert!(parts.contains(&("port.core".into(), "offline-double(thread10::DoublePort)".into())));
    assert!(parts.iter().any(|(p, s)| p == "gate.override" && s.contains("simulated human")));
    assert!(!parts.iter().any(|(p, _)| p == "pretend") || d.iter().any(|x| x["part"] == "pretend" && x["provider"] == "scripted"), "a real step with a non-real provider is still listed");
}

#[test]
fn summary_lists_doubles_before_steps_with_labels() {
    let mut r = report();
    r["doubles"] = json!(generate(&r));
    let s = summary(&r);
    let (d, st) = (s.find("NOT REAL (doubles[]").unwrap(), s.find("STEPS (id, status").unwrap());
    assert!(d < st);
    assert!(s.contains("- trigger: stand-in (provider=manual-command, data_class=generated_sample)"), "{s}");
    assert!(s.contains("3. recompute: real-narrow | generated_sample | claude-standin"), "{s}");
    assert!(s.contains("quality_claims: forbidden; gate judge claude-gsipy, gate verdict fail"), "{s}");
    assert!(s.contains("- override: human_override of gate verdict fail by human (simulated=true): exercise"), "{s}");
}
