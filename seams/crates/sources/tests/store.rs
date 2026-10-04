mod common;
use common::temp_path;
use sources::store::{FileStore, MemStore, WatermarkRecord, WatermarkStore};
use sources::{DataMode, SourceError, SourceId, Watermark};

fn rec(adapter: &str, w: Watermark) -> WatermarkRecord {
    WatermarkRecord { watermark: w, adapter: adapter.into(), first_event_time: Some("2026-10-01T10:00:00Z".into()), cases_opened: 2, batches: 1 }
}
fn pid(n: &str) -> SourceId {
    SourceId::new(DataMode::Platform, &format!("platform:{n}")).unwrap()
}

fn contract(s: &dyn WatermarkStore) {
    let a = pid("a");
    let b = pid("b");
    assert_eq!(s.get(&a).unwrap(), None);
    s.commit(&a, None, &rec("product-sqlite", Watermark::Sequence(5))).unwrap();
    assert_eq!(s.get(&a).unwrap().unwrap().watermark, Watermark::Sequence(5));
    // sources never mix: b is untouched by a
    assert_eq!(s.get(&b).unwrap(), None);
    s.commit(&b, None, &rec("product-sqlite", Watermark::Sequence(1))).unwrap();
    s.commit(&a, Some(&Watermark::Sequence(5)), &rec("product-sqlite", Watermark::Sequence(9))).unwrap();
    assert_eq!(s.get(&b).unwrap().unwrap().watermark, Watermark::Sequence(1));
    assert_eq!(s.get(&a).unwrap().unwrap().watermark, Watermark::Sequence(9));
    // compare-and-set: a stale expected watermark, or a creation over an existing row, is a conflict
    assert!(matches!(s.commit(&a, Some(&Watermark::Sequence(5)), &rec("product-sqlite", Watermark::Sequence(12))), Err(SourceError::Conflict(_))));
    assert!(matches!(s.commit(&a, None, &rec("product-sqlite", Watermark::Sequence(12))), Err(SourceError::Conflict(_))));
    // never backwards
    assert!(matches!(s.commit(&a, Some(&Watermark::Sequence(9)), &rec("product-sqlite", Watermark::Sequence(3))), Err(SourceError::BadWatermark(_))));
    // a source is bound to one adapter and one watermark kind
    assert!(matches!(s.commit(&a, Some(&Watermark::Sequence(9)), &rec("product-postgres", Watermark::Sequence(10))), Err(SourceError::Conflict(_))));
    assert!(matches!(s.commit(&a, Some(&Watermark::Sequence(9)), &rec("product-sqlite", Watermark::origin_for("dataset-pg"))), Err(SourceError::BadWatermark(_))));
    // a dataset source cannot take a product watermark
    let d = SourceId::new(DataMode::Dataset, "dataset:e0:b1").unwrap();
    assert!(matches!(s.commit(&d, None, &rec("dataset-pg", Watermark::Sequence(1))), Err(SourceError::BadWatermark(_))));
    let w = Watermark::Dataset { ingested_at: "2026-10-01 10:00:00+00".into(), batch_id: "b|1".into(), key: "e0_case:c1".into() };
    s.commit(&d, None, &rec("dataset-pg", w.clone())).unwrap();
    assert_eq!(s.get(&d).unwrap().unwrap().watermark, w);
    assert_eq!(s.get(&a).unwrap().unwrap().watermark, Watermark::Sequence(9));
}

#[test]
fn mem_store_meets_the_contract() {
    contract(&MemStore::default());
}

#[test]
fn file_store_meets_the_contract_and_persists_across_reopen() {
    let dir = temp_path("fstore");
    contract(&FileStore::open(&dir).unwrap());
    let again = FileStore::open(&dir).unwrap();
    let r = again.get(&pid("a")).unwrap().unwrap();
    assert_eq!(r.watermark, Watermark::Sequence(9));
    assert_eq!(r.adapter, "product-sqlite");
    assert_eq!(r.first_event_time.as_deref(), Some("2026-10-01T10:00:00Z"));
    assert_eq!(r.cases_opened, 2);
}

#[test]
fn watermark_text_round_trips() {
    for w in [Watermark::Sequence(0), Watermark::Sequence(77), Watermark::Dataset { ingested_at: "t|%".into(), batch_id: "b".into(), key: "".into() }] {
        assert_eq!(Watermark::decode(&w.encode()).unwrap(), w);
    }
    for bad in ["", "seq:-1", "seq:x", "ds:a|b", "zzz"] {
        assert!(Watermark::decode(bad).is_err(), "{bad}");
    }
}

#[test]
fn migration_creates_the_pulso_watermark_table_additively() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../migrations");
    let f = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.starts_with("0053_")).expect("0053 migration");
    assert_eq!(f, "0053_pulso_source_watermark.sql");
    let sql = std::fs::read_to_string(dir.join(&f)).unwrap();
    assert!(sql.contains("CREATE SCHEMA IF NOT EXISTS pulso"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS pulso.source_watermark"));
    assert!(sql.contains("source_id TEXT PRIMARY KEY"));
    let up = sql.to_uppercase();
    assert!(!up.contains("DROP ") && !up.contains("ALTER TABLE PULSO_JOBS"));
}

#[test]
fn stores_refuse_an_adapter_that_cannot_serve_the_source_mode() {
    let s = MemStore::default();
    let bad = s.commit(&pid("x"), None, &rec("dataset-pg", Watermark::Sequence(1)));
    assert!(bad.is_err());
    let ds = SourceId::new(DataMode::Dataset, "dataset:y").unwrap();
    let w = Watermark::Dataset { ingested_at: "a".into(), batch_id: "b".into(), key: "k".into() };
    assert!(s.commit(&ds, None, &rec("product-sqlite", w.clone())).is_err());
    assert!(s.commit(&pid("x"), None, &rec("nonsense", Watermark::Sequence(1))).is_err());
    assert!(s.commit(&ds, None, &rec("dataset-pg", w)).is_ok());
}
