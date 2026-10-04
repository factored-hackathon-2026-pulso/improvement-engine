//! GSI: stand-in two-gate verdict over arm reports (safety and improvement), author is not judge.
//!
//! Port of the reviewed Python reference `e2e-core/src/claude_standin/gate_step.py`, output shape per
//! `contracts/engine-steps` gate.out. STRUCTURAL verdict only: it never claims quality
//! (`quality_claims` is always "forbidden"). Label of this stand-in: [`LABEL`].
//!
//! `run` input is one JSON envelope: `{"gate_in": <gate.in>, "reports": {<ref>: {"runs": [..]}},
//! "world_authors": {"world": .., "suite": ..}}`; output is the gate.out JSON. std only.
//! Only [`READ_FIELDS`] of each run are read; planted columns (verdict, lift, mechanism..) are
//! ignored by construction because runs are projected first.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Authorship label of this gate (the gate is claude-authored, so Claude never judges its own work).
pub const LABEL: &str = "gate=claude-authored";
/// The only run fields the gate reads.
pub const READ_FIELDS: [&str; 5] = ["case_ref", "status", "closed_early", "cost_known", "oracle_ref"];
const GATES: [&str; 2] = ["safety", "improvement"];

#[derive(Debug, Clone, PartialEq)]
pub struct GateError(pub String);

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gate: {}", self.0)
    }
}
impl std::error::Error for GateError {}

fn err<T>(m: impl Into<String>) -> Result<T, GateError> {
    Err(GateError(m.into()))
}

// ---------------------------------------------------------------- minimal JSON

#[derive(Debug, Clone, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<J>),
    Obj(BTreeMap<String, J>),
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), GateError> {
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            err(format!("json: expected '{}' at {}", c as char, self.i))
        }
    }
    fn lit(&mut self, s: &str, v: J) -> Result<J, GateError> {
        if self.b[self.i..].starts_with(s.as_bytes()) {
            self.i += s.len();
            Ok(v)
        } else {
            err(format!("json: bad literal at {}", self.i))
        }
    }
    fn hex4(&mut self) -> Result<u32, GateError> {
        let h = self.b.get(self.i..self.i + 4).filter(|s| s.iter().all(u8::is_ascii_hexdigit));
        let v = h.and_then(|s| std::str::from_utf8(s).ok()).and_then(|s| u32::from_str_radix(s, 16).ok());
        self.i += 4;
        v.ok_or_else(|| GateError("json: bad \\u escape".into()))
    }
    fn string(&mut self) -> Result<String, GateError> {
        self.eat(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i).ok_or_else(|| GateError("json: unterminated string".into()))?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or_else(|| GateError("json: bad escape".into()))?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if self.b.get(self.i..self.i + 2) != Some(b"\\u") {
                                    return err("json: lone surrogate");
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return err("json: bad surrogate pair");
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            char::from_u32(cp).ok_or_else(|| GateError("json: bad code point".into()))?
                        }
                        _ => return err("json: bad escape"),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c if c < 0x20 => return err("json: control character in string"),
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| GateError("json: invalid utf-8".into()))
    }
    fn digits(&mut self) -> Result<(), GateError> {
        let s = self.i;
        while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        if self.i == s {
            return err("json: bad number");
        }
        Ok(())
    }
    /// Strict RFC 8259 number: -?(0|[1-9][0-9]*)(.[0-9]+)?([eE][+-]?[0-9]+)?
    fn number(&mut self) -> Result<(), GateError> {
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        if self.b.get(self.i) == Some(&b'0') {
            self.i += 1;
        } else {
            self.digits()?;
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            self.digits()?;
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            self.digits()?;
        }
        Ok(())
    }
    fn value(&mut self, depth: u32) -> Result<J, GateError> {
        if depth > 64 {
            return err("json: too deep");
        }
        self.ws();
        match self.b.get(self.i) {
            None => err("json: unexpected end"),
            Some(b'n') => self.lit("null", J::Null),
            Some(b't') => self.lit("true", J::Bool(true)),
            Some(b'f') => self.lit("false", J::Bool(false)),
            Some(b'"') => Ok(J::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(J::Arr(v));
                }
                loop {
                    v.push(self.value(depth + 1)?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(J::Arr(v));
                        }
                        _ => return err("json: expected ',' or ']'"),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut m = BTreeMap::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(J::Obj(m));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    self.eat(b':')?;
                    let v = self.value(depth + 1)?;
                    m.insert(k, v);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(J::Obj(m));
                        }
                        _ => return err("json: expected ',' or '}'"),
                    }
                }
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let s = self.i;
                self.number()?;
                let t = std::str::from_utf8(&self.b[s..self.i]).unwrap_or("");
                Ok(J::Num(t.to_string()))
            }
            Some(_) => err(format!("json: unexpected byte at {}", self.i)),
        }
    }
}

