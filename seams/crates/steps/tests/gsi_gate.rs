//! GSI: two-gate verdict, author is not judge, parity with the reviewed Python reference.
use steps::gate::{canonical_json, run};

const CASES: &str = include_str!("fixtures/gsi/cases.json");
const EXPECTED: &str = include_str!("fixtures/gsi/cases.expected.json");
const SAMPLE_OUT: &str = include_str!("../../../../contracts/engine-steps/samples/valid-gate.out.json");
const SAMPLE_IN: &str = include_str!("../../../../contracts/engine-steps/samples/valid-gate.in.json");

/// Pull one top-level member (raw JSON text) out of a JSON document via the crate's canonical reader.
fn member(doc: &str, key: &str) -> String {
    let c = canonical_json(doc).unwrap();
    let needle = format!("\"{key}\":");
    let start = c.find(&needle).unwrap_or_else(|| panic!("missing {key}")) + needle.len();
    let bytes = c.as_bytes();
    let (mut depth, mut in_str, mut esc, mut end) = (0i32, false, false, start);
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        end = i;
        if in_str {
            if esc { esc = false } else if b == b'\\' { esc = true } else if b == b'"' { in_str = false }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => { if depth == 0 { break } depth -= 1 }
            b',' if depth == 0 => break,
            _ => {}
        }
        end = i + 1;
    }
    c[start..end].to_string()
}

fn cases() -> Vec<(String, String)> {
    // Cases are a JSON array of {name, input}; split by the canonical reader is overkill: use member() on a wrapper.
    let c = canonical_json(CASES).unwrap();
    let mut out = Vec::new();
    let mut rest = c.as_str();
    while let Some(i) = rest.find("{\"input\":") {
        let sub = &rest[i..];
        // find the matching close brace of this element
        let (mut depth, mut in_str, mut esc, mut end) = (0i32, false, false, 0);
        for (j, b) in sub.bytes().enumerate() {
            if in_str { if esc { esc = false } else if b == b'\\' { esc = true } else if b == b'"' { in_str = false } continue }
            match b { b'"' => in_str = true, b'{' | b'[' => depth += 1, b'}' | b']' => { depth -= 1; if depth == 0 { end = j + 1; break } } _ => {} }
        }
        let el = &sub[..end];
        let name = member(el, "name").trim_matches('"').to_string();
        out.push((name, member(el, "input")));
        rest = &sub[end..];
    }
    out
}

fn verdict_of(out: &str) -> String { member(out, "verdict").trim_matches('"').to_string() }

#[test]
fn parity_with_python_reference_on_shared_fixtures() {
    let cs = cases();
    assert!(cs.len() >= 20, "fixtures must be loaded, got {}", cs.len());
    for (name, input) in cs {
        let want = member(EXPECTED, &name);
        match run(&input) {
            Ok(got) => assert_eq!(canonical_json(&got).unwrap(), want, "case {name}"),
            Err(_) => assert_eq!(want, "{\"error\":true}", "case {name} errored but reference did not"),
        }
    }
}

#[test]
fn five_frz0_gate_cases_verdicts() {
    let want = [("fx1_pass", "pass"), ("fx2_fail_improvement_equal", "fail"), ("fx3_fail_safety_closed_early", "fail"),
                ("fx4_case_mismatch", "not_evaluable"), ("fx5_cost_unknown", "not_evaluable")];
    let cs = cases();
    for (n, v) in want {
        let input = &cs.iter().find(|(c, _)| c == n).unwrap().1;
        assert_eq!(verdict_of(&run(input).unwrap()), v, "{n}");
    }
}

#[test]
fn frz0_sample_golden_matches_exactly() {
    let reports = r#"{"arm_report:sample-1@1":{"runs":[{"case_ref":"c1","status":"failed","closed_early":false,"cost_known":true,"oracle_ref":"oracle:o@1"}]},
        "arm_report:sample-2@1":{"runs":[{"case_ref":"c1","status":"completed","closed_early":false,"cost_known":true,"oracle_ref":"oracle:o@1"}]}}"#;
    let input = format!("{{\"gate_in\":{SAMPLE_IN},\"reports\":{reports},\"world_authors\":{{\"world\":\"claude-wrld0\",\"suite\":\"suite-author\"}}}}");
    assert_eq!(canonical_json(&run(&input).unwrap()).unwrap(), canonical_json(SAMPLE_OUT).unwrap());
}

