//! CMP stand-in: ChangeSpec -> DraftPlan compile for Replace Prompt plus Add EvalSuite over the seeded base world.
//! Mirrors the reviewed Python reference (e2e-core/src/claude_standin/compile_step.py) and the FRZ0 compile schemas
//! (contracts/engine-steps). Label: `compile=claude-standin` (output `compiler_label` = "claude-standin").
//!
//! std only: a minimal JSON reader/writer lives here. Output is canonical JSON (sorted keys, compact, ASCII).
//!
//! Denied reasons (closed set of the compile.out schema): `kind_not_supported`, `missing_precondition`,
//! `outside_bridge`, `mutable_reference`. Schema-invalid input is an `Err`, not a denial.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const LABEL: &str = "claude-standin";
const CONTRACT: &str = "engine-steps/0";
const BRIDGE_MAJOR: &str = "1";

#[derive(Debug, PartialEq, Eq)]
pub struct CompileError(pub String);

fn err<T>(m: impl Into<String>) -> Result<T, CompileError> {
    Err(CompileError(m.into()))
}

// ---------------------------------------------------------------- JSON

type Map = BTreeMap<String, Json>;

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Map),
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn lit(&mut self, w: &str, v: Json) -> Result<Json, CompileError> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            err("json: bad literal")
        }
    }
    fn value(&mut self) -> Result<Json, CompileError> {
        self.ws();
        match self.s.get(self.i) {
            None => err("json: unexpected end"),
            Some(b'{') => {
                self.i += 1;
                let mut m = Map::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return err("json: expected ':'");
                    }
                    self.i += 1;
                    let v = self.value()?;
                    m.insert(k, v);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return err("json: expected ',' or '}'"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    a.push(self.value()?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        _ => return err("json: expected ',' or ']'"),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let st = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                Ok(Json::Num(String::from_utf8_lossy(&self.s[st..self.i]).into_owned()))
            }
            _ => err("json: unexpected character"),
        }
    }
    fn hex4(&mut self) -> Result<u32, CompileError> {
        let h = self.s.get(self.i..self.i + 4).and_then(|b| std::str::from_utf8(b).ok());
        let v = h.and_then(|h| u32::from_str_radix(h, 16).ok());
        self.i += 4;
        v.ok_or_else(|| CompileError("json: bad \\u escape".into()))
    }
    fn string(&mut self) -> Result<String, CompileError> {
        if self.s.get(self.i) != Some(&b'"') {
            return err("json: expected string");
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return err("json: unterminated string") };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { return err("json: bad escape") };
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
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.s[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return err("json: bad surrogate");
                                }
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                            }
                            char::from_u32(cp).ok_or_else(|| CompileError("json: lone surrogate".into()))?
                        }
                        _ => return err("json: bad escape"),
                    };
                    let mut b = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                }
                c if c < 0x20 => return err("json: control character in string"),
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| CompileError("json: invalid utf-8".into()))
    }
}

fn parse(src: &str) -> Result<Json, CompileError> {
    let mut p = Parser { s: src.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return err("json: trailing data");
    }
    Ok(v)
}

fn write_str(v: &str, out: &mut String) {
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7f => {
                let mut u = [0u16; 2];
                for unit in c.encode_utf16(&mut u) {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write(v: &Json, out: &mut String) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(n) => out.push_str(n),
        Json::Str(x) => write_str(x, out),
        Json::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(x, out);
            }
            out.push(']');
        }
        Json::Obj(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(k, out);
                out.push(':');
                write(x, out);
            }
            out.push('}');
        }
    }
}

