//! Story correlation (O11Y / ENGO): the W3C `traceparent` and `baggage` headers the engine adds to every llm-gateway and agent-core
//! call made for a finding, so the callee's spans join the engine's trace.
//!
//! The derivation is byte-identical to `scripts/o11y/trace_id.py` (vectors: `scripts/o11y/fixtures/traceparent_vectors.json`, read by
//! the tests below):
//!
//! ```text
//! trace_id      = hex(sha256("pulso.story.v1\n" + finding_key + "\n" + run_id))[0:32]
//! span_id(t, p) = hex(sha256(t + "|" + p.join("|")))[0:16]
//! root span     = span_id(t, ["story"]);  stage span = span_id(t, ["stage", stage, attempt]);  generation = span_id(t, ["gen", role, n])
//! traceparent   = "00-" + trace_id + "-" + parent_span_id + "-01"
//! ```
//!
//! The engine is single threaded per job, so the context of the finding being worked on is a thread-local scope (`enter`), narrowed to
//! a stage by `set_stage`. `headers()` is empty outside a scope. Header values never carry CR, LF or any control byte
//! (`header_value_ok`); `http::request` refuses a header that does.
use crate::canon::sha256_hex;
use std::cell::RefCell;

pub fn story_trace_id(finding_key: &str, run_id: &str) -> String {
    sha256_hex(format!("pulso.story.v1\n{finding_key}\n{run_id}").as_bytes())[..32].to_string()
}

pub fn span_id(trace_id: &str, parts: &[&str]) -> String {
    let mut s = trace_id.to_string();
    for p in parts {
        s.push('|');
        s.push_str(p);
    }
    sha256_hex(s.as_bytes())[..16].to_string()
}

pub fn root_span_id(trace_id: &str) -> String {
    span_id(trace_id, &["story"])
}

pub fn stage_span_id(trace_id: &str, stage: &str, attempt: u32) -> String {
    span_id(trace_id, &["stage", stage, &attempt.to_string()])
}

pub fn generation_span_id(trace_id: &str, role: &str, n: u32) -> String {
    span_id(trace_id, &["gen", role, &n.to_string()])
}

/// The header the engine sends. The parent is the stage span, or the root span when there is no stage.
pub fn traceparent_for(finding_key: &str, run_id: &str, stage: Option<&str>, attempt: u32) -> String {
    let t = story_trace_id(finding_key, run_id);
    let parent = match stage {
        Some(s) => stage_span_id(&t, s, attempt),
        None => root_span_id(&t),
    };
    format!("00-{t}-{parent}-01")
}

/// A header value is a single line of visible ASCII: no CR, LF, NUL or other control byte (request splitting) and bounded.
pub fn header_value_ok(v: &str) -> bool {
    v.len() <= 1024 && v.bytes().all(|b| b == b'\t' || (0x20..0x7f).contains(&b))
}

/// What identifies the story and its tags (O11Y.md: session = run id; tags agent, release, locale, case-type, stage).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceCtx {
    pub finding_key: String,
    pub run_id: String,
    pub release: String,
    pub agent: String,
    pub locale: String,
    pub case_type: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Current {
    pub ctx: TraceCtx,
    pub stage: Option<String>,
    pub attempt: u32,
}

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
}

/// Restores the previous context when dropped.
pub struct Scope(Option<Current>);

impl Drop for Scope {
    fn drop(&mut self) {
        let prev = self.0.take();
        CURRENT.with(|c| *c.borrow_mut() = prev);
    }
}

pub fn enter(ctx: TraceCtx) -> Scope {
    let prev = CURRENT.with(|c| c.borrow_mut().replace(Current { ctx, stage: None, attempt: 1 }));
    Scope(prev)
}

/// Narrows the current context to a stage (and its attempt, 1-based). A no-op outside a scope.
pub fn set_stage(stage: &str, attempt: u32) {
    CURRENT.with(|c| {
        if let Some(cur) = c.borrow_mut().as_mut() {
            cur.stage = Some(stage.to_string());
            cur.attempt = attempt.max(1);
        }
    });
}

pub fn current() -> Option<Current> {
    CURRENT.with(|c| c.borrow().clone())
}

fn pct(v: &str) -> String {
    let mut o = String::new();
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            o.push(char::from(b));
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// W3C baggage: `session` (the run id), `release`, `agent`, `locale`, `case-type`, `stage`; empty values are left out.
pub fn baggage(c: &Current) -> String {
    let stage = c.stage.clone().unwrap_or_default();
    // no explicit agent: the callee is the engine's role for the stage (`pulso-scout`, `pulso-builder`, ...)
    let agent = if c.ctx.agent.is_empty() && !stage.is_empty() { format!("pulso-{stage}") } else { c.ctx.agent.clone() };
    let items = [("session", &c.ctx.run_id), ("release", &c.ctx.release), ("agent", &agent), ("locale", &c.ctx.locale), ("case-type", &c.ctx.case_type), ("stage", &stage)];
    let mut out = String::new();
    for (k, v) in items {
        if v.is_empty() {
            continue;
        }
        let item = format!("{k}={}", pct(&v.chars().take(128).collect::<String>()));
        if out.len() + item.len() + 1 > 512 {
            break;
        }
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&item);
    }
    out
}

