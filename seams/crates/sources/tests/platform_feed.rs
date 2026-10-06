//! EVT2 REAL FEED: the monitor tick writes the platform events of contract 1.3.0 into a package the cells aggregator reads
//! (`scripts/aggregate/platform_event_cells.py --package-root`). With `platform_cells = on` the package keeps the allow-listed
//! payload keys of the 12 event types the cells need, and `cases.ndjson` carries `case_type`, `opened_at` and a salted hash of the
//! customer (`customer_key`): no customer id, no analyst id, no free text. Off (default) the package format is the old one.
mod common;
use common::temp_path;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sources::config::Config;
use sources::monitor::{TickOutcome, tick};
use sources::sqlite::SqliteProduct;
use sources::store::FileStore;
use sources::{DataMode, SourceId};
use std::path::{Path, PathBuf};

const RUNNER: &str = env!("CARGO_BIN_EXE_sources-synth-runner");
const ID: &str = "platform:evt2";
const SALT: &str = "salt-for-the-evt2-tests";

struct Row {
    case: String,
    customer: String,
    case_type: &'static str,
    channel: &'static str,
    decision: &'static str,
}

fn schema(c: &Connection) {
    c.execute_batch(
        "CREATE TABLE event_log(sequence INTEGER PRIMARY KEY, event_id TEXT NOT NULL, event_type TEXT NOT NULL, entity TEXT, entity_id TEXT,
           case_id TEXT, actor_role TEXT, actor_id TEXT, event_time TEXT NOT NULL, ingested_at TEXT, payload TEXT, tenant_id TEXT);
         CREATE TABLE cases(id TEXT PRIMARY KEY, customer_id TEXT, channel TEXT, language TEXT, priority TEXT, opened_at TEXT, sla_due_at TEXT,
           previous_case_id TEXT, tenant_id TEXT, case_type TEXT);",
    )
    .unwrap();
}

fn insert_event(c: &Connection, seq: i64, etype: &str, case: &str, payload: Option<&str>) {
    c.execute(
        "INSERT INTO event_log(sequence,event_id,event_type,entity,entity_id,case_id,actor_role,actor_id,event_time,payload) VALUES(?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![seq, format!("EVT-{seq}"), etype, "case", case, case, "system", "STF-0001", format!("2026-09-{:02}T10:00:00Z", 1 + seq % 20), payload],
    )
    .unwrap();
}

/// One case per row: opened, suggestion ready, decided. `n` rows, customers repeat every 7 rows.
fn build_db(dir: &Path, rows: &[Row]) -> PathBuf {
    let p = dir.join("product.db");
    let c = Connection::open(&p).unwrap();
    schema(&c);
    let tx = c.unchecked_transaction().unwrap();
    let mut seq = 0i64;
    for r in rows {
        tx.execute(
            "INSERT INTO cases(id,customer_id,channel,language,priority,opened_at,case_type) VALUES(?,?,?,?,?,?,?)",
            rusqlite::params![r.case, r.customer, r.channel, "es", "medium", "2026-09-01T09:00:00Z", r.case_type],
        )
        .unwrap();
        seq += 1;
        insert_event(&tx, seq, "case.opened", &r.case, Some("{}"));
        seq += 1;
        insert_event(&tx, seq, "copilot.suggestion_ready", &r.case, Some(&json!({"analyst_id": "STF-0001", "agent": "copiloto-sugerencias@1.0.0", "kinds": ["reply"], "count": 1, "run_id": "run-1", "trace_id": "tr-1", "release": "rel-a"}).to_string()));
        seq += 1;
        let dec = json!({"subject": "reply", "decision": r.decision, "edit_distance_permille": null, "turn_id": "TRN-9", "agent": "copiloto-sugerencias@1.0.0", "release": "rel-a",
                         "note": "hola mi correo es ana@example.test", "analyst_id": "STF-0001"});
        insert_event(&tx, seq, "copilot.suggestion_decided", &r.case, Some(&dec.to_string()));
        seq += 1;
        insert_event(&tx, seq, "turn.created", &r.case, Some(&json!({"text": "SECRET-TURN-TEXT"}).to_string()));
    }
    tx.commit().unwrap();
    p
}