fn canon(v: &Json) -> String {
    let mut out = String::new();
    write(v, &mut out);
    out
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn st(v: &str) -> Json {
    Json::Str(v.to_string())
}

// ---------------------------------------------------------------- SHA-256

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in m.chunks(64) {
        let mut w = [0u32; 64];
        for (i, b) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let mj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(mj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

// ---------------------------------------------------------------- world

/// The seeded base world declaration (agent-core-assets/worlds/seeded-base.world.yaml), as constants.
pub struct World {
    pub world: &'static str,
    pub flow: &'static str,
    pub prompt_name: &'static str,
    pub prompt_version: &'static str,
    pub suite_name: &'static str,
    pub suite_version: &'static str,
    pub targetable_kinds: &'static [&'static str],
    pub root: PathBuf,
}

impl World {
    pub fn seeded_base() -> World {
        World {
            world: "attention-task",
            flow: "disputa-tarea",
            prompt_name: "resumen_radicado",
            prompt_version: "1.0.0",
            suite_name: "disputas-tarea-suite",
            suite_version: "1.0.0",
            targetable_kinds: &["replace_prompt", "add_eval_suite"],
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../agent-core-assets/worlds"),
        }
    }

    fn slot(&self, kind: &str) -> (&str, &str, PathBuf) {
        let dir = self.root.join(self.world);
        if kind == "prompt" {
            let f = format!("{}@{}.yaml", self.prompt_name, self.prompt_version);
            (self.prompt_name, self.prompt_version, dir.join("prompts/p").join(f))
        } else {
            let f = format!("{}@{}.yaml", self.suite_name, self.suite_version);
            (self.suite_name, self.suite_version, dir.join("eval_suites").join(f))
        }
    }

    /// sha256 of the LF-normalised asset file bytes.
    fn asset_digest(&self, kind: &str) -> Result<String, CompileError> {
        let (_, _, path) = self.slot(kind);
        let raw = std::fs::read(&path).map_err(|e| CompileError(format!("world asset {}: {e}", path.display())))?;
        let mut norm = Vec::with_capacity(raw.len());
        for (i, b) in raw.iter().enumerate() {
            if *b == b'\r' && raw.get(i + 1) == Some(&b'\n') {
                continue;
            }
            norm.push(*b);
        }
        Ok(format!("sha256:{}", sha256_hex(&norm)))
    }
}

// ---------------------------------------------------------------- schema checks (compile.in)

/// `^([a-z_]+):([A-Za-z0-9._-]+)@([0-9]+)$` -> (kind, name, major digits)
fn parse_ref(v: &str) -> Option<(&str, &str, &str)> {
    let (kind, rest) = v.split_once(':')?;
    let (name, major) = rest.rsplit_once('@')?;
    let ok = !kind.is_empty()
        && kind.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
        && !name.is_empty()
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !major.is_empty()
        && major.bytes().all(|c| c.is_ascii_digit());
    ok.then_some((kind, name, major))
}

fn is_ref(v: &str) -> bool {
    parse_ref(v).is_some()
}

fn num_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn is_digest(v: &str) -> bool {
    v.strip_prefix("sha256:").is_some_and(|h| h.len() == 64 && h.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')))
}

fn is_run_id(v: &str) -> bool {
    let b = v.as_bytes();
    (3..=64).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
}

fn as_obj<'a>(v: &'a Json, path: &str, req: &[&str], opt: &[&str]) -> Result<&'a Map, CompileError> {
    let Json::Obj(m) = v else { return err(format!("{path}: expected object")) };
    for r in req {
        if !m.contains_key(*r) {
            return err(format!("{path}: missing {r}"));
        }
    }
    for k in m.keys() {
        if !req.contains(&k.as_str()) && !opt.contains(&k.as_str()) {
            return err(format!("{path}: unexpected field {k}"));
        }
    }
    Ok(m)
}

fn str_field<'a>(m: &'a Map, k: &str, path: &str, ok: impl Fn(&str) -> bool) -> Result<&'a str, CompileError> {
    match m.get(k) {
        Some(Json::Str(v)) if ok(v) => Ok(v),
        _ => err(format!("{path}.{k}: invalid")),
    }
}

struct Op<'a> {
    op: &'a str,
    kind: &'a str,
    target: &'a str,
    new_ref: Option<&'a str>,
    digest: &'a str,
}

struct Spec<'a> {
    run_id: &'a str,
    data_class: &'a str,
    base_bundle_ref: &'a str,
    cs_bundle_ref: &'a str,
    bridge_ref: &'a str,
    ops: Vec<Op<'a>>,
    routes: Vec<&'a str>,
}

fn validate(doc: &Json) -> Result<Spec<'_>, CompileError> {
    let top = as_obj(
        doc,
        "$",
        &["contract_version", "step", "run_id", "data_class", "change_spec", "base_bundle_ref"],
        &[],
    )?;
    str_field(top, "contract_version", "$", |v| v == CONTRACT)?;
    str_field(top, "step", "$", |v| v == "compile")?;
    let run_id = str_field(top, "run_id", "$", is_run_id)?;
    let data_class = str_field(top, "data_class", "$", |v| ["synthetic", "treated", "e0", "original"].contains(&v))?;
    let base_bundle_ref = str_field(top, "base_bundle_ref", "$", is_ref)?;
    let p = "$.change_spec";
    let cs = as_obj(
        &top["change_spec"],
        p,
        &[
            "base_bundle_ref",
            "opportunity_ref",
            "workflow_bridge_ref",
            "operations",
            "expected_mechanism",
            "affected_routes",
            "rollback_ref",
        ],
        &[],
    )?;
    let cs_bundle_ref = str_field(cs, "base_bundle_ref", p, is_ref)?;
    str_field(cs, "opportunity_ref", p, is_ref)?;
    let bridge_ref = str_field(cs, "workflow_bridge_ref", p, is_ref)?;
    str_field(cs, "expected_mechanism", p, |v| !v.is_empty())?;
    str_field(cs, "rollback_ref", p, is_ref)?;
    let Json::Arr(ops_j) = &cs["operations"] else { return err("$.change_spec.operations: expected array") };
    if ops_j.is_empty() {
        return err("$.change_spec.operations: minItems 1");
    }
    let q = "$.change_spec.operations[]";
    let mut ops = Vec::new();
    for o in ops_j {
        let m = as_obj(o, q, &["op", "target_kind", "target_ref", "precondition_digest"], &["new_ref"])?;
        ops.push(Op {
            op: str_field(m, "op", q, |v| ["add", "replace", "disable"].contains(&v))?,
            kind: str_field(m, "target_kind", q, |v| ["prompt", "eval_suite"].contains(&v))?,
            target: str_field(m, "target_ref", q, is_ref)?,
            new_ref: if m.contains_key("new_ref") { Some(str_field(m, "new_ref", q, is_ref)?) } else { None },
            digest: str_field(m, "precondition_digest", q, is_digest)?,
        });
    }
    let Json::Arr(routes_j) = &cs["affected_routes"] else { return err("$.change_spec.affected_routes: expected array") };
    if routes_j.is_empty() {
        return err("$.change_spec.affected_routes: minItems 1");
    }
    let mut routes = Vec::new();
    for r in routes_j {
        match r {
            Json::Str(v) if !v.is_empty() => routes.push(v.as_str()),
            _ => return err("$.change_spec.affected_routes[]: invalid"),
        }
    }
    Ok(Spec { run_id, data_class, base_bundle_ref, cs_bundle_ref, bridge_ref, ops, routes })
}

