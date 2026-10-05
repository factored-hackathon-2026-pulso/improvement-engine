//! `pulso.model_call/1`: the record of ONE model call (O11Y contract, `docs/dev/O11Y.md`), built from what `Recording` observed.
//!
//! ```text
//! {"schema":"pulso.model_call/1","evidence_ref","run_id","role","n","model_id","tier","label","data_class","stage","attempt","retries",
//!  "started_at","duration_ms","tokens_in","tokens_out","cost_usd","cost_source","outcome","why",
//!  "request":{"messages":[{"role":"system","content"},{"role":"user","content"}],"max_tokens"},"response","truncated",
//!  "trace_id","span_id","parent_span_id"}
//! ```
//!
//! Content: the engine only sends TREATED aggregates to a model (the TPS scan stays on and runs before the call), so the request is
//! treated text; the response is the raw text the model returned. Both pass `scrub` (bearer tokens, `sk-` / `pk-lf-` keys, JWTs and
//! `key=value` secrets are masked, a credential never reaches a record) and are bounded (`MAX_CONTENT_CHARS` per item, `truncated`
//! says when). The gateway key travels only in a header and is not part of any record. Cost comes from the price table
//! (`price_per_token`); a model outside the table keeps the cost the gateway reported (`cost_source`).
use super::{CallRecord, Outcome};
use core_client::trace::{self, Current};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA: &str = "pulso.model_call/1";
/// Bound of one content item (system prompt, user payload, response), in characters.
pub const MAX_CONTENT_CHARS: usize = 24 * 1024;

/// USD per token (in, out) of the models the loop uses (the same table as `scripts/o11y/runtrace_bridge.py`).
pub fn price_per_token(model: &str) -> Option<(f64, f64)> {
    match model.to_ascii_lowercase().as_str() {
        "xiaomi/mimo-v2.6-flash" => Some((1.4e-7, 2.8e-7)),
        "xiaomi/mimo-v2.6-pro" => Some((4.35e-7, 8.7e-7)),
        "z-ai/glm-5.3-flash" => Some((1.5e-7, 5e-7)),
        _ => None,
    }
}

/// `flash | pro | other` from the model id.
pub fn tier_of(model_id: &str) -> &'static str {
    let m = model_id.rsplit('/').next().unwrap_or(model_id);
    if m.contains("flash") {
        "flash"
    } else if m.contains("-pro") {
        "pro"
    } else {
        "other"
    }
}

/// RFC 3339 UTC with milliseconds (civil-from-days, no time crate).
pub fn now_rfc3339() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let (secs, ms) = (i64::try_from(d.as_secs()).unwrap_or(0), d.subsec_millis());
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn is_tok(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+' | b'/' | b'=' | b'~')
}

