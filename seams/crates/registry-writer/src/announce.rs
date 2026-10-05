//! ANN1: after agent-core accepted an `announced` proposal, tell the support platform (PR 17, ADR 0007) so supervisors get ONE
//! notification. The engine proposes, the platform relays, humans decide: nothing here approves or publishes.
//!
//! `announce_payload` builds the exact body of `POST /api/v1/internal/builder/proposals/announce` from the dossier and validates every
//! platform bound locally (lengths, no email, no run of 9 or more digits, at most 8 distinct `CASE-` ids), so the engine never gets a
//! 422 for its own mistake. Over-long texts are CUT here (the platform never truncates); personal-data shapes are REFUSED.
//!
//! Evidence links are OPAQUE ids derived from the finding's evidence ref (a hash of aggregate numbers, never a customer or case id of
//! the platform): they have the `CASE-<26 Crockford>` shape the route demands but name no real case.
//!
//! `Announcer` is best effort and never fails the agent-core delivery: its result is a record string, `platform_announced` or
//! `platform_announce_failed:<class>`. The token is read from the environment, redacted in `Debug`, never put in a record.
//! No trace headers: this base has no `core_client::trace` helper, so no `traceparent`/`baggage` is sent.
use crate::guard;
use crate::transport::{HttpTransport, Reply, Request, Transport, TransportError};
use core_client::authorizer::Jws;
use reasoning::finding::Finding;
use serde_json::{Value, json};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

pub const ROUTE: &str = "/api/v1/internal/builder/proposals/announce";
pub const MAX_PROPOSAL_ID: usize = 64;
pub const MAX_TITLE: usize = 120;
pub const MAX_PROBLEM: usize = 600;
pub const MAX_EVIDENCE: usize = 600;
pub const MAX_EFFECT: usize = 400;
pub const MAX_LINKS: usize = 8;
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A local refusal: the payload would not be accepted by the platform (or carries personal data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    pub field: &'static str,
    pub why: &'static str,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.why)
    }
}

/// The platform's `x@y.<letters>` rule (an `id@1.0.0` artifact reference is allowed).
pub fn email_shaped(s: &str) -> bool {
    let cs: Vec<char> = s.chars().collect();
    for (i, c) in cs.iter().enumerate() {
        if *c != '@' || i == 0 || cs[i - 1].is_whitespace() || cs[i - 1] == '@' {
            continue;
        }
        let run: Vec<char> = cs[i + 1..].iter().copied().take_while(|c| !c.is_whitespace() && *c != '@').collect();
        for j in 1..run.len() {
            if run[j] == '.' && run.get(j + 1).is_some_and(char::is_ascii_alphabetic) && run.get(j + 2).is_some_and(char::is_ascii_alphabetic) {
                return true;
            }
        }
    }
    false
}

/// The platform's `(\d[ -.]?){9,}` rule: nine or more digits, a single space, hyphen or dot allowed between them.
pub fn long_number(s: &str) -> bool {
    let (mut run, mut sep) = (0usize, false);
    for c in s.chars() {
        if c.is_ascii_digit() {
            run += 1;
            sep = false;
            if run >= 9 {
                return true;
            }
        } else if matches!(c, ' ' | '-' | '.') && run > 0 && !sep {
            sep = true;
        } else {
            run = 0;
            sep = false;
        }
    }
    false
}

fn text(field: &'static str, raw: &str, max: usize) -> Result<String, Invalid> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(Invalid { field, why: "empty" });
    }
    let t = reasoning::dossier::cut_title(t, max);
    if email_shaped(&t) || long_number(&t) {
        return Err(Invalid { field, why: "personal-data shape (email or 9+ digit run)" });
    }
    Ok(t)
}