// ---------------------------------------------------------------- compile

fn check(op: &Op, w: &World) -> Result<Option<&'static str>, CompileError> {
    let key = match (op.op, op.kind) {
        ("replace", "prompt") => "replace_prompt",
        ("add", "eval_suite") => "add_eval_suite",
        _ => "",
    };
    if !w.targetable_kinds.contains(&key) {
        return Ok(Some("kind_not_supported"));
    }
    let (name, version, _) = w.slot(op.kind);
    let slot_major = version.split('.').next().unwrap_or("");
    let Some((tk, tn, tm)) = parse_ref(op.target) else { return Ok(Some("outside_bridge")) };
    if tk != op.kind || tn != name || num_cmp(tm, slot_major) != Ordering::Equal {
        return Ok(Some("outside_bridge"));
    }
    if op.digest != w.asset_digest(tk)? {
        return Ok(Some("missing_precondition"));
    }
    match op.new_ref.and_then(parse_ref) {
        Some((nk, nn, nm)) if nk == tk && nn == tn && num_cmp(nm, tm) == Ordering::Greater => Ok(None),
        _ => Ok(Some("mutable_reference")),
    }
}

/// Compile with the default seeded base world and the local canonical digest.
pub fn run(input: &str) -> Result<String, CompileError> {
    run_with(input, &World::seeded_base(), None)
}

/// `dry_run` takes the canonical JSON of each operation and returns the plan digest of the real Core dry-run;
/// without it the digest is sha256 of the canonical operations array.
pub fn run_with(input: &str, w: &World, dry_run: Option<&dyn Fn(&[String]) -> String>) -> Result<String, CompileError> {
    let doc = parse(input)?;
    let spec = validate(&doc)?;
    let head = |status: &str| -> Map {
        let mut m = Map::new();
        m.insert("contract_version".into(), st(CONTRACT));
        m.insert("step".into(), st("compile"));
        m.insert("run_id".into(), st(spec.run_id));
        m.insert("data_class".into(), st(spec.data_class));
        m.insert("compiler_label".into(), st(LABEL));
        m.insert("status".into(), st(status));
        m
    };
    let mut reason = None;
    for op in &spec.ops {
        reason = check(op, w)?;
        if reason.is_some() {
            break;
        }
    }
    if reason.is_none() {
        let mut seen: Vec<&str> = Vec::new();
        for op in &spec.ops {
            if seen.contains(&op.target) {
                reason = Some("mutable_reference"); // two ops would publish the same new version of one target
            }
            seen.push(op.target);
        }
    }
    if reason.is_none() {
        let bridge = format!("bridge:{}@{}", w.flow, BRIDGE_MAJOR);
        if spec.routes.iter().any(|r| *r != w.flow) || spec.bridge_ref != bridge || spec.cs_bundle_ref != spec.base_bundle_ref {
            reason = Some("outside_bridge");
        }
    }
    if let Some(r) = reason {
        let mut m = head("denied");
        m.insert("denied_reason".into(), st(r));
        return Ok(canon(&Json::Obj(m)));
    }
    let ops: Vec<Json> = spec
        .ops
        .iter()
        .map(|o| {
            obj(vec![
                ("op", st(o.op)),
                ("target_kind", st(o.kind)),
                ("target_ref", st(o.target)),
                ("new_ref", st(o.new_ref.unwrap_or(""))),
                ("precondition_digest", st(o.digest)),
            ])
        })
        .collect();
    let digest = match dry_run {
        Some(f) => f(&ops.iter().map(canon).collect::<Vec<_>>()),
        None => format!("sha256:{}", sha256_hex(canon(&Json::Arr(ops.clone())).as_bytes())),
    };
    if !is_digest(&digest) {
        return err(format!("dry-run digest malformed: {digest:?}"));
    }
    let mut m = head("compiled");
    m.insert("draft_plan".into(), obj(vec![("operations", Json::Arr(ops)), ("digest", Json::Str(digest))]));
    Ok(canon(&Json::Obj(m)))
}