/// Masks credentials in free text: `Bearer <tok>`, `sk-<8+>`, `pk-lf-<tok>`, JWTs (`eyJ..<dot>..`) and `api_key|secret|token|password` followed by
/// `=` or `:` and a value. Case-insensitive on the markers; everything else is kept as is.
pub fn scrub(text: &str) -> String {
    let b = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut last) = (0, 0);
    let end_tok = |from: usize| {
        let mut j = from;
        while j < b.len() && is_tok(b[j]) {
            j += 1;
        }
        j
    };
    while i < b.len() {
        let rest = &lb[i..];
        let mut hit: Option<usize> = None; // end of the secret starting at i
        if rest.starts_with(b"bearer") && b.get(i + 6).is_some_and(|c| c.is_ascii_whitespace()) {
            let mut j = i + 6;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let e = end_tok(j);
            if e > j {
                hit = Some(e);
            }
        } else if rest.starts_with(b"pk-lf-") {
            hit = Some(end_tok(i + 6)).filter(|e| *e > i + 6);
        } else if rest.starts_with(b"sk-") && (i == 0 || !is_tok(b[i - 1])) {
            let e = end_tok(i + 3);
            if e - (i + 3) >= 8 {
                hit = Some(e);
            }
        } else if rest.starts_with(b"eyj") && (i == 0 || !is_tok(b[i - 1])) {
            let e = end_tok(i);
            if text[i..e].matches('.').count() >= 1 && e - i >= 13 {
                hit = Some(e);
            }
        } else {
            for key in ["api_key", "api-key", "apikey", "secret", "token", "password"] {
                if rest.starts_with(key.as_bytes()) && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
                    let mut j = i + key.len();
                    while j < b.len() && b[j] == b' ' {
                        j += 1;
                    }
                    if j < b.len() && (b[j] == b'=' || b[j] == b':') {
                        j += 1;
                        while j < b.len() && b[j] == b' ' {
                            j += 1;
                        }
                        let mut e = j;
                        while e < b.len() && !b[e].is_ascii_whitespace() && !matches!(b[e], b'"' | b'\'' | b',' | b'}' | b']') {
                            e += 1;
                        }
                        if e > j {
                            hit = Some(e);
                        }
                    }
                    break;
                }
            }
        }
        match hit {
            Some(e) => {
                out.push_str(&text[last..i]);
                out.push_str("[redacted]");
                i = e;
                last = e;
            }
            None => i += 1,
        }
        while i < b.len() && !text.is_char_boundary(i) {
            i += 1;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// Scrubbed and bounded content; the flag says whether it was cut.
pub fn bound(text: &str) -> (String, bool) {
    let s = scrub(text);
    if s.chars().count() > MAX_CONTENT_CHARS {
        (s.chars().take(MAX_CONTENT_CHARS).collect(), true)
    } else {
        (s, false)
    }
}

fn outcome_parts(o: &Outcome) -> (&'static str, String) {
    match o {
        Outcome::Answered => ("answered", String::new()),
        Outcome::Refused(w) => ("refused", w.clone()),
        Outcome::Unavailable(w) => ("unavailable", w.clone()),
        Outcome::Invalid(w) => ("invalid", w.clone()),
    }
}

/// The record of one call. `n` is the 1-based index of the call within its role for the finding (`gen` span id); `ctx` is the story the call
/// ran in (None outside a story: the ids are then absent).
pub fn call_record(c: &CallRecord, n: u32, ctx: Option<&Current>) -> Value {
    let (outcome, why) = outcome_parts(&c.outcome);
    let (tin, tout) = c.usage.as_ref().map_or((None, None), |u| (Some(u.tokens_in), Some(u.tokens_out)));
    let table = c.usage.as_ref().and_then(|u| price_per_token(&c.model_id).map(|(pi, po)| u.tokens_in as f64 * pi + u.tokens_out as f64 * po));
    let (cost, source) = match (table, &c.usage) {
        (Some(t), _) => (Some(t), "price_table"),
        (None, Some(u)) if !u.cost_usd.is_empty() => (Some(u.cost_f64()), "gateway"),
        _ => (None, "unknown"),
    };
    let (mut truncated, mut messages) = (false, vec![]);
    let mut max_tokens = Value::Null;
    if let Some((system, payload)) = &c.request {
        let (s, t1) = bound(system);
        let (u, t2) = bound(&payload.to_string());
        truncated |= t1 | t2;
        messages = vec![json!({"role": "system", "content": s}), json!({"role": "user", "content": u})];
        max_tokens = payload.get("max_tokens").cloned().unwrap_or(Value::Null);
    }
    let response = c.response.as_ref().map(|r| {
        let (t, cut) = bound(r);
        truncated |= cut;
        t
    });
    let mut v = json!({
        "schema": SCHEMA, "role": c.role.as_str(), "n": n, "model_id": c.model_id, "tier": tier_of(&c.model_id), "label": c.label.as_str(), "data_class": c.data_class.as_str(),
        "stage": c.stage, "attempt": c.attempt, "retries": c.attempt.saturating_sub(1),
        "started_at": c.started_at, "duration_ms": c.wall_ms, "tokens_in": tin, "tokens_out": tout,
        "cost_usd": cost.map(|x| (x * 1e12).round() / 1e12), "cost_source": source,
        "outcome": outcome, "why": scrub(&why).chars().take(300).collect::<String>(),
        "request": {"messages": messages, "max_tokens": max_tokens}, "response": response, "truncated": truncated,
    });
    if let Some(cx) = ctx {
        let t = trace::story_trace_id(&cx.ctx.finding_key, &cx.ctx.run_id);
        let stage = c.stage.clone().unwrap_or_default();
        v["evidence_ref"] = json!(cx.ctx.finding_key);
        v["run_id"] = json!(cx.ctx.run_id);
        v["trace_id"] = json!(t);
        v["span_id"] = json!(trace::generation_span_id(&t, c.role.as_str(), n));
        v["parent_span_id"] = json!(if stage.is_empty() { trace::root_span_id(&t) } else { trace::stage_span_id(&t, &stage, c.attempt.max(1)) });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_masked_and_ordinary_text_is_not() {
        let t = "auth Bearer abc.DEF-123 and sk-abcdefgh12345 and pk-lf-1234-abcd and eyJhbGciOiJIUzI1.eyJzdWIiOiIxIn0.sig and api_key=XYZ123 and password: hunter2, token = t0k3n \"ok\"";
        let s = scrub(t);
        for secret in ["abc.DEF-123", "sk-abcdefgh12345", "pk-lf-1234-abcd", "eyJhbGciOiJIUzI1", "XYZ123", "hunter2", "t0k3n"] {
            assert!(!s.contains(secret), "{secret} in {s}");
        }
        assert_eq!(s.matches("[redacted]").count(), 7, "{s}");
        let plain = "la tasa de reapertura es 0.45 (task-force, sketch, tokenizer, risk-free) en el canal Phone";
        assert_eq!(scrub(plain), plain);
        assert_eq!(scrub("Bearer"), "Bearer");
        assert_eq!(scrub("caf\u{e9} \u{1f600} sk-abcdefgh12345 fin"), "caf\u{e9} \u{1f600} [redacted] fin");
    }

    #[test]
    fn content_is_bounded_on_a_char_boundary_and_says_so() {
        let (s, cut) = bound(&"\u{e9}".repeat(MAX_CONTENT_CHARS + 5));
        assert!(cut && s.chars().count() == MAX_CONTENT_CHARS);
        let (s, cut) = bound("short");
        assert!(!cut && s == "short");
    }

    #[test]
    fn price_table_and_tier() {
        assert_eq!(price_per_token("xiaomi/mimo-v2.6-flash"), Some((1.4e-7, 2.8e-7)));
        assert_eq!(price_per_token("xiaomi/mimo-v2.6-pro"), Some((4.35e-7, 8.7e-7)));
        assert_eq!(price_per_token("z-ai/glm-5.3-flash"), Some((1.5e-7, 5e-7)));
        assert_eq!(price_per_token("other/x"), None);
        assert_eq!((tier_of("xiaomi/mimo-v2.6-flash"), tier_of("xiaomi/mimo-v2.6-pro"), tier_of("a/b")), ("flash", "pro", "other"));
    }

    #[test]
    fn the_clock_is_rfc3339_utc_with_milliseconds() {
        let t = now_rfc3339();
        assert!(t.len() == 24 && t.ends_with('Z') && &t[10..11] == "T" && &t[19..20] == ".", "{t}");
        assert!(t.starts_with("20"), "{t}");
    }
}