#[test]
fn skipping_a_gate_is_detectable_both_gates_always_reported() {
    // improvement fails while safety passes: a verdict that skipped the improvement gate would say "pass".
    let cs = cases();
    let input = &cs.iter().find(|(c, _)| c == "fx2_fail_improvement_equal").unwrap().1;
    let out = run(input).unwrap();
    assert_eq!(verdict_of(&out), "fail");
    let s = out.find("\"safety\"").expect("safety gate reported");
    let i = out.find("\"improvement\"").expect("improvement gate reported");
    assert!(s < i);
    // and the mirror: safety fails while improvement passes
    let input = &cs.iter().find(|(c, _)| c == "safety_regression_but_more_completed").unwrap().1;
    assert_eq!(verdict_of(&run(input).unwrap()), "fail");
}

#[test]
fn labels_quality_claims_forbidden_and_gate_authored_by_claude() {
    let cs = cases();
    let out = run(&cs[0].1).unwrap();
    assert_eq!(member(&out, "quality_claims"), "\"forbidden\"");
    assert_eq!(steps::gate::LABEL, "gate=claude-authored");
}

#[test]
fn json_reader_rejects_non_json_number_and_escape_forms() {
    for bad in ["01", "-01", "1.", "-.5", "1.e3", "1e", "1e+", "+1", "-", "[01]", "\"\\u+123\"", "\"\\u 123\"", "\"\\u-123\"", "[1,]", "{\"a\":1,}", "[1 2]", "nul", "{\"a\" 1}", "\"\\x\""] {
        assert!(canonical_json(bad).is_err(), "must reject {bad:?}");
    }
    for good in ["0", "-0", "0.0", "1e2", "1E+2", "-1.5e-3", "[]", "{}", "\"\\ud83d\\ude00\"", "{\"a\":1,\"a\":2}"] {
        assert!(canonical_json(good).is_ok(), "must accept {good:?}");
    }
    assert_eq!(canonical_json("{\"a\":1,\"a\":2}").unwrap(), "{\"a\":2}");
    assert!(canonical_json(&format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000))).is_err());
}

#[test]
fn malformed_inputs_return_err_never_panic() {
    let cs = cases();
    let base = cs.iter().find(|(c, _)| c == "fx1_pass").unwrap().1.clone();
    // every prefix and every single-byte mutation of a valid envelope
    for n in 0..base.len() {
        if base.is_char_boundary(n) { let _ = run(&base[..n]); }
    }
    for i in 0..base.len() {
        for rep in [b'{', b'}', b'[', b']', b'"', b'\\', b',', b':', b'-', b'0', b'u', b'e'] {
            let mut b = base.clone().into_bytes();
            b[i] = rep;
            if let Ok(s) = String::from_utf8(b) { let _ = run(&s); }
        }
    }
    for junk in ["", "null", "[]", "{}", "{\"gate_in\":null}", "{\"gate_in\":{},\"world_authors\":{}}", "\u{feff}{}", "{\"gate_in\":[1]}"] {
        assert!(run(junk).is_err(), "{junk:?}");
    }
}

#[test]
fn judge_separation_edge_spellings() {
    let cs = cases();
    let base = cs.iter().find(|(c, _)| c == "fx1_pass").unwrap().1.clone();
    for (judge, world) in [("claude-wrld0", "CLAUDE_WRLD0@9"), ("claude-wrld0", "  claude..wrld0@rev@2 "), ("claude-wrld0", "claude-_-wrld0")] {
        let input = base.replace("\"world\":\"claude-wrld0\"", &format!("\"world\":\"{world}\"")).replace("\"judge_actor\":\"claude-gsipy\"", &format!("\"judge_actor\":\"{judge}\""));
        assert!(input.contains(world) && input.contains(judge));
        assert_eq!(verdict_of(&run(&input).unwrap()), "not_evaluable", "{world}");
    }
}