fn parse(s: &str) -> Result<J, GateError> {
    let mut p = P { b: s.as_bytes(), i: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return err("json: trailing data");
    }
    Ok(v)
}

fn write_str(s: &str, o: &mut String) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
}

/// Canonical JSON: sorted keys, no whitespace.
fn write(v: &J, o: &mut String) {
    match v {
        J::Null => o.push_str("null"),
        J::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
        J::Num(n) => o.push_str(n),
        J::Str(s) => write_str(s, o),
        J::Arr(a) => {
            o.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                write(x, o);
            }
            o.push(']');
        }
        J::Obj(m) => {
            o.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                write_str(k, o);
                o.push(':');
                write(x, o);
            }
            o.push('}');
        }
    }
}

/// Re-serialise any JSON text in canonical form (for byte-exact comparison across implementations).
pub fn canonical_json(input: &str) -> Result<String, GateError> {
    let v = parse(input)?;
    let mut o = String::new();
    write(&v, &mut o);
    Ok(o)
}

// ---------------------------------------------------------------- gate

fn s(x: &str) -> J {
    J::Str(x.to_string())
}

fn obj(pairs: Vec<(&str, J)>) -> J {
    J::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Canonical actor identity: case, `_ . -` separators and an `@revision` suffix do not make a different actor.
fn actor(a: &str) -> String {
    let base = a.trim().to_lowercase();
    let base = base.split('@').next().unwrap_or("");
    let mut out = String::new();
    let mut in_sep = false;
    for c in base.chars() {
        if matches!(c, '-' | '_' | '.') {
            if !in_sep {
                out.push('-');
            }
            in_sep = true;
        } else {
            out.push(c);
            in_sep = false;
        }
    }
    out
}

fn id_ok(v: &str) -> bool {
    let b = v.as_bytes();
    (3..=64).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
}

fn ref_ok(v: &str) -> bool {
    // ^[a-z_]+:[A-Za-z0-9._-]+@[0-9]+$
    let Some((kind, rest)) = v.split_once(':') else { return false };
    let Some((name, rev)) = rest.rsplit_once('@') else { return false };
    !kind.is_empty()
        && kind.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
        && !name.is_empty()
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !rev.is_empty()
        && rev.bytes().all(|c| c.is_ascii_digit())
}

/// Validate gate.in per the FRZ0 schema (required keys, no extras, const/enum/patterns).
fn validate_in(m: &BTreeMap<String, J>) -> Result<(), GateError> {
    const KEYS: [&str; 9] = [
        "contract_version",
        "step",
        "run_id",
        "data_class",
        "base_arm_report_ref",
        "candidate_arm_report_ref",
        "suite_ref",
        "judge_actor",
        "author_actors",
    ];
    if m.len() != KEYS.len() || !KEYS.iter().all(|k| m.contains_key(*k)) {
        return err("gate input invalid: keys");
    }
    let st = |k: &str| match &m[k] {
        J::Str(x) => Some(x.as_str()),
        _ => None,
    };
    let ok = st("contract_version") == Some("engine-steps/0")
        && st("step") == Some("gate")
        && st("run_id").is_some_and(id_ok)
        && matches!(st("data_class"), Some("synthetic" | "treated" | "e0" | "original"))
        && ["base_arm_report_ref", "candidate_arm_report_ref", "suite_ref"]
            .iter()
            .all(|k| st(k).is_some_and(ref_ok))
        && st("judge_actor").is_some_and(id_ok)
        && matches!(&m["author_actors"], J::Arr(a) if !a.is_empty() && a.iter().all(|x| matches!(x, J::Str(t) if id_ok(t))));
    if ok {
        Ok(())
    } else {
        err("gate input invalid")
    }
}

/// One run projected onto READ_FIELDS (absent field = Null).
struct Run {
    case_ref: J,
    status: J,
    closed_early: J,
    cost_known: J,
    oracle_ref: J,
}

fn project(runs: &[J]) -> Vec<Run> {
    let g = |r: &J, k: &str| match r {
        J::Obj(m) => m.get(k).cloned().unwrap_or(J::Null),
        _ => J::Null,
    };
    runs.iter()
        .map(|r| Run {
            case_ref: g(r, READ_FIELDS[0]),
            status: g(r, READ_FIELDS[1]),
            closed_early: g(r, READ_FIELDS[2]),
            cost_known: g(r, READ_FIELDS[3]),
            oracle_ref: g(r, READ_FIELDS[4]),
        })
        .collect()
}

fn completed(r: &Run) -> bool {
    r.status == J::Str("completed".into())
}

fn truthy(v: &J) -> bool {
    match v {
        J::Null => false,
        J::Bool(b) => *b,
        J::Str(x) => !x.is_empty(),
        J::Arr(a) => !a.is_empty(),
        J::Obj(m) => !m.is_empty(),
        J::Num(n) => n.parse::<f64>().map(|f| f != 0.0).unwrap_or(true),
    }
}

type Verdict = (&'static str, Option<&'static str>);

fn key(r: &Run) -> String {
    format!("{:?}", r.case_ref)
}

fn safety(base: &[Run], cand: &[Run]) -> Verdict {
    if cand.iter().any(|r| r.closed_early == J::Bool(true)) {
        return ("fail", Some("candidate_closed_early"));
    }
    let bdone: BTreeSet<String> = base.iter().filter(|r| completed(r)).map(key).collect();
    if cand.iter().any(|r| bdone.contains(&key(r)) && !completed(r)) {
        return ("fail", Some("candidate_regressed_a_completed_case"));
    }
    ("pass", None)
}

fn improvement(base: &[Run], cand: &[Run]) -> Verdict {
    let b = base.iter().filter(|r| completed(r)).count();
    let c = cand.iter().filter(|r| completed(r)).count();
    if c > b {
        ("pass", None)
    } else {
        ("fail", Some("no_structural_improvement"))
    }
}

fn gate_obj(g: &str, status: &str, reason: Option<&str>) -> J {
    let mut p = vec![("gate", s(g)), ("status", s(status))];
    if let Some(r) = reason {
        p.push(("reason", s(r)));
    }
    obj(p)
}

fn head(doc: &BTreeMap<String, J>) -> Vec<(&'static str, J)> {
    vec![
        ("contract_version", s("engine-steps/0")),
        ("step", s("gate")),
        ("run_id", doc["run_id"].clone()),
        ("data_class", doc["data_class"].clone()),
        ("judge_actor", doc["judge_actor"].clone()),
        ("quality_claims", s("forbidden")),
    ]
}

fn emit(mut h: Vec<(&'static str, J)>, verdict: &str, gates: Vec<J>) -> String {
    h.push(("verdict", s(verdict)));
    h.push(("gates", J::Arr(gates)));
    let mut o = String::new();
    write(&obj(h), &mut o);
    o
}

fn not_evaluable(doc: &BTreeMap<String, J>, reason: &str) -> String {
    emit(head(doc), "not_evaluable", GATES.iter().map(|g| gate_obj(g, "not_evaluable", Some(reason))).collect())
}

fn text<'a>(doc: &'a BTreeMap<String, J>, k: &str) -> &'a str {
    match &doc[k] {
        J::Str(x) => x,
        _ => "",
    }
}

