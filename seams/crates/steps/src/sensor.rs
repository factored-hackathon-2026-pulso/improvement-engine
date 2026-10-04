//! STP1 sensor step (`semantics: claude-standin`): wraps the existing local-sim runner binary.
//! Also hosts the minimal std-only JSON reader/writer shared by `recompute` and `intent`
//! (serde is in the offline registry but adding it would touch Cargo files owned by other lanes).

use crate::StepError;

pub mod json {
    //! Minimal JSON value, parser and writer (std only).
    use crate::StepError;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Json {
        Null,
        Bool(bool),
        Int(i64),
        Float(f64),
        Str(String),
        Arr(Vec<Json>),
        Obj(Vec<(String, Json)>),
    }

    impl Json {
        pub fn get(&self, key: &str) -> Option<&Json> {
            match self {
                Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                _ => None,
            }
        }
        pub fn as_str(&self) -> Option<&str> {
            if let Json::Str(s) = self { Some(s) } else { None }
        }
        pub fn as_i64(&self) -> Option<i64> {
            if let Json::Int(i) = self { Some(*i) } else { None }
        }
        pub fn as_f64(&self) -> Option<f64> {
            match self {
                Json::Int(i) => Some(*i as f64),
                Json::Float(f) => Some(*f),
                _ => None,
            }
        }
        pub fn as_arr(&self) -> Option<&[Json]> {
            if let Json::Arr(a) = self { Some(a) } else { None }
        }
        pub fn obj(kv: Vec<(&str, Json)>) -> Json {
            Json::Obj(kv.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
        }
        pub fn s(v: &str) -> Json {
            Json::Str(v.to_string())
        }
        pub fn write(&self) -> String {
            let mut out = String::new();
            self.write_into(&mut out);
            out
        }
        fn write_into(&self, o: &mut String) {
            match self {
                Json::Null => o.push_str("null"),
                Json::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
                Json::Int(i) => o.push_str(&i.to_string()),
                Json::Float(f) => o.push_str(&format!("{f}")),
                Json::Str(s) => write_str(s, o),
                Json::Arr(a) => {
                    o.push('[');
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            o.push(',');
                        }
                        v.write_into(o);
                    }
                    o.push(']');
                }
                Json::Obj(kv) => {
                    o.push('{');
                    for (i, (k, v)) in kv.iter().enumerate() {
                        if i > 0 {
                            o.push(',');
                        }
                        write_str(k, o);
                        o.push(':');
                        v.write_into(o);
                    }
                    o.push('}');
                }
            }
        }
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

    pub fn parse(text: &str) -> Result<Json, StepError> {
        let mut p = P { b: text.as_bytes(), i: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(p.err("trailing characters"));
        }
        Ok(v)
    }

    struct P<'a> {
        b: &'a [u8],
        i: usize,
    }

    impl P<'_> {
        fn err(&self, m: &str) -> StepError {
            StepError::Invalid(format!("json: {m} at byte {}", self.i))
        }
        fn ws(&mut self) {
            while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            }
        }
        fn eat(&mut self, lit: &str) -> bool {
            if self.b[self.i..].starts_with(lit.as_bytes()) {
                self.i += lit.len();
                true
            } else {
                false
            }
        }
        fn value(&mut self) -> Result<Json, StepError> {
            self.ws();
            match self.b.get(self.i) {
                None => Err(self.err("unexpected end")),
                Some(b'{') => {
                    self.i += 1;
                    let mut kv = Vec::new();
                    self.ws();
                    if self.eat("}") {
                        return Ok(Json::Obj(kv));
                    }
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.ws();
                        if !self.eat(":") {
                            return Err(self.err("expected ':'"));
                        }
                        kv.push((k, self.value()?));
                        self.ws();
                        if self.eat(",") {
                            continue;
                        }
                        if self.eat("}") {
                            return Ok(Json::Obj(kv));
                        }
                        return Err(self.err("expected ',' or '}'"));
                    }
                }
                Some(b'[') => {
                    self.i += 1;
                    let mut a = Vec::new();
                    self.ws();
                    if self.eat("]") {
                        return Ok(Json::Arr(a));
                    }
                    loop {
                        a.push(self.value()?);
                        self.ws();
                        if self.eat(",") {
                            continue;
                        }
                        if self.eat("]") {
                            return Ok(Json::Arr(a));
                        }
                        return Err(self.err("expected ',' or ']'"));
                    }
                }
                Some(b'"') => Ok(Json::Str(self.string()?)),
                Some(_) if self.eat("true") => Ok(Json::Bool(true)),
                Some(_) if self.eat("false") => Ok(Json::Bool(false)),
                Some(_) if self.eat("null") => Ok(Json::Null),
                Some(_) => self.number(),
            }
        }
        fn number(&mut self) -> Result<Json, StepError> {
            let s = self.i;
            while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                self.i += 1;
            }
            let t = std::str::from_utf8(&self.b[s..self.i]).unwrap_or("");
            if t.is_empty() {
                return Err(self.err("unexpected character"));
            }
            if let Ok(i) = t.parse::<i64>() {
                return Ok(Json::Int(i));
            }
            t.parse::<f64>().map(Json::Float).map_err(|_| self.err("bad number"))
        }
        fn string(&mut self) -> Result<String, StepError> {
            if !self.eat("\"") {
                return Err(self.err("expected string"));
            }
            let mut out: Vec<u8> = Vec::new();
            loop {
                let c = *self.b.get(self.i).ok_or_else(|| self.err("unterminated string"))?;
                self.i += 1;
                match c {
                    b'"' => break,
                    b'\\' => {
                        let e = *self.b.get(self.i).ok_or_else(|| self.err("bad escape"))?;
                        self.i += 1;
                        let ch = match e {
                            b'"' => '"',
                            b'\\' => '\\',
                            b'/' => '/',
                            b'n' => '\n',
                            b'r' => '\r',
                            b't' => '\t',
                            b'b' => '\u{8}',
                            b'f' => '\u{c}',
                            b'u' => {
                                let h = self.b.get(self.i..self.i + 4).ok_or_else(|| self.err("bad \\u"))?;
                                let n = u32::from_str_radix(std::str::from_utf8(h).unwrap_or(""), 16)
                                    .map_err(|_| self.err("bad \\u"))?;
                                self.i += 4;
                                char::from_u32(n).unwrap_or('\u{fffd}')
                            }
                            _ => return Err(self.err("bad escape")),
                        };
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    c => out.push(c),
                }
            }
            String::from_utf8(out).map_err(|_| self.err("invalid utf-8"))
        }
    }
}