/// Opaque `CASE-` id (26 Crockford chars) derived from the evidence ref and an index; deterministic, resolves to no case.
pub fn opaque_link(evidence_ref: &str, i: usize) -> String {
    let hex = steps::compile::sha256_hex(format!("ann1|{evidence_ref}|{i}").as_bytes());
    let body: String = hex
        .as_bytes()
        .chunks(2)
        .take(26)
        .map(|p| CROCKFORD[(u8::from_str_radix(std::str::from_utf8(p).unwrap_or("00"), 16).unwrap_or(0) & 0x1f) as usize] as char)
        .collect();
    format!("CASE-{body}")
}

pub fn valid_case_id(s: &str) -> bool {
    s.strip_prefix("CASE-").is_some_and(|b| b.len() == 26 && b.bytes().all(|c| CROCKFORD.contains(&c)))
}

/// The Spanish dossier (`dossier/1`, `announce: true`) as the platform body. `proposal_ref` is agent-core's proposal id.
pub fn announce_payload(finding: &Finding, proposal_ref: &str, dossier: &Value) -> Result<Value, Invalid> {
    let id = proposal_ref.trim();
    if id.is_empty() || id.chars().count() > MAX_PROPOSAL_ID || !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@')) {
        return Err(Invalid { field: "proposalId", why: "1-64 id characters required" });
    }
    if dossier["announce"].as_bool() != Some(true) {
        return Err(Invalid { field: "dossier", why: "not an announced dossier" });
    }
    let es = &dossier["es"];
    let sec = &es["sections"];
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let links: Vec<String> = vec![opaque_link(&finding.evidence_ref(), 0)];
    if links.len() > MAX_LINKS || links.iter().any(|l| !valid_case_id(l)) {
        return Err(Invalid { field: "evidenceLinks", why: "case ids only, at most 8" });
    }
    Ok(json!({
        "proposalId": id,
        "title": text("title", &s(&es["title"]), MAX_TITLE)?,
        "problem": text("problem", &s(&sec["problem"]), MAX_PROBLEM)?,
        "evidence": text("evidence", &s(&sec["evidence"]), MAX_EVIDENCE)?,
        "expectedEffect": text("expectedEffect", &s(&sec["expected_effect"]), MAX_EFFECT)?,
        "evidenceLinks": links,
    }))
}

/// `PULSO_PLATFORM_URL` -> `host:port`: plain http, a loopback or private (RFC 1918 / ULA / link-local) IP literal or `localhost`,
/// explicit or default port, no path, query or credentials.
pub fn parse_platform_url(url: &str) -> Result<String, String> {
    let rest = url.trim().strip_prefix("http://").ok_or("PULSO_PLATFORM_URL must be http://host:port (plain HTTP, loopback or private network only)")?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', ' ']) {
        return Err("PULSO_PLATFORM_URL must be http://host:port with no path, query or credentials".into());
    }
    let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
        let (h, tail) = v6.split_once(']').ok_or("PULSO_PLATFORM_URL has a bad IPv6 literal")?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse::<u16>().map_err(|_| "PULSO_PLATFORM_URL has a bad port")?,
            None => 80,
        };
        (h, port)
    } else {
        match rest.split_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "PULSO_PLATFORM_URL has a bad port")?),
            None => (rest, 80),
        }
    };
    let private = host == "localhost"
        || host.parse::<IpAddr>().is_ok_and(|ip| match ip {
            IpAddr::V4(v) => v.is_loopback() || v.is_private() || v.is_link_local(),
            IpAddr::V6(v) => v.is_loopback() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80,
        });
    if !private {
        return Err("PULSO_PLATFORM_URL must be a loopback or private-network address".into());
    }
    Ok(if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") })
}

/// Result of one announcement; `record()` is the string kept on the finding record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnounceOutcome {
    /// `None` = announced.
    pub failure: Option<&'static str>,
    pub attempts: u32,
}

impl AnnounceOutcome {
    pub fn announced(&self) -> bool {
        self.failure.is_none()
    }
    pub fn record(&self) -> String {
        match self.failure {
            None => "platform_announced".into(),
            Some(c) => format!("platform_announce_failed:{c}"),
        }
    }
}

