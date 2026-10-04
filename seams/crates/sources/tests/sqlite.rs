mod common;
use common::*;
use sources::sqlite::SqliteProduct;
use sources::{DataMode, SourceAdapter, SourceError, SourceId, Watermark};

fn open(tag: &str) -> (SqliteProduct, std::path::PathBuf) {
    let dir = temp_path(tag);
    let p = fixture(&dir, "tenant-a");
    (SqliteProduct::open(&p, SourceId::new(DataMode::Platform, "platform:tenant-a").unwrap()).unwrap(), p)
}

#[test]
fn opens_read_only_and_never_creates_or_changes_the_file() {
    let dir = temp_path("missing");
    let missing = dir.join("nope.db");
    assert!(SqliteProduct::open(&missing, SourceId::new(DataMode::Platform, "platform:t").unwrap()).is_err());
    assert!(!missing.exists());
    let (a, p) = open("ro");
    let before = std::fs::read(&p).unwrap();
    a.read_events(&Watermark::Sequence(0), 100).unwrap();
    assert!(!a.can_write(), "the connection must refuse writes");
    assert_eq!(before, std::fs::read(&p).unwrap());
}

#[test]
fn lists_only_allow_listed_tables_that_exist() {
    let (a, _) = open("list");
    let mut t = a.list_tables().unwrap();
    t.sort();
    assert_eq!(t, ["cases", "event_log", "staff", "turns"]);
}

#[test]
fn batches_follow_the_watermark_without_rereading() {
    let (a, _) = open("wm");
    let b1 = a.read_events(&Watermark::Sequence(0), 3).unwrap();
    assert_eq!(b1.events.iter().map(|e| e.sequence.unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(b1.next, Watermark::Sequence(3));
    assert!(b1.more);
    let b2 = a.read_events(&b1.next, 100).unwrap();
    assert_eq!(b2.events.iter().map(|e| e.sequence.unwrap()).collect::<Vec<_>>(), [4, 5, 6, 7]);
    assert_eq!(b2.next, Watermark::Sequence(7));
    assert!(!b2.more);
    let b3 = a.read_events(&b2.next, 100).unwrap();
    assert!(b3.events.is_empty());
    assert_eq!(b3.next, Watermark::Sequence(7));
    assert_eq!(b1.events[0].event_type, "case.opened");
    assert_eq!(b1.events[0].case_id.as_deref(), Some("case-1"));
}

#[test]
fn limit_beyond_the_hard_cap_or_zero_is_refused_and_wrong_watermark_kind_too() {
    let (a, _) = open("cap");
    assert_eq!(a.read_events(&Watermark::Sequence(0), 0), Err(SourceError::BadLimit(0)));
    assert!(matches!(a.read_events(&Watermark::Sequence(0), 10_001), Err(SourceError::BadLimit(_))));
    let ds = Watermark::origin_for("dataset-pg");
    assert!(matches!(a.read_events(&ds, 5), Err(SourceError::BadWatermark(_))));
}

#[test]
fn denied_data_never_leaves_the_adapter() {
    let (a, _) = open("deny");
    let evs = a.read_events(&Watermark::Sequence(0), 100).unwrap();
    let dim = a.read_dimension("turns", 10).unwrap();
    let dump = format!("{evs:?}{dim:?}{:?}{:?}", a.read_dimension("cases", 10).unwrap(), a.read_dimension("staff", 10).unwrap());
    for s in SENTINELS {
        assert!(!dump.contains(s), "{s} leaked");
    }
    assert!(!dim[0].contains_key("text"));
    assert!(dim[0].contains_key("author_role"));
    for t in ["login_accounts", "mfa_challenges", "staff_sessions", "teams", "labels", "pseudonym_map", "event_log"] {
        assert!(a.read_dimension(t, 10).is_err(), "{t}");
    }
    assert!(a.guarded_probe("SELECT password_hash FROM login_accounts").is_err());
    assert!(a.guarded_probe("SELECT text FROM turns").is_err());
    assert!(a.guarded_probe("SELECT id, case_id FROM turns").is_ok());
    assert!(a.guarded_probe("DELETE FROM turns").is_err());
}

#[test]
fn schema_check_reports_drift_and_denied_tables_present_without_reading_them() {
    let (a, _) = open("schema");
    let r = a.schema_check().unwrap();
    assert!(r.ok(), "{r:?}");
    assert_eq!(r.denied_present, ["login_accounts"]);
    let dir = temp_path("drift");
    let q = dir.join("d.db");
    let c = rusqlite::Connection::open(&q).unwrap();
    c.execute_batch("CREATE TABLE event_log(sequence INTEGER PRIMARY KEY, event_id TEXT, event_type TEXT)").unwrap();
    drop(c);
    let b = SqliteProduct::open(&q, SourceId::new(DataMode::Platform, "platform:drift").unwrap()).unwrap();
    let r = b.schema_check().unwrap();
    assert!(!r.ok());
    assert!(r.missing.contains(&("event_log".into(), "event_time".into())));
    assert!(b.read_events(&Watermark::Sequence(0), 5).is_err(), "reads fail closed on drift");
}
