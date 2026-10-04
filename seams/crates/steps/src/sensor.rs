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

/// Run the sensor step: JSON in, JSON out.
pub fn run(_input: &str) -> Result<String, StepError> {
    Err(StepError::Runner("not implemented".into()))
}