use json::Json;
use std::path::PathBuf;
use std::process::Command;

fn is_id(s: &str) -> bool {
    // ^[a-z0-9][a-z0-9._-]{2,63}$
    let b = s.as_bytes();
    (3..=64).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
}

/// `kind:id@rev` (^[a-z_]+:[A-Za-z0-9._-]+@[0-9]+$) -> id.
pub(crate) fn ref_id(r: &str) -> Result<&str, StepError> {
    let bad = || StepError::Invalid(format!("bad ref {r:?}"));
    let (kind, rest) = r.split_once(':').ok_or_else(bad)?;
    let (id, rev) = rest.rsplit_once('@').ok_or_else(bad)?;
    let ok = !kind.is_empty()
        && kind.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
        && !id.is_empty()
        && id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !rev.is_empty()
        && rev.bytes().all(|c| c.is_ascii_digit());
    if ok { Ok(id) } else { Err(bad()) }
}

/// Check the common envelope; returns (run_id, data_class).
pub(crate) fn envelope(v: &Json, step: &str) -> Result<(String, String), StepError> {
    let inv = |m: &str| StepError::Invalid(m.to_string());
    if v.get("contract_version").and_then(Json::as_str) != Some("engine-steps/0") {
        return Err(inv("contract_version must be engine-steps/0"));
    }
    if v.get("step").and_then(Json::as_str) != Some(step) {
        return Err(inv(&format!("step must be {step}")));
    }
    let run_id = v.get("run_id").and_then(Json::as_str).filter(|s| is_id(s)).ok_or_else(|| inv("bad run_id"))?;
    let dc = v
        .get("data_class")
        .and_then(Json::as_str)
        .filter(|s| matches!(*s, "synthetic" | "treated" | "e0" | "original"))
        .ok_or_else(|| inv("bad data_class"))?;
    Ok((run_id.to_string(), dc.to_string()))
}

pub(crate) fn valid_id(s: &str) -> bool {
    is_id(s)
}

pub(crate) fn env_path(var: &str) -> Result<PathBuf, StepError> {
    std::env::var_os(var).map(PathBuf::from).ok_or_else(|| StepError::Io(format!("{var} is not set")))
}

fn n(v: &Json, k: &str) -> Result<i64, StepError> {
    v.get(k).and_then(Json::as_i64).ok_or_else(|| StepError::Runner(format!("runner result lacks integer {k}")))
}

