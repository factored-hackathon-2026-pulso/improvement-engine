//! Treated-payload scan (Rust port of `roleplay-llm/roleplay_llm/scanner.py`, TPS), deny-by-default over the fixed
//! agent input dict. Top-level keys are exactly `goal, inputs, step, tools, observations, feedback, output_schema`.
//! Everything that can carry data (inputs, observations) must be opaque ids, enums, booleans, small integers or
//! k-anonymous aggregate rows; any other string is rejected. Static text is bounded and PII-pattern checked.
//!
//! Differences from the Python scanner (stated, not hidden): no Unicode NFKC normalisation; instead static text
//! containing a non-ASCII numeric or an at-sign look-alike is rejected outright (stricter, never looser). Shapes are
//! hand matchers equal to the Python regexes (lowercase hex only).
use serde_json::{Map, Value};

pub const SCANNER_ID: &str = "tps-1-rs";
pub const DEFAULT_K: i64 = 10;
pub const SUPPRESSED: &str = "<k";
const TOP_KEYS: [&str; 7] = ["goal", "inputs", "step", "tools", "observations", "feedback", "output_schema"];
const OBS_KEYS: [&str; 5] = ["tool", "args", "status", "result", "error"];
const TOOL_KEYS: [&str; 3] = ["tool", "description", "args_schema"];
const ROW_KEYS: [&str; 5] = ["metric_id", "window_id", "count", "rate", "evidence_ref"];
const KNOWN_METRICS: [&str; 3] = ["recurrence_rate", "resolution_rate", "synthetic_metric"];
const MAX_STATIC: usize = 2000;
const MAX_FEEDBACK: usize = 500;
const MAX_INT: i64 = 10_000_000;
const MAX_FREE_INT: i64 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub ok: bool,
    pub violations: Vec<String>,
}

fn int_of(x: &Value) -> Option<i64> {
    x.as_i64().or_else(|| x.as_u64().map(|u| i64::try_from(u).unwrap_or(i64::MAX)))
}

fn opaque(s: &str) -> bool {
    (1..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'/' | b'-'))
}

fn enum_like(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=64).contains(&b.len()) && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-'))
}

