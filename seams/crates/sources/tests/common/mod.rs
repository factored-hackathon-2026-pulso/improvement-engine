#![allow(dead_code)]
//! Synthetic platform-shaped SQLite fixture. Shapes follow support-platform's tables (cases, turns, assignments, event_log,
//! login_accounts ...) but every value is invented; the `SECRET-*` sentinels prove that denied data never leaves the adapter.
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

pub const SENTINELS: &[&str] = &["SECRET-PW-HASH", "SECRET-TURN-TEXT", "SECRET-PAYLOAD", "SECRET-EMAIL", "SECRET-PREVIEW"];
static N: AtomicU32 = AtomicU32::new(0);

pub fn temp_path(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sources-test-{}-{}-{tag}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d); // stale dir from a recycled pid
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 7 events (sequence 1..=7), 2 cases, 2 turns, credential tables present.
pub fn fixture(dir: &std::path::Path, tenant: &str) -> PathBuf {
    let p = dir.join("platform.db");
    let c = Connection::open(&p).unwrap();
    c.execute_batch(&format!(
        "CREATE TABLE event_log(sequence INTEGER PRIMARY KEY, event_id TEXT NOT NULL, event_type TEXT NOT NULL, entity TEXT, entity_id TEXT,
           case_id TEXT, actor_role TEXT, actor_id TEXT, event_time TEXT NOT NULL, ingested_at TEXT, payload TEXT, tenant_id TEXT);
         CREATE TABLE cases(id TEXT PRIMARY KEY, customer_id TEXT, channel TEXT, language TEXT, priority TEXT, opened_at TEXT, sla_due_at TEXT,
           previous_case_id TEXT, tenant_id TEXT, status TEXT, search_text TEXT, last_message_preview TEXT);
         CREATE TABLE turns(id TEXT PRIMARY KEY, case_id TEXT, sequence INTEGER, kind TEXT, audience TEXT, author_role TEXT, created_at TEXT, text TEXT);
         CREATE TABLE staff(id TEXT PRIMARY KEY, roles TEXT, languages TEXT, team TEXT, team_id TEXT, active INTEGER, name TEXT, email TEXT);
         CREATE TABLE login_accounts(id TEXT PRIMARY KEY, email TEXT, password_hash TEXT);
         CREATE TABLE teams(id TEXT PRIMARY KEY, name TEXT);
         INSERT INTO login_accounts VALUES('l1','SECRET-EMAIL','SECRET-PW-HASH');
         INSERT INTO teams VALUES('t1','team one');
         INSERT INTO cases VALUES('case-1','cust-1','chat','es','normal','2026-10-01T10:00:00Z','2026-10-01T12:00:00Z',NULL,'{tenant}','open','SECRET-PREVIEW','SECRET-PREVIEW');
         INSERT INTO cases VALUES('case-2','cust-2','email','en','high','2026-10-01T11:00:00Z','2026-10-01T13:00:00Z','case-1','{tenant}','open','x','SECRET-PREVIEW');
         INSERT INTO turns VALUES('turn-1','case-1',1,'message','public','customer','2026-10-01T10:00:05Z','SECRET-TURN-TEXT');
         INSERT INTO turns VALUES('turn-2','case-1',2,'message','public','analyst','2026-10-01T10:05:00Z','SECRET-TURN-TEXT');
         INSERT INTO staff VALUES('st-1','analyst','es','team-a','team-a',1,'Fake Name','SECRET-EMAIL');"
    ))
    .unwrap();
    let ev = [
        ("case.opened", "case", "case-1", "customer", "2026-10-01T10:00:00Z"),
        ("turn.created", "turn", "turn-1", "customer", "2026-10-01T10:00:05Z"),
        ("case.assigned", "case", "case-1", "system", "2026-10-01T10:01:00Z"),
        ("turn.created", "turn", "turn-2", "analyst", "2026-10-01T10:05:00Z"),
        ("case.first_responded", "case", "case-1", "analyst", "2026-10-01T10:05:01Z"),
        ("case.opened", "case", "case-2", "customer", "2026-10-01T11:00:00Z"),
        ("case.closed", "case", "case-1", "analyst", "2026-10-01T11:30:00Z"),
    ];
    for (i, (t, e, id, role, time)) in ev.iter().enumerate() {
        let n = i + 1;
        let case = if *e == "turn" { "case-1" } else { id };
        c.execute(
            "INSERT INTO event_log VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            rusqlite::params![n as i64, format!("ev-{n}"), t, e, id, case, role, format!("actor-{n}"), time, time, "SECRET-PAYLOAD", tenant],
        )
        .unwrap();
    }
    p
}

/// Append `n` more events to an existing fixture (sequence continues).
pub fn append_events(p: &std::path::Path, from: i64, n: i64) {
    let c = Connection::open(p).unwrap();
    for s in from..from + n {
        c.execute(
            "INSERT INTO event_log VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            rusqlite::params![s, format!("ev-{s}"), "case.opened", "case", format!("case-x{s}"), format!("case-x{s}"), "customer", "a", "2026-10-02T09:00:00Z", "2026-10-02T09:00:00Z", "SECRET-PAYLOAD", "t"],
        )
        .unwrap();
    }
}