fn rows(n: usize) -> Vec<Row> {
    (0..n)
        .map(|i| Row {
            case: format!("CASE-{i:05}"),
            customer: format!("CUS-{:04}", i % 7),
            case_type: if i % 2 == 0 { "service_quality" } else { "app_issue" },
            channel: "app_chat",
            decision: if i % 3 == 0 { "used" } else { "discarded" },
        })
        .collect()
}

fn cfg(work: &Path, extra: &[(&str, &str)]) -> Config {
    let mut pairs = vec![
        ("data_mode", "platform"),
        ("adapter", "product-sqlite"),
        ("source_id", ID),
        ("work_dir", work.to_str().unwrap()),
        ("runner_exe", RUNNER),
        ("batch_cap", "5000"),
    ];
    for (k, v) in extra {
        pairs.retain(|(e, _)| e != k);
        pairs.push((k, v));
    }
    Config::from_pairs(&pairs).unwrap()
}

fn run_all(db: &Path, c: &Config, work: &Path) -> Vec<Value> {
    let id = SourceId::new(DataMode::Platform, ID).unwrap();
    let a = if c.platform_cells { SqliteProduct::open_platform(db, id) } else { SqliteProduct::open(db, id) }.unwrap();
    let store = FileStore::open(work.join("wm")).unwrap();
    let mut out = vec![];
    while let TickOutcome::Processed { record, more, .. } = tick(c, &a, &store).unwrap() {
        out.push(record);
        if !more {
            break;
        }
    }
    out
}

fn lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn packages(work: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(work.join("packages")).unwrap().map(|e| e.unwrap().path()).collect();
    v.sort();
    v
}

fn keys(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort_unstable();
    k
}

#[test]
fn the_config_keys_are_validated_and_the_salt_never_shows_in_debug_output() {
    let w = temp_path("cfg");
    let c = cfg(&w, &[("platform_cells", "on"), ("customer_key_salt", SALT)]);
    assert!(!format!("{c:?}").contains(SALT), "a secret salt must not be printable");
    for bad in [("platform_cells", "maybe"), ("customer_key_salt", "")] {
        let r = Config::from_pairs(&[("data_mode", "platform"), ("adapter", "product-sqlite"), ("source_id", ID), ("work_dir", "x"), ("runner_exe", "y"), bad]);
        assert!(r.is_err(), "{bad:?}");
    }
    assert!(Config::from_pairs(&[("data_mode", "platform"), ("adapter", "product-sqlite"), ("source_id", ID), ("work_dir", "x"), ("runner_exe", "y"), ("customer_key_salt", SALT)]).is_err(), "a salt without platform_cells is a misconfiguration");
}

#[test]
fn the_package_keeps_only_the_allow_listed_payload_keys_of_the_platform_event_types() {
    let dir = temp_path("payload");
    let db = build_db(&dir, &rows(40));
    let work = dir.join("work");
    let recs = run_all(&db, &cfg(&work, &[("platform_cells", "on"), ("customer_key_salt", SALT)]), &work);
    assert!(!recs.is_empty());
    let pkg = packages(&work).remove(0);
    let ev = lines(&pkg.join("events.ndjson"));
    let by = |t: &str| -> Vec<&Value> { ev.iter().filter(|e| e["event_type"] == t).collect() };
    let dec = by("copilot.suggestion_decided");
    assert_eq!(dec.len(), 40);
    assert_eq!(keys(&dec[0]["payload"]), ["agent", "decision", "release", "subject"], "the null distance is dropped, free text and ids are never kept");
    assert_eq!(dec[0]["payload"]["agent"], "copiloto-sugerencias@1.0.0");
    let ready = by("copilot.suggestion_ready");
    assert_eq!(keys(&ready[0]["payload"]), ["agent", "release"], "analyst_id, run_id, trace_id, kinds, count are not read");
    assert!(by("turn.created").iter().all(|e| e.get("payload").is_none()), "event types the cells do not need carry no payload at all");
    let text = std::fs::read_to_string(pkg.join("events.ndjson")).unwrap();
    for needle in ["SECRET-TURN-TEXT", "ana@example.test", "hola mi correo", "TRN-9", "run-1"] {
        assert!(!text.contains(needle), "{needle} leaked into the package");
    }
}