/// Run the sensor step: JSON in, JSON out. The runner path comes from STEPS_RUNNER_EXE and the
/// snapshot package from STEPS_SNAPSHOT_ROOT/<snapshot id>; optional STEPS_ARRANQUE (default 30)
/// and STEPS_MIN_SUPPORT (default 5) tune the sensor.
pub fn run(input: &str) -> Result<String, StepError> {
    let v = json::parse(input)?;
    let (run_id, data_class) = envelope(&v, "sensors")?;
    let inv = |m: &str| StepError::Invalid(m.to_string());
    let snap = ref_id(v.get("source_snapshot_ref").and_then(Json::as_str).ok_or_else(|| inv("source_snapshot_ref"))?)?;
    ref_id(v.get("discovery_config_ref").and_then(Json::as_str).ok_or_else(|| inv("discovery_config_ref"))?)?;
    let end = v.get("window").and_then(|w| w.get("end")).and_then(Json::as_str).ok_or_else(|| inv("window.end"))?;
    v.get("window").and_then(|w| w.get("start")).and_then(Json::as_str).ok_or_else(|| inv("window.start"))?;
    let specs = v.get("metric_spec_refs").and_then(Json::as_arr).ok_or_else(|| inv("metric_spec_refs"))?;
    if specs.is_empty() {
        return Err(inv("metric_spec_refs must not be empty"));
    }
    for s in specs {
        ref_id(s.as_str().ok_or_else(|| inv("metric_spec_refs item"))?)?;
    }

    let exe = env_path("STEPS_RUNNER_EXE")?;
    if !exe.is_file() {
        return Err(StepError::Io(format!("runner exe not found: {}", exe.display())));
    }
    let package = env_path("STEPS_SNAPSHOT_ROOT")?.join(snap);
    let out_dir = std::env::temp_dir().join(format!("steps-sensor-{run_id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).map_err(|e| StepError::Io(e.to_string()))?;
    let arranque = std::env::var("STEPS_ARRANQUE").unwrap_or_else(|_| "30".into());
    let min_support = std::env::var("STEPS_MIN_SUPPORT").unwrap_or_else(|_| "5".into());
    let done = Command::new(&exe)
        .args(["local-sim", "--mode", "local-simulation", "--source", "e0", "--input"])
        .arg(&package)
        .arg("--output")
        .arg(&out_dir)
        .args(["--tenant-id", "pulso_local", "--observed-cutoff"])
        .arg(format!("{end}T00:00:00Z"))
        .args(["--arranque-cases", &arranque, "--min-recurring-query-cases", &min_support])
        .output()
        .map_err(|e| StepError::Io(format!("cannot start runner: {e}")))?;
    let result = (|| {
        if !done.status.success() {
            let err = String::from_utf8_lossy(&done.stderr);
            let tail: String = err.chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
            return Err(StepError::Runner(format!("sensor failed: {tail}")));
        }
        let run_dir = std::fs::read_dir(&out_dir)
            .map_err(|e| StepError::Io(e.to_string()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .ok_or_else(|| StepError::Runner("runner produced no run directory".into()))?;
        let text = std::fs::read_to_string(run_dir.join("result.json")).map_err(|e| StepError::Io(e.to_string()))?;
        json::parse(&text)
    })();
    let _ = std::fs::remove_dir_all(&out_dir);
    project(&run_id, &data_class, &result?)
}

/// Aggregate-only projection of the runner result onto the sensors out schema.
fn project(run_id: &str, data_class: &str, r: &Json) -> Result<String, StepError> {
    let bad = |m: &str| StepError::Runner(m.to_string());
    let sig = r.get("signal").ok_or_else(|| bad("runner result lacks signal"))?;
    let digest = sig.get("digest").and_then(Json::as_str);
    let metric = sig.get("metric_id").and_then(Json::as_str).filter(|m| is_id(m)).ok_or_else(|| bad("bad metric_id"))?;
    let holdout = r.get("e0_recurrence_holdout").and_then(|h| h.get("status")).and_then(Json::as_str).is_some();
    let signal = Json::obj(vec![
        ("signal_id", Json::s("sig-0001")),
        ("metric_id", Json::s(metric)),
        ("population", Json::s("e0/copilot_query")),
        ("numerator", Json::Int(n(sig, "numerator")?)),
        ("denominator", Json::Int(n(sig, "denominator")?)),
        ("holdout_checked", Json::Bool(holdout)),
        ("evidence_ref", Json::s("ev-0001")),
    ]);
    let mut discards = Vec::new();
    for s in r.get("signals").and_then(Json::as_arr).unwrap_or(&[]) {
        if s.get("digest").and_then(Json::as_str) == digest {
            continue;
        }
        let m = s.get("metric_id").and_then(Json::as_str).filter(|m| is_id(m)).ok_or_else(|| bad("bad discard metric_id"))?;
        let reason = if n(s, "numerator")? < n(s, "minimum_support")? { "below_k" } else { "other" };
        discards.push(Json::obj(vec![("metric_id", Json::s(m)), ("reason", Json::s(reason))]));
    }
    Ok(Json::obj(vec![
        ("contract_version", Json::s("engine-steps/0")),
        ("step", Json::s("sensors")),
        ("run_id", Json::s(run_id)),
        ("data_class", Json::s(data_class)),
        ("signals", Json::Arr(vec![signal])),
        ("discards", Json::Arr(discards)),
    ])
    .write())
}