/// Run the stand-in gate. See the module docs for the envelope shape.
pub fn run(input: &str) -> Result<String, GateError> {
    let env = match parse(input)? {
        J::Obj(m) => m,
        _ => return err("input must be an object"),
    };
    let doc = match env.get("gate_in") {
        Some(J::Obj(m)) => m,
        _ => return err("gate_in missing"),
    };
    validate_in(doc)?;
    let reports = match env.get("reports") {
        Some(J::Obj(m)) => m.clone(),
        None | Some(J::Null) => BTreeMap::new(),
        _ => return err("reports must be an object"),
    };
    let wa = match env.get("world_authors") {
        Some(J::Obj(m)) => m,
        _ => return err("world_authors missing"),
    };
    let wauthor = |k: &str| match wa.get(k) {
        Some(J::Str(x)) => Ok(actor(x)),
        _ => err(format!("world_authors.{k} missing")),
    };

    let mut authors: BTreeSet<String> = match &doc["author_actors"] {
        J::Arr(a) => a.iter().filter_map(|x| if let J::Str(t) = x { Some(actor(t)) } else { None }).collect(),
        _ => BTreeSet::new(),
    };
    authors.insert(wauthor("world")?);
    authors.insert(wauthor("suite")?);
    if authors.contains(&actor(text(doc, "judge_actor"))) {
        return Ok(not_evaluable(doc, "judge_not_separated"));
    }

    let runs_of = |k: &str| -> Option<Vec<J>> {
        match reports.get(text(doc, k)) {
            Some(J::Obj(r)) => match r.get("runs") {
                Some(J::Arr(a)) if !a.is_empty() && a.iter().all(|x| matches!(x, J::Obj(_))) => Some(a.clone()),
                _ => None,
            },
            _ => None,
        }
    };
    let (Some(braw), Some(craw)) = (runs_of("base_arm_report_ref"), runs_of("candidate_arm_report_ref")) else {
        return Ok(not_evaluable(doc, "empty_arm_report"));
    };
    let (base, cand) = (project(&braw), project(&craw));
    for side in [&base, &cand] {
        let valid = side.iter().all(|r| matches!(&r.case_ref, J::Str(c) if !c.is_empty()));
        let uniq: BTreeSet<String> = side.iter().map(key).collect();
        if !valid || uniq.len() != side.len() {
            return Ok(not_evaluable(doc, "case_sets_differ"));
        }
    }
    let set = |v: &[Run]| -> BTreeSet<String> { v.iter().map(key).collect() };
    if set(&base) != set(&cand) {
        return Ok(not_evaluable(doc, "case_sets_differ"));
    }
    let all: Vec<&Run> = base.iter().chain(cand.iter()).collect();
    if !all.iter().all(|r| r.cost_known == J::Bool(true)) {
        return Ok(not_evaluable(doc, "cost_unknown"));
    }
    if !all.iter().all(|r| matches!(r.closed_early, J::Bool(_)) && matches!(r.status, J::Str(_))) {
        return Ok(not_evaluable(doc, "malformed_run"));
    }
    if !all.iter().all(|r| truthy(&r.oracle_ref)) {
        return Ok(not_evaluable(doc, "oracle_missing"));
    }

    // Both gates are always evaluated and reported; neither can be skipped.
    let results = [safety(&base, &cand), improvement(&base, &cand)];
    let gates: Vec<J> = GATES.iter().zip(results.iter()).map(|(g, (st, r))| gate_obj(g, st, *r)).collect();
    let v = if results.iter().all(|(st, _)| *st == "pass") {
        "pass"
    } else if results.iter().any(|(st, _)| *st == "fail") {
        "fail"
    } else {
        "not_evaluable"
    };
    Ok(emit(head(doc), v, gates))
}
