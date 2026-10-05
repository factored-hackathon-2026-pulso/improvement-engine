//! Allow-list of the `platform_live` source (mirror of `platform-exporter/.../policy.py`, drift-tested).
//! Every query of every adapter is built from names that passed these assertions BEFORE any SQL text exists.
use crate::SourceError;

pub const HARD_CAP: usize = 10_000;
pub const DENIED_TABLES: &[&str] = &["login_accounts", "mfa_challenges", "staff_sessions"];
/// `event_log` columns the adapters read. `payload`, `tenant_id` and `ingested_at` are allow-listed but deliberately not
/// read: the sensor needs no payload and payload free-text treatment is an open product gap (G9).
pub const EVENT_READ_COLUMNS: &[&str] =
    &["sequence", "event_id", "event_type", "entity", "entity_id", "case_id", "actor_role", "actor_id", "event_time"];

const ALLOWED: &[(&str, &[&str])] = &[
    ("event_log", &["sequence", "event_id", "event_type", "entity", "entity_id", "case_id", "actor_role", "actor_id", "event_time", "ingested_at", "payload", "tenant_id"]),
    ("cases", &["id", "customer_id", "channel", "language", "priority", "opened_at", "sla_due_at", "previous_case_id", "rating_score", "rated_at", "tenant_id"]),
    ("customers", &["id", "simulator"]),
    ("staff", &["id", "roles", "languages", "team", "team_id", "active"]),
    ("turns", &["id", "case_id", "sequence", "kind", "audience", "author_role", "created_at"]),
    ("assignments", &["id", "case_id", "staff_id", "reason", "policy_rule_id", "strategy", "open_cases_at_assignment", "waited_seconds", "previous_staff_id", "paused_override", "assigned_at"]),
    ("customer_case_slots", &["customer_id", "open_case_id"]),
];

pub fn allowed_tables() -> Vec<&'static str> {
    ALLOWED.iter().map(|(t, _)| *t).collect()
}

pub fn allowed_columns(table: &str) -> Option<&'static [&'static str]> {
    ALLOWED.iter().find(|(t, _)| *t == table).map(|(_, c)| *c)
}

fn norm(name: &str) -> String {
    name.trim().trim_matches(|c| matches!(c, '"' | '`' | '[' | ']')).to_ascii_lowercase()
}

fn denied(m: String) -> SourceError {
    SourceError::AccessDenied(m)
}

/// Returns the normalised table name; refuses denied, unknown or non-identifier names.
pub fn assert_table_allowed(t: &str) -> Result<String, SourceError> {
    let n = norm(t);
    let ident = !n.is_empty() && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if DENIED_TABLES.contains(&n.as_str()) || !ident || allowed_columns(&n).is_none() {
        return Err(denied(format!("table not allowed: {n:?}")));
    }
    Ok(n)
}

pub fn assert_columns_allowed(t: &str, cols: &[&str]) -> Result<(), SourceError> {
    let t = assert_table_allowed(t)?;
    let ok = allowed_columns(&t).unwrap_or(&[]);
    let bad: Vec<String> = cols.iter().map(|c| norm(c)).filter(|c| !ok.contains(&c.as_str())).collect();
    if bad.is_empty() { Ok(()) } else { Err(denied(format!("columns not allowed on {t}: {bad:?}"))) }
}

pub fn check_limit(n: usize) -> Result<usize, SourceError> {
    if (1..=HARD_CAP).contains(&n) { Ok(n) } else { Err(SourceError::BadLimit(n)) }
}