pub struct Announcer {
    transport: Arc<dyn Transport + Send + Sync>,
    token: Jws,
    pub max_attempts: u32,
    pub backoff: Duration,
    pub sleep: fn(Duration),
}

impl std::fmt::Debug for Announcer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Announcer {{ token: <redacted>, max_attempts: {} }}", self.max_attempts)
    }
}

impl Announcer {
    pub fn new(transport: Arc<dyn Transport + Send + Sync>, token: Jws) -> Announcer {
        Announcer { transport, token, max_attempts: 3, backoff: Duration::from_millis(500), sleep: std::thread::sleep }
    }

    /// `PULSO_ANNOUNCE_TO_PLATFORM` (`on|off`), `PULSO_PLATFORM_URL`, `PULSO_PLATFORM_SERVICE_TOKEN`. Unset flag: ON only when URL and
    /// token are both set (the live path), else OFF; `off` always wins; `on` without both is refused at start, naming the variable.
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Result<Option<Announcer>, String> {
        let url = get("PULSO_PLATFORM_URL").filter(|v| !v.trim().is_empty());
        let token = get("PULSO_PLATFORM_SERVICE_TOKEN").filter(|v| !v.is_empty());
        let on = match get("PULSO_ANNOUNCE_TO_PLATFORM").as_deref().map(str::trim).unwrap_or("") {
            "off" | "0" | "false" => return Ok(None),
            "on" | "1" | "true" => true,
            "" => url.is_some() && token.is_some(),
            _ => return Err("PULSO_ANNOUNCE_TO_PLATFORM is not on|off".into()),
        };
        if !on {
            return Ok(None);
        }
        let url = url.ok_or("PULSO_PLATFORM_URL is required with PULSO_ANNOUNCE_TO_PLATFORM=on")?;
        let token = token.ok_or("PULSO_PLATFORM_SERVICE_TOKEN is required with PULSO_ANNOUNCE_TO_PLATFORM=on")?;
        let addr = parse_platform_url(&url)?;
        Ok(Some(Announcer::new(Arc::new(HttpTransport::new(&addr, Duration::from_secs(15))), Jws::new(token))))
    }

    /// Builds, validates and sends. Never panics, never errors: the outcome is a closed record.
    pub fn announce(&self, finding: &Finding, proposal_id: &str, dossier: &Value) -> AnnounceOutcome {
        let fail = |c, attempts| AnnounceOutcome { failure: Some(c), attempts };
        if self.token.reveal().is_empty() {
            return fail("token_missing", 0);
        }
        let Ok(body) = announce_payload(finding, proposal_id, dossier) else { return fail("invalid_payload", 0) };
        if !guard::platform_allowed("POST", ROUTE) {
            return fail("forbidden_operation", 0);
        }
        let key = format!("announce:{proposal_id}");
        let tries = self.max_attempts.max(1);
        for attempt in 1..=tries {
            let req = Request { method: "POST", path: ROUTE.into(), bearer: &self.token, idempotency_key: Some(&key), body: Some(body.clone()) };
            match self.transport.send(&req) {
                Ok(Reply { status: 200 | 201, body: b }) => {
                    if b["proposalId"].as_str().is_some_and(|p| p != proposal_id) {
                        return fail("response_mismatch", attempt);
                    }
                    return AnnounceOutcome { failure: None, attempts: attempt };
                }
                Ok(Reply { status: 401, .. }) => return fail("unauthorized", attempt),
                Ok(Reply { status: 404, .. }) => return fail("not_found", attempt),
                Ok(Reply { status: 422, .. }) => return fail("rejected", attempt),
                Ok(Reply { status, .. }) if status < 500 => return fail("unexpected_status", attempt),
                Ok(_) | Err(TransportError::NotSent(_) | TransportError::Unknown(_)) => {}
            }
            if attempt < tries {
                (self.sleep)(self.backoff * attempt);
            }
        }
        fail("unavailable", tries)
    }
}
