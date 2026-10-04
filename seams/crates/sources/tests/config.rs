use sources::config::{AdapterKind, Config};
use sources::{DataMode, SourceError};
use std::time::Duration;

const OK: &[(&str, &str)] = &[("data_mode", "platform"), ("adapter", "product-sqlite"), ("source_id", "platform:tenant-a"), ("work_dir", "w"), ("runner_exe", "r")];

fn with(extra: &[(&'static str, &'static str)]) -> Result<Config, SourceError> {
    let mut v: Vec<(&str, &str)> = OK.to_vec();
    v.retain(|(k, _)| !extra.iter().any(|(e, _)| e == k));
    v.extend_from_slice(extra);
    Config::from_pairs(&v)
}

#[test]
fn valid_config_gets_documented_defaults() {
    let c = with(&[]).unwrap();
    assert_eq!((c.data_mode, c.adapter), (DataMode::Platform, AdapterKind::ProductSqlite));
    assert_eq!(c.source_id.as_str(), "platform:tenant-a");
    assert_eq!((c.poll_interval, c.batch_cap, c.min_history_days, c.min_history_cases, c.min_support), (Duration::from_secs(30), 1000, 14, 200, 5));
    let d = with(&[("data_mode", "dataset"), ("adapter", "dataset-pg"), ("source_id", "dataset:e0:b1"), ("poll_interval_secs", "5"), ("batch_cap", "250")]).unwrap();
    assert_eq!((d.data_mode, d.adapter, d.batch_cap, d.poll_interval), (DataMode::Dataset, AdapterKind::DatasetPg, 250, Duration::from_secs(5)));
}

#[test]
fn unknown_invalid_or_inconsistent_config_refuses() {
    let refused: &[&[(&'static str, &'static str)]] = &[
        &[("frobnicate", "1")],
        &[("data_mode", "prod")],
        &[("adapter", "product-mysql")],
        &[("adapter", "dataset-pg")],                                   // dataset adapter on platform mode
        &[("data_mode", "dataset"), ("source_id", "dataset:e0:b1")],    // product adapter on dataset mode
        &[("source_id", "dataset:e0:b1")],                              // source id of the other mode
        &[("source_id", "platform:UPPER")],
        &[("source_id", "")],
        &[("poll_interval_secs", "0")],
        &[("poll_interval_secs", "86401")],
        &[("poll_interval_secs", "soon")],
        &[("batch_cap", "0")],
        &[("batch_cap", "10001")],
        &[("min_support", "0")],
    ];
    for r in refused {
        assert!(with(r).is_err(), "{r:?} must refuse");
    }
    for missing in ["data_mode", "adapter", "source_id", "work_dir", "runner_exe"] {
        let v: Vec<(&str, &str)> = OK.iter().copied().filter(|(k, _)| *k != missing).collect();
        assert!(Config::from_pairs(&v).is_err(), "{missing}");
    }
    let mut dup = OK.to_vec();
    dup.push(("batch_cap", "1"));
    dup.push(("batch_cap", "2"));
    assert!(Config::from_pairs(&dup).is_err());
}
