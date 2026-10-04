//! Deterministic generator of platform-shaped event streams for sensor tests. It mirrors the R1S product_stream
//! scenarios (cells, base and planted rates, onset at 30% of the stream, volume ramp) without needing Python.
#![allow(dead_code)]

pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn f(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
    pub fn exp(&mut self, mean: f64) -> f64 {
        -mean * (1.0 - self.f()).ln()
    }
}

pub const CELLS: [(&str, &str); 4] = [("es", "app_chat"), ("es", "web_chat"), ("pt", "app_chat"), ("pt", "web_chat")];
const W: [f64; 4] = [0.4, 0.3, 0.15, 0.15];
pub const START: i64 = 1_788_249_600; // 2026-09-01T08:00:00Z

pub fn iso(t: i64) -> String {
    let days = t.div_euclid(86_400);
    let s = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scn {
    Null,
    Escalation,
    Recurrence,
    VolumeDrift,
}

pub struct Data {
    pub events: String,
    pub cases: String,
    /// every identifier present in the input (for the privacy assertion)
    pub ids: Vec<String>,
    pub n_cases: usize,
}

struct Ev {
    t: f64,
    ty: &'static str,
    case: String,
    role: &'static str,
    actor: String,
}

fn onset(frac: f64) -> bool {
    frac >= 0.30
}

pub fn make(scn: Scn, seed: u64, n_cases: usize) -> Data {
    make_spread(scn, seed, n_cases, 0)
}

/// Same stream (sequence order, cells, rates unchanged) with `event_time` stretched linearly so the stream spans at least `min_days`.
pub fn make_spread(scn: Scn, seed: u64, n_cases: usize, min_days: i64) -> Data {
    let mut r = Rng::new(seed);
    let target = ("pt", "web_chat");
    let mut evs: Vec<Ev> = vec![];
    let mut case_rows: Vec<String> = vec![];
    let mut ids = vec![];
    let mut t = START as f64;
    let mut made = 0usize;
    let mut idx = 0usize;
    let mut pending: Vec<(f64, usize, String)> = vec![]; // (time, cell, previous case id)
    while made < n_cases {
        let frac = made as f64 / n_cases as f64;
        let mult = if scn == Scn::VolumeDrift && onset(frac) { 1.0 + 3.0 * ((frac - 0.3) / 0.7).min(1.0) } else { 1.0 };
        t += r.exp(60.0 / mult);
        let (cell, prev, t0) = if let Some(p) = pending.iter().position(|(pt, _, _)| *pt <= t) {
            let (pt, c, id) = pending.remove(p);
            (c, Some(id), pt)
        } else {
            let u = r.f();
            let mut acc = 0.0;
            let mut c = 3;
            for (i, w) in W.iter().enumerate() {
                acc += w;
                if u < acc {
                    c = i;
                    break;
                }
            }
            (c, None, t)
        };
        made += 1;
        idx += 1;
        let cid = format!("CAS-{seed:x}{idx:06x}");
        let cust = format!("CUS-{:08x}", r.next_u64() as u32);
        let stf = format!("STF-{:04x}", r.next_u64() as u16);
        ids.extend([cid.clone(), cust.clone(), stf.clone()]);
        let lang = CELLS[cell].0;
        let is_t = CELLS[cell] == target;
        let prev_json = prev.as_ref().map_or("null".to_string(), |p| format!("\"{p}\""));
        case_rows.push(format!("{{\"case_id\":\"{cid}\",\"channel\":\"{}\",\"language\":\"{lang}\",\"priority\":\"medium\",\"previous_case_id\":{prev_json}}}", CELLS[cell].1));
        let slow = if scn == Scn::VolumeDrift && onset(frac) { 1.0 + (mult - 1.0) * if lang == "pt" { 4.0 } else { 0.5 } } else { 1.0 };
        let mut tt = t0;
        let ev = |t: f64, ty, role, actor: &str| Ev { t, ty, case: cid.clone(), role, actor: actor.to_string() };
        evs.push(ev(tt, "case.opened", "customer", &cust));
        tt += r.uniform(5.0, 40.0) * slow;
        evs.push(ev(tt, "case.assigned", "system", ""));
        tt += r.uniform(5.0, 40.0) * slow;
        evs.push(ev(tt, "turn.created", "analyst", &stf));
        evs.push(ev(tt, "case.first_responded", "analyst", &stf));
        let p_re = if scn == Scn::Escalation && is_t && onset(frac) { 0.32 } else { 0.05 };
        if r.f() < p_re {
            tt += r.uniform(60.0, 240.0);
            evs.push(ev(tt, "case.assigned", "supervisor", "STF-sup"));
        }
        tt += r.uniform(100.0, 600.0);
        evs.push(ev(tt, "case.closed", "analyst", &stf));
        let p_op = if scn == Scn::Recurrence && is_t && onset(frac) { 0.36 } else { 0.06 };
        if prev.is_none() && r.f() < p_op {
            pending.push((tt + r.uniform(120.0, 900.0), cell, cid.clone()));
        }
    }
    evs.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap().then(a.case.cmp(&b.case)));
    let span = evs.last().map_or(1.0, |e| e.t) - evs.first().map_or(0.0, |e| e.t);
    let scale = (min_days as f64 * 86_400.0 / span.max(1.0)).max(1.0);
    let mut out = String::new();
    for (i, e) in evs.iter().enumerate() {
        let actor = if e.actor.is_empty() { "null".to_string() } else { format!("\"{}\"", e.actor) };
        out.push_str(&format!(
            "{{\"ordinal\":{i},\"sequence\":{},\"event_id\":\"EVT-{seed:x}{i:07x}\",\"event_type\":\"{}\",\"entity\":\"case\",\"entity_id\":\"{}\",\"case_id\":\"{}\",\"actor_role\":\"{}\",\"actor_id\":{actor},\"event_time\":\"{}\"}}\n",
            i + 1,
            e.ty,
            e.case,
            e.case,
            e.role,
            iso((START as f64 + (e.t - START as f64) * scale) as i64)
        ));
    }
    Data { events: out, cases: case_rows.join("\n") + "\n", ids, n_cases }
}
