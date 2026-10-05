//! W15 / rubric R11: no generated text may carry a run of 6 or more digits (the PII wrapper tokenises it).
use steps::compile::{defuse_digit_runs, sha256_hex, sha256_hex_calm};

fn longest_run(s: &str) -> usize {
    let (mut best, mut cur) = (0, 0);
    for c in s.chars() {
        cur = if c.is_ascii_digit() { cur + 1 } else { 0 };
        best = best.max(cur);
    }
    best
}

#[test]
fn a_calm_digest_is_a_deterministic_hex_prefix_without_a_long_digit_run() {
    let mut raw_bad = 0;
    for i in 0..4000u32 {
        let data = format!("finding-{i}|M1|{}", i * 7);
        let calm = sha256_hex_calm(data.as_bytes(), 24);
        assert_eq!(calm.len(), 24);
        assert!(calm.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{calm}");
        assert!(longest_run(&calm) < 6, "{calm}");
        assert_eq!(calm, sha256_hex_calm(data.as_bytes(), 24), "deterministic");
        if longest_run(&sha256_hex(data.as_bytes())[..24]) >= 6 {
            raw_bad += 1;
        }
    }
    assert!(raw_bad > 0, "the raw digest does hit the rule, so the test is meaningful");
    // a digest that is already calm is the raw prefix (existing keys do not move needlessly)
    let calm_raw = (0..200u32).map(|i| format!("x{i}")).find(|d| longest_run(&sha256_hex(d.as_bytes())[..16]) < 6).unwrap();
    assert_eq!(sha256_hex_calm(calm_raw.as_bytes(), 16), sha256_hex(calm_raw.as_bytes())[..16]);
}

#[test]
fn defusing_breaks_digit_runs_of_six_and_keeps_everything_else() {
    assert_eq!(defuse_digit_runs("no digits, v1.2 and 12345 stay"), "no digits, v1.2 and 12345 stay");
    let d = defuse_digit_runs("ticket 1234567 and 987654 then 12-345678901234");
    assert!(longest_run(&d) < 6, "{d}");
    assert!(d.starts_with("ticket ") && d.contains(" and ") && d.contains(" then "), "{d}");
    assert_eq!(d.chars().filter(|c| c.is_ascii_digit()).count(), 27, "no digit is lost: {d}");
}