fn hex(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn digits(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

fn tool_ref(s: &str) -> bool {
    let Some((name, ver)) = s.split_once('@') else { return false };
    let Some((a, b)) = name.split_once('/') else { return false };
    let part = |p: &str| !p.is_empty() && p.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-'));
    let v: Vec<&str> = ver.split('.').collect();
    part(a) && part(b) && v.len() == 3 && v.iter().all(|x| digits(x, 1, usize::MAX))
}

/// System-issued id shapes (a string passes without being registered).
fn shaped(s: &str) -> bool {
    if let Some(r) = s.strip_prefix("g_") {
        return hex(r, 16, 16);
    }
    if let Some(r) = s.strip_prefix("ev_") {
        return hex(r, 8, 64);
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 5 && [8, 4, 4, 4, 12].iter().zip(&parts).all(|(n, p)| hex(p, *n, *n)) {
        return true;
    }
    for p in ["binding", "job", "artifact"] {
        if let Some(r) = s.strip_prefix(p)
            && let Some(r) = r.strip_prefix(['-', '_', ':'])
        {
            return hex(r, 8, 32);
        }
    }
    for p in ["h_", "hyp_", "alt_", "fam_", "finding_"] {
        if let Some(r) = s.strip_prefix(p) {
            return digits(r, 1, 4);
        }
    }
    if let Some(r) = s.strip_prefix("w_") {
        let (a, b) = r.split_once('_').unwrap_or(("", ""));
        return digits(a, 4, 4) && digits(b, 2, 2);
    }
    s.strip_prefix('w').is_some_and(|r| digits(r, 1, 3))
}

fn email_like(s: &str) -> bool {
    s.split(char::is_whitespace).any(|tok| {
        let Some((l, r)) = tok.split_once('@') else { return false };
        !l.is_empty() && !r.contains('@') && r.split('.').count() >= 2 && r.split('.').all(|p| !p.is_empty())
    })
}

fn rut_like(s: &str) -> bool {
    let b = s.as_bytes();
    for lead in 1..=2usize {
        let mut i = 0;
        let take = |i: &mut usize, n: usize| -> bool {
            if *i + n <= b.len() && b[*i..*i + n].iter().all(u8::is_ascii_digit) {
                *i += n;
                true
            } else {
                false
            }
        };
        let opt = |i: &mut usize, c: u8| {
            if b.get(*i) == Some(&c) {
                *i += 1;
            }
        };
        if take(&mut i, lead) {
            opt(&mut i, b'.');
            if take(&mut i, 3) {
                opt(&mut i, b'.');
                if take(&mut i, 3) {
                    opt(&mut i, b'-');
                    if b.get(i).is_some_and(|c| c.is_ascii_digit() || matches!(c, b'k' | b'K')) && i + 1 == b.len() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn phone_like(s: &str) -> bool {
    let t = s.strip_prefix('+').unwrap_or(s);
    t.len() >= 7 && t.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'(' | b')' | b'.' | b'-'))
}

fn pii_like(s: &str) -> bool {
    email_like(s) || phone_like(s) || rut_like(s)
}

fn run_hits(run: &mut Vec<char>) -> bool {
    let f = run.iter().position(char::is_ascii_digit);
    let l = run.iter().rposition(char::is_ascii_digit);
    let hit = matches!((f, l), (Some(f), Some(l)) if l >= f + 7);
    run.clear();
    hit
}

/// `\d[\d\s().-]{6,}\d` anywhere: a run of that class holds a digit at i and a digit at j >= i + 7.
fn long_digits(s: &str) -> bool {
    let mut run: Vec<char> = vec![];
    for c in s.chars() {
        if c.is_ascii_digit() || c.is_whitespace() || matches!(c, '(' | ')' | '.' | '-') {
            run.push(c);
        } else if run_hits(&mut run) {
            return true;
        }
    }
    run_hits(&mut run)
}

fn disguised(s: &str) -> bool {
    s.chars().any(|c| (!c.is_ascii() && c.is_numeric()) || matches!(c, '\u{ff20}' | '\u{fe6b}'))
}

fn static_text(x: &Value, path: &str, v: &mut Vec<String>, limit: usize) {
    match x.as_str() {
        None => v.push(format!("{path}: not a string")),
        Some(s) if s.chars().count() > limit => v.push(format!("{path}: longer than {limit}")),
        Some(s) if email_like(s) || long_digits(s) || disguised(s) => v.push(format!("{path}: contains a PII-like pattern")),
        Some(_) => {}
    }
}

fn bounded_number(x: &Value, path: &str, v: &mut Vec<String>) {
    if let Some(i) = int_of(x) {
        if i.abs() > MAX_INT {
            v.push(format!("{path}: integer beyond {MAX_INT}"));
        }
    } else if x.is_f64() && x.as_f64().is_none_or(|f| !f.is_finite() || f.abs() > MAX_INT as f64) {
        v.push(format!("{path}: non-finite or oversized number"));
    }
}

fn static_tree(x: &Value, path: &str, v: &mut Vec<String>, depth: usize) {
    match x {
        _ if depth > 12 => v.push(format!("{path}: too deep")),
        Value::Object(m) => {
            for (k, val) in m {
                static_text(&Value::String(k.clone()), &format!("{path}.<key>"), v, 128);
                static_tree(val, &format!("{path}.{k}"), v, depth + 1);
            }
        }
        Value::Array(a) => a.iter().enumerate().for_each(|(i, val)| static_tree(val, &format!("{path}[{i}]"), v, depth + 1)),
        Value::String(_) => static_text(x, path, v, MAX_STATIC),
        Value::Null | Value::Bool(_) => {}
        Value::Number(_) => bounded_number(x, path, v),
    }
}

struct Allowed<'a>(&'a [String]);

impl Allowed<'_> {
    fn expected(&self, s: &str) -> bool {
        KNOWN_METRICS.contains(&s) || self.0.iter().any(|t| t == s) || shaped(s)
    }
}

fn opaque_str(s: &str, path: &str, v: &mut Vec<String>, a: &Allowed) {
    if !opaque(s) || pii_like(s) {
        v.push(format!("{path}: string is not an opaque id or enum"));
    } else if !a.expected(s) {
        v.push(format!("{path}: string is not an expected id or registered token"));
    }
}

fn opaque_tree(x: &Value, path: &str, v: &mut Vec<String>, a: &Allowed, depth: usize) {
    match x {
        _ if depth > 8 => v.push(format!("{path}: too deep")),
        Value::Object(m) => {
            for (k, val) in m {
                if !enum_like(k) {
                    v.push(format!("{path}.<key>: key is not an identifier"));
                    continue;
                }
                opaque_tree(val, &format!("{path}.{k}"), v, a, depth + 1);
            }
        }
        Value::Array(arr) => arr.iter().enumerate().for_each(|(i, val)| opaque_tree(val, &format!("{path}[{i}]"), v, a, depth + 1)),
        Value::String(s) => opaque_str(s, path, v, a),
        Value::Null | Value::Bool(_) => {}
        Value::Number(_) => match int_of(x) {
            Some(i) if i.abs() > MAX_FREE_INT => v.push(format!("{path}: free-form integer beyond {MAX_FREE_INT}")),
            _ => bounded_number(x, path, v),
        },
    }
}

fn tools(t: Option<&Value>, v: &mut Vec<String>) {
    let Some(arr) = t.and_then(Value::as_array) else {
        if t.is_some() {
            v.push("tools: not a list".into());
        }
        return;
    };
    for (i, t) in arr.iter().enumerate() {
        let p = format!("tools[{i}]");
        let Some(o) = t.as_object() else {
            v.push(format!("{p}: not an object"));
            continue;
        };
        for k in o.keys().filter(|k| !TOOL_KEYS.contains(&k.as_str())) {
            v.push(format!("{p}.{k}: key not allowed"));
        }
        if !o.get("tool").and_then(Value::as_str).is_some_and(tool_ref) {
            v.push(format!("{p}.tool: not an exact tool ref"));
        }
        static_text(o.get("description").unwrap_or(&Value::Null), &format!("{p}.description"), v, MAX_STATIC);
        static_tree(o.get("args_schema").unwrap_or(&Value::Null), &format!("{p}.args_schema"), v, 0);
    }
}

fn row(r: &Value, path: &str, k: i64, v: &mut Vec<String>, a: &Allowed) {
    let Some(o) = r.as_object() else {
        v.push(format!("{path}: not an object"));
        return;
    };
    for (key, val) in o.iter().filter(|(key, _)| !ROW_KEYS.contains(&key.as_str())) {
        let group = key.starts_with("g_") && enum_like(key) && val.as_str().is_some_and(|s| hex(s, 16, 16));
        if !group {
            v.push(format!("{path}.{key}: field not allowed (group keys are g_* HMAC hashes of 16 hex chars)"));
        }
    }
    for key in ["metric_id", "window_id"] {
        if !o.get(key).and_then(Value::as_str).is_some_and(|s| enum_like(s) && a.expected(s)) {
            v.push(format!("{path}.{key}: must be a known metric/window id"));
        }
    }
    let count = o.get("count");
    if count.and_then(Value::as_str) == Some(SUPPRESSED) {
        let extra: Vec<&String> = o.keys().filter(|k| !["metric_id", "window_id", "count"].contains(&k.as_str())).collect();
        if !extra.is_empty() {
            v.push(format!("{path}: suppressed row carries {extra:?}"));
        }
        return;
    }
    if !count.is_some_and(|c| int_of(c).is_some_and(|n| n >= k && n <= MAX_INT)) {
        v.push(format!("{path}.count: must be an integer >= k={k} or the suppressed marker"));
    }
    if let Some(rate) = o.get("rate") {
        let ok = rate.as_f64().is_some_and(|f| f.is_finite() && (0.0..=1.0).contains(&f) && ((f * 100.0).round() / 100.0 - f).abs() < 1e-12);
        if !ok {
            v.push(format!("{path}.rate: must be a number rounded to 2 decimals"));
        }
    }
    if let Some(e) = o.get("evidence_ref")
        && !e.as_str().is_some_and(|s| s.strip_prefix("ev_").is_some_and(|h| hex(h, 8, 64)))
    {
        v.push(format!("{path}.evidence_ref: not an opaque ev_ id"));
    }
}

fn observations(obs: Option<&Value>, k: i64, v: &mut Vec<String>, a: &Allowed) {
    let Some(arr) = obs.and_then(Value::as_array) else {
        if obs.is_some() {
            v.push("observations: not a list".into());
        }
        return;
    };
    for (i, o) in arr.iter().enumerate() {
        let p = format!("observations[{i}]");
        let Some(m) = o.as_object() else {
            v.push(format!("{p}: not an object"));
            continue;
        };
        for key in m.keys().filter(|k| !OBS_KEYS.contains(&k.as_str())) {
            v.push(format!("{p}.{key}: key not allowed"));
        }
        if !m.get("tool").and_then(Value::as_str).is_some_and(tool_ref) {
            v.push(format!("{p}.tool: not an exact tool ref"));
        }
        if !m.get("status").and_then(Value::as_str).is_some_and(enum_like) {
            v.push(format!("{p}.status: not an enum"));
        }
        if let Some(args) = m.get("args") {
            opaque_tree(args, &format!("{p}.args"), v, a, 0);
        }
        if let Some(e) = m.get("error").filter(|e| !e.is_null())
            && !e.as_str().is_some_and(enum_like)
        {
            v.push(format!("{p}.error: must be null or an enum code"));
        }
        let rp = format!("{p}.result");
        match m.get("result") {
            None | Some(Value::Null) => {}
            Some(Value::Object(res)) if res.keys().all(|k| k == "rows") => match res.get("rows").and_then(Value::as_array) {
                Some(rows) => rows.iter().enumerate().for_each(|(j, r)| row(r, &format!("{rp}.rows[{j}]"), k, v, a)),
                None => v.push(format!("{rp}.rows: not a list")),
            },
            Some(_) => v.push(format!("{rp}: only a rows list of treated aggregates is allowed")),
        }
    }
}

/// Scan one agent input dict. `registry` = the exact tokens the stage registered (ids, labels, metric/window ids).
pub fn scan_payload(payload: &Value, k: i64, registry: &[String]) -> Scan {
    let mut v: Vec<String> = vec![];
    let Some(o) = payload.as_object() else {
        return Scan { ok: false, violations: vec!["payload: not an object".into()] };
    };
    for key in o.keys().filter(|key| !TOP_KEYS.contains(&key.as_str())) {
        v.push(format!("payload.{key}: key not allowed"));
    }
    for key in ["goal", "step", "observations"] {
        if !o.contains_key(key) {
            v.push(format!("payload.{key}: missing"));
        }
    }
    if let Some(g) = o.get("goal") {
        static_text(g, "goal", &mut v, MAX_STATIC);
    }
    if let Some(s) = o.get("step")
        && !int_of(s).is_some_and(|n| (0..=1000).contains(&n))
    {
        v.push("step: not a non-negative integer".into());
    }
    let allowed = Allowed(registry);
    if let Some(i) = o.get("inputs") {
        opaque_tree(i, "inputs", &mut v, &allowed, 0);
    }
    if let Some(f) = o.get("feedback").filter(|f| !f.is_null()) {
        static_text(f, "feedback", &mut v, MAX_FEEDBACK);
    }
    if let Some(s) = o.get("output_schema") {
        static_tree(s, "output_schema", &mut v, 0);
    }
    tools(o.get("tools").or(Some(&Value::Array(vec![]))), &mut v);
    observations(o.get("observations"), k, &mut v, &allowed);
    Scan { ok: v.is_empty(), violations: v }
}

/// Below-k rows keep only their enums and a `<k` count; rate, hashed keys and evidence ref are dropped.
pub fn suppress_rows(rows: &[Value], k: i64) -> Vec<Value> {
    rows.iter()
        .map(|r| match r.get("count").and_then(int_of) {
            Some(c) if c < k => {
                let mut m = Map::new();
                for key in ["metric_id", "window_id"] {
                    if let Some(x) = r.get(key) {
                        m.insert(key.into(), x.clone());
                    }
                }
                m.insert("count".into(), Value::String(SUPPRESSED.into()));
                Value::Object(m)
            }
            _ => r.clone(),
        })
        .collect()
}
