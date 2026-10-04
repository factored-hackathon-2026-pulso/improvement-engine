use sources::policy::*;
use sources::SourceError;
use std::collections::BTreeMap;

fn python_policy() -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../platform-exporter/src/platform_exporter/policy.py");
    std::fs::read_to_string(p).expect("policy.py")
}

fn py_allowed() -> BTreeMap<String, Vec<String>> {
    let src = python_policy();
    let body = src.split("ALLOWED_COLUMNS: dict[str, tuple[str, ...]] = {").nth(1).unwrap().split("\n}\n").next().unwrap().to_string();
    let mut out = BTreeMap::new();
    for part in body.split(')').filter(|p| p.contains('(')) {
        let (k, cols) = part.split_once('(').unwrap();
        let table = k.split('"').nth(1).unwrap().to_string();
        out.insert(table, cols.split('"').skip(1).step_by(2).map(String::from).collect());
    }
    out
}

#[test]
fn allow_list_mirrors_the_exporter_policy() {
    let py = py_allowed();
    assert_eq!(py.len(), 7, "parsed {py:?}");
    let mut tables = allowed_tables();
    tables.sort();
    assert_eq!(tables, py.keys().map(String::as_str).collect::<Vec<_>>());
    for (t, cols) in &py {
        assert_eq!(allowed_columns(t).unwrap(), cols.as_slice(), "{t}");
    }
}

#[test]
fn denylist_mirrors_the_exporter_policy() {
    let src = python_policy();
    let line = src.lines().find(|l| l.starts_with("DENIED_TABLES")).unwrap();
    let py: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
    assert_eq!(DENIED_TABLES, py.as_slice());
}

#[test]
fn denied_and_unknown_tables_and_columns_are_refused() {
    for t in ["login_accounts", "mfa_challenges", "staff_sessions", "labels", "timeline", "pseudonym_map", "teams", "event_log; drop", "EVENT_LOG ", ""] {
        let r = assert_table_allowed(t);
        if t.trim().eq_ignore_ascii_case("event_log") {
            assert_eq!(r.unwrap(), "event_log");
        } else {
            assert!(matches!(r, Err(SourceError::AccessDenied(_))), "{t:?} -> {r:?}");
        }
    }
    assert!(assert_columns_allowed("turns", &["id", "text"]).is_err());
    assert!(assert_columns_allowed("staff", &["id", "email"]).is_err());
    assert!(assert_columns_allowed("cases", &["status"]).is_err());
    assert!(assert_columns_allowed("cases", &["id", "channel"]).is_ok());
}

#[test]
fn events_are_read_without_payload_and_inside_the_allow_list() {
    assert!(!EVENT_READ_COLUMNS.contains(&"payload"));
    assert!(assert_columns_allowed("event_log", EVENT_READ_COLUMNS).is_ok());
    assert!(EVENT_READ_COLUMNS.contains(&"sequence"));
}

#[test]
fn limit_is_hard_capped() {
    assert_eq!(check_limit(1), Ok(1));
    assert_eq!(check_limit(HARD_CAP), Ok(HARD_CAP));
    assert_eq!(check_limit(0), Err(SourceError::BadLimit(0)));
    assert_eq!(check_limit(HARD_CAP + 1), Err(SourceError::BadLimit(HARD_CAP + 1)));
}

#[test]
fn event_type_lists_mirror_the_contract_catalog() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../platform-contract/event-catalog.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let by = |status: &str| -> Vec<String> {
        let mut x: Vec<String> = v["event_types"].as_array().unwrap().iter().filter(|e| e["status"] == status).map(|e| e["event_type"].as_str().unwrap().to_owned()).collect();
        x.sort();
        x
    };
    let sorted = |l: &[&str]| {
        let mut x: Vec<String> = l.iter().map(|s| (*s).to_owned()).collect();
        x.sort();
        x
    };
    assert_eq!(by("admitted"), sorted(sources::monitor::ADMITTED_EVENT_TYPES));
    assert_eq!(by("denied"), sorted(sources::monitor::DENIED_EVENT_TYPES));
}