/// `traceparent` (and `baggage`) for the call in progress; empty outside a scope.
pub fn headers() -> Vec<(&'static str, String)> {
    let Some(c) = current() else { return vec![] };
    let mut h = vec![("traceparent", traceparent_for(&c.ctx.finding_key, &c.ctx.run_id, c.stage.as_deref(), c.attempt))];
    let b = baggage(&c);
    if !b.is_empty() {
        h.push(("baggage", b));
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn vectors() -> Value {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../scripts/o11y/fixtures/traceparent_vectors.json");
        serde_json::from_str(&std::fs::read_to_string(p).expect("vectors file")).expect("json")
    }

    #[test]
    fn every_vector_of_the_python_derivation_is_reproduced_byte_for_byte() {
        let v = vectors();
        assert_eq!(v["schema"], "pulso.traceparent_vectors/1");
        let list = v["vectors"].as_array().unwrap();
        assert!(list.len() >= 5);
        let mut stages = 0;
        for x in list {
            let (fk, run) = (x["finding_key"].as_str().unwrap_or(""), x["run_id"].as_str().unwrap());
            let t = story_trace_id(fk, run);
            assert_eq!(t, x["trace_id"].as_str().unwrap(), "{fk:?} {run:?}");
            assert_eq!(root_span_id(&t), x["root_span_id"].as_str().unwrap());
            assert_eq!(traceparent_for(fk, run, None, 1), x["traceparent_root"].as_str().unwrap());
            for (stage, s) in x["stages"].as_object().unwrap() {
                assert_eq!(stage_span_id(&t, stage, 1), s["span_id"].as_str().unwrap(), "{stage}");
                assert_eq!(traceparent_for(fk, run, Some(stage), 1), s["traceparent"].as_str().unwrap());
                stages += 1;
            }
            let a2 = &x["stage_attempt_2"];
            let st = a2["stage"].as_str().unwrap();
            assert_eq!(stage_span_id(&t, st, 2), a2["span_id"].as_str().unwrap());
            assert_eq!(traceparent_for(fk, run, Some(st), 2), a2["traceparent"].as_str().unwrap());
            let g = &x["generation"];
            assert_eq!(generation_span_id(&t, g["role"].as_str().unwrap(), u32::try_from(g["n"].as_u64().unwrap()).unwrap()), g["span_id"].as_str().unwrap());
        }
        assert!(stages >= 40, "{stages}");
    }

    #[test]
    fn headers_follow_the_scope_and_the_stage_and_restore_on_exit() {
        assert!(headers().is_empty());
        let ctx = TraceCtx { finding_key: "ev_3f9a1c07d2b84e51".into(), run_id: "value-loop-trg-20261005-0001".into(), release: "r1".into(), agent: "pulso-scout".into(), locale: "es".into(), case_type: "prompt".into() };
        {
            let _g = enter(ctx.clone());
            let h = headers();
            assert_eq!(h[0], ("traceparent", "00-7e1ffba44834058839ef1a914c474128-35b5c9ea138bec11-01".to_string()));
            set_stage("scout", 1);
            let h = headers();
            assert_eq!(h[0].1, "00-7e1ffba44834058839ef1a914c474128-69dba9a106f229ee-01");
            assert_eq!(h[1], ("baggage", "session=value-loop-trg-20261005-0001,release=r1,agent=pulso-scout,locale=es,case-type=prompt,stage=scout".to_string()));
            {
                let _inner = enter(TraceCtx { run_id: "x".into(), ..ctx.clone() });
                assert_eq!(current().unwrap().ctx.run_id, "x");
            }
            assert_eq!(current().unwrap().stage.as_deref(), Some("scout"));
        }
        assert!(current().is_none());
    }

    #[test]
    fn baggage_is_percent_encoded_bounded_and_never_carries_control_bytes() {
        let c = Current { ctx: TraceCtx { run_id: "a b,c=d\r\nX: y".into(), release: "\u{e9}".into(), ..TraceCtx::default() }, stage: None, attempt: 1 };
        let b = baggage(&c);
        assert_eq!(b, "session=a%20b%2Cc%3Dd%0D%0AX%3A%20y,release=%C3%A9");
        assert!(header_value_ok(&b));
        let long = Current { ctx: TraceCtx { run_id: "z".repeat(400), release: "r".repeat(400), agent: "a".repeat(400), ..TraceCtx::default() }, stage: None, attempt: 1 };
        assert!(baggage(&long).len() <= 512);
    }

    #[test]
    fn header_values_refuse_cr_lf_nul_and_other_control_bytes() {
        assert!(header_value_ok("00-7e1ffba44834058839ef1a914c474128-35b5c9ea138bec11-01"));
        for bad in ["a\r\nX: y", "a\nb", "a\rb", "a\0b", "a\x7fb", "\u{e9}"] {
            assert!(!header_value_ok(bad), "{bad:?}");
        }
        assert!(!header_value_ok(&"x".repeat(1025)));
    }
}