#[test]
fn an_unsafe_payload_value_is_dropped_not_kept_and_a_broken_payload_never_stops_the_tick() {
    let dir = temp_path("unsafe");
    let mut r = rows(12);
    r.truncate(12);
    let db = build_db(&dir, &r);
    {
        let c = Connection::open(&db).unwrap();
        insert_event(&c, 1000, "copilot.suggestion_decided", "CASE-00000", Some(&json!({"subject": "reply", "decision": "used", "agent": "CASE-00001 call me", "release": "rel-a"}).to_string()));
        insert_event(&c, 1001, "copilot.suggestion_decided", "CASE-00000", Some("{not json"));
        insert_event(&c, 1002, "copilot.suggestion_decided", "CASE-00000", None);
        insert_event(&c, 1003, "case.type_changed", "CASE-00000", Some(&json!({"from": "none", "to": "a made up type", "by": "STF-0001"}).to_string()));
        insert_event(&c, 1004, "copilot.tool_used", "CASE-00000", Some(&json!({"tool": "consultar_cargos", "analyst_id": "STF-0001"}).to_string()));
        insert_event(&c, 1005, "assistant.ended", "CASE-00000", Some(&json!({"result": "escalated", "reason": "customer typed his card"}).to_string()));
    }
    let work = dir.join("work");
    run_all(&db, &cfg(&work, &[("platform_cells", "on"), ("customer_key_salt", SALT)]), &work);
    let ev: Vec<Value> = packages(&work).iter().flat_map(|p| lines(&p.join("events.ndjson"))).collect();
    let at = |seq: i64| ev.iter().find(|e| e["sequence"] == seq).unwrap();
    assert_eq!(keys(&at(1000)["payload"]), ["decision", "release", "subject"], "the agent value that looks like an id plus text is dropped");
    assert!(at(1001).get("payload").is_none() && at(1002).get("payload").is_none());
    assert_eq!(keys(&at(1003)["payload"]), ["from"], "an unknown case type is dropped");
    assert_eq!(at(1004)["payload"], json!({"tool": "consultar_cargos"}));
    assert_eq!(at(1005)["payload"], json!({"result": "escalated"}));
}

#[test]
fn cases_carry_case_type_opened_at_and_a_salted_customer_hash_never_the_id() {
    let dir = temp_path("cases");
    let db = build_db(&dir, &rows(40));
    let work = dir.join("work");
    let c1 = cfg(&work, &[("platform_cells", "on"), ("customer_key_salt", SALT)]);
    run_all(&db, &c1, &work);
    let cases: Vec<Value> = packages(&work).iter().flat_map(|p| lines(&p.join("cases.ndjson"))).collect();
    assert_eq!(cases.len(), 40);
    assert_eq!(keys(&cases[0]), ["case_id", "case_type", "channel", "customer_key", "language", "opened_at", "previous_case_id", "priority"]);
    let c0 = cases.iter().find(|c| c["case_id"] == "CASE-00000").unwrap();
    assert_eq!(c0["case_type"], "service_quality");
    assert_eq!(c0["opened_at"], "2026-09-01T09:00:00Z");
    let key = c0["customer_key"].as_str().unwrap();
    let want = format!("{:x}", Sha256::digest(format!("{SALT}|{ID}|CUS-0000").as_bytes()))[..16].to_string();
    assert_eq!(key, want);
    assert!(key.len() == 16 && key.bytes().all(|b| b.is_ascii_hexdigit()));
    // same customer, same key; another customer, another key
    let k = |case: &str| cases.iter().find(|c| c["case_id"] == case).unwrap()["customer_key"].clone();
    assert_eq!(k("CASE-00000"), k("CASE-00007"));
    assert_ne!(k("CASE-00000"), k("CASE-00001"));
    let all: String = packages(&work).iter().map(|p| std::fs::read_to_string(p.join("cases.ndjson")).unwrap()).collect();
    assert!(!all.contains("CUS-") && !all.contains("customer_id"));
    // another salt, another key (the hash is not reversible by enumerating ids without the salt)
    let dir2 = temp_path("cases2");
    let work2 = dir2.join("work");
    run_all(&db, &cfg(&work2, &[("platform_cells", "on"), ("customer_key_salt", "another-salt")]), &work2);
    let other: Vec<Value> = packages(&work2).iter().flat_map(|p| lines(&p.join("cases.ndjson"))).collect();
    assert_ne!(other.iter().find(|c| c["case_id"] == "CASE-00000").unwrap()["customer_key"], c0["customer_key"]);
}

