//! W7: the source / model / core selection of `pulso run` (adapter by config, honest defaults, closed vocabularies).
use pulso::config::{ConfigError, DataMode, ModelPortKind, Provenance, RunConfig};
use std::collections::HashMap;
use std::path::PathBuf;

fn load(pairs: &[(&str, &str)]) -> Result<RunConfig, ConfigError> {
    let mut m: HashMap<String, String> = [("PULSO_STORAGE", "memory")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    m.extend(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    RunConfig::from_lookup(&|k| m.get(k).cloned())
}

fn err(pairs: &[(&str, &str)]) -> String {
    load(pairs).expect_err("must refuse").to_string()
}

#[test]
fn the_defaults_are_the_stub_the_scripted_models_and_the_offline_core() {
    let c = load(&[("PULSO_DATA_MODE", "platform")]).unwrap();
    assert_eq!(c.adapter, "stub");
    assert_eq!(c.source_id, "platform:local");
    assert_eq!(load(&[("PULSO_DATA_MODE", "dataset")]).unwrap().source_id, "dataset:local");
    assert_eq!((c.work_dir.as_ref(), c.source_sqlite.as_ref(), c.read_batch), (None, None, 1000));
    assert_eq!((c.model_port, c.core_live, c.provenance), (ModelPortKind::Scripted, false, Provenance::Unspecified));
    assert!(c.roleplay_queue.is_none());
}

#[test]
fn product_sqlite_needs_its_file_and_a_work_dir() {
    let base = [("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_ADAPTER", "product-sqlite")];
    let refusal = |extra: &[(&str, &str)]| {
        let mut p = base.to_vec();
        p.extend_from_slice(extra);
        load(&p).unwrap().check_source().expect_err("must refuse").to_string()
    };
    assert!(refusal(&[]).contains("PULSO_WORK_DIR"), "{}", refusal(&[]));
    assert!(refusal(&[("PULSO_WORK_DIR", "w")]).contains("PULSO_SOURCE_SQLITE"));
    assert!(load(&[("PULSO_DATA_MODE", "platform")]).unwrap().check_source().is_ok(), "the stub needs nothing");
    let c = load(&[base[0], base[1], ("PULSO_WORK_DIR", "w"), ("PULSO_SOURCE_SQLITE", "p.db"), ("PULSO_SOURCE_ID", "platform:sim")]).unwrap();
    c.check_source().unwrap();
    assert_eq!((c.work_dir, c.source_sqlite, c.source_id.as_str()), (Some(PathBuf::from("w")), Some(PathBuf::from("p.db")), "platform:sim"));
}

#[test]
fn dataset_adapters_are_dataset_only_and_dataset_pg_is_the_named_one() {
    for a in ["dataset-pg", "dataset-raw", "dataset-augmented"] {
        let c = load(&[("PULSO_DATA_MODE", "dataset"), ("PULSO_SOURCE_ADAPTER", a), ("PULSO_WORK_DIR", "w")]).unwrap();
        assert_eq!((c.data_mode, c.adapter.as_str()), (DataMode::Dataset, a));
        let e = err(&[("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_ADAPTER", a), ("PULSO_WORK_DIR", "w")]);
        assert!(e.contains("config_conflict"), "{a}: {e}");
    }
}

#[test]
fn the_source_id_prefix_must_match_the_data_mode() {
    let e = err(&[("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_ID", "dataset:e0")]);
    assert!(e.contains("config_invalid") && e.contains("PULSO_SOURCE_ID"), "{e}");
    assert!(load(&[("PULSO_DATA_MODE", "dataset"), ("PULSO_SOURCE_ID", "dataset:e0-raw")]).is_ok());
}

#[test]
fn the_read_batch_is_bounded() {
    for bad in ["0", "10001", "x"] {
        assert!(err(&[("PULSO_DATA_MODE", "platform"), ("PULSO_READ_BATCH", bad)]).contains("PULSO_READ_BATCH"), "{bad}");
    }
    assert_eq!(load(&[("PULSO_DATA_MODE", "platform"), ("PULSO_READ_BATCH", "10000")]).unwrap().read_batch, 10_000);
}

#[test]
fn the_model_port_is_a_closed_set_and_roleplay_needs_its_queue() {
    for (v, k) in [("scripted", ModelPortKind::Scripted), ("gateway", ModelPortKind::Gateway)] {
        assert_eq!(load(&[("PULSO_DATA_MODE", "platform"), ("PULSO_MODEL_PORT", v)]).unwrap().model_port, k);
    }
    assert!(err(&[("PULSO_DATA_MODE", "platform"), ("PULSO_MODEL_PORT", "roleplay")]).contains("PULSO_ROLEPLAY_QUEUE"));
    let c = load(&[("PULSO_DATA_MODE", "platform"), ("PULSO_MODEL_PORT", "roleplay"), ("PULSO_ROLEPLAY_QUEUE", "q")]).unwrap();
    assert_eq!((c.model_port, c.roleplay_queue), (ModelPortKind::Roleplay, Some(PathBuf::from("q"))));
    assert!(err(&[("PULSO_DATA_MODE", "platform"), ("PULSO_MODEL_PORT", "gpt")]).contains("PULSO_MODEL_PORT"));
}

#[test]
fn the_core_is_the_offline_double_unless_live_is_asked_for_by_name() {
    assert!(!load(&[("PULSO_DATA_MODE", "platform"), ("PULSO_CORE_PORT", "offline")]).unwrap().core_live);
    assert!(load(&[("PULSO_DATA_MODE", "platform"), ("PULSO_CORE_PORT", "live")]).unwrap().core_live);
    assert!(err(&[("PULSO_DATA_MODE", "platform"), ("PULSO_CORE_PORT", "yes")]).contains("PULSO_CORE_PORT"));
}

#[test]
fn the_source_provenance_label_is_a_closed_set() {
    for (v, p) in [("simulated", Provenance::Simulated), ("real", Provenance::Real)] {
        assert_eq!(load(&[("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_PROVENANCE", v)]).unwrap().provenance, p);
    }
    assert!(err(&[("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_PROVENANCE", "genuine")]).contains("PULSO_SOURCE_PROVENANCE"));
}