#[test]
fn without_the_flag_the_package_is_the_old_one() {
    let dir = temp_path("off");
    let db = build_db(&dir, &rows(30));
    let work = dir.join("work");
    run_all(&db, &cfg(&work, &[]), &work);
    let pkg = packages(&work).remove(0);
    assert!(lines(&pkg.join("events.ndjson")).iter().all(|e| e.get("payload").is_none()));
    assert_eq!(keys(&lines(&pkg.join("cases.ndjson"))[0]), ["case_id", "channel", "language", "previous_case_id", "priority"]);
}

#[test]
fn a_history_with_more_cases_than_one_dimension_read_keeps_every_case_of_the_batches() {
    // HARD_CAP is 10_000 rows per dimension read: the second batch below holds cases an unordered LIMIT read would have lost
    let dir = temp_path("many");
    let n = 10_400usize;
    let db = build_db(&dir, &rows(n));
    let work = dir.join("work");
    let recs = run_all(&db, &cfg(&work, &[("platform_cells", "on"), ("customer_key_salt", SALT), ("batch_cap", "10000")]), &work);
    assert!(recs.len() >= 4, "{}", recs.len());
    let cases: std::collections::BTreeSet<String> = packages(&work).iter().flat_map(|p| lines(&p.join("cases.ndjson"))).map(|c| c["case_id"].as_str().unwrap().to_string()).collect();
    assert_eq!(cases.len(), n, "every case with an event reached a package");
}

#[test]
fn the_package_feeds_the_cells_aggregator_end_to_end() {
    let Some(py) = ["python", "python3"].iter().find(|p| std::process::Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success())) else {
        eprintln!("SKIP: no python on PATH");
        return;
    };
    let dir = temp_path("e2e");
    let db = build_db(&dir, &rows(400));
    let work = dir.join("work");
    run_all(&db, &cfg(&work, &[("platform_cells", "on"), ("customer_key_salt", SALT), ("batch_cap", "700")]), &work);
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/aggregate/platform_event_cells.py");
    let out = dir.join("cells.ndjson");
    let r = std::process::Command::new(py)
        .args([script.to_str().unwrap(), "--package-root", work.join("packages").to_str().unwrap(), "--source-id", ID, "--out", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let cells = lines(&out);
    assert!(!cells.is_empty());
    let draft: Vec<&Value> = cells.iter().filter(|c| c["metric"] == "P_DRAFT_REJECT" && c["period"] == "ALL").collect();
    assert!(!draft.is_empty(), "the draft acceptance family reaches the cells");
    let all: i64 = draft.iter().filter(|c| c["dims"].get("case_type").is_some()).map(|c| c["denominator"].as_i64().unwrap()).sum();
    assert_eq!(all, 400, "all 400 decided drafts counted once across batches and case types");
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(!text.contains("CASE-") && !text.contains("CUS-") && !text.contains("STF-"));
}
