//! `pulso run` configuration matrix: defaults, every refusal with its named reason, and no secret in any rendering.
use pulso::config::{ConfigError, DataMode, RunConfig, Storage};
use std::collections::HashMap;
use std::time::Duration;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const DSN: &str = "postgres://svc:hunter2-secret@db.internal:5432/pulso";

fn load(pairs: &[(&str, &str)]) -> Result<RunConfig, ConfigError> {
    let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    RunConfig::from_lookup(&|k| m.get(k).cloned())
}

fn err(pairs: &[(&str, &str)]) -> String {
    load(pairs).expect_err("must refuse").to_string()
}

#[test]
fn minimal_postgres_config_has_safe_defaults() {
    let c = load(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset")]).unwrap();
    assert_eq!(c.storage, Storage::Postgres);
    assert_eq!(c.data_mode, DataMode::Dataset);
    assert_eq!(c.adapter, "stub");
    assert_eq!(c.listen_addr.to_string(), "127.0.0.1:8080");
    assert_eq!(c.poll_interval, Duration::from_secs(30));
    assert_eq!(c.batch_cap, 100);
    assert_eq!(c.grace, Duration::from_secs(25), "below the ECS default stopTimeout of 30 s");
    assert!(!c.exit_on_stdin_eof);
    assert!(c.debug_token.is_none());
}

#[test]
fn explicit_memory_mode_needs_no_database() {
    let c = load(&[("PULSO_STORAGE", "memory"), ("PULSO_DATA_MODE", "platform")]).unwrap();
    assert_eq!(c.storage, Storage::Memory);
    assert!(c.database_url.is_none());
    assert_eq!(c.data_mode, DataMode::Platform);
}

#[test]
fn data_mode_is_required_and_closed() {
    assert!(err(&[("PULSO_DATABASE_URL", DSN)]).contains("PULSO_DATA_MODE"));
    let e = err(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "both")]);
    assert!(e.contains("config_invalid") && e.contains("PULSO_DATA_MODE"), "{e}");
}

#[test]
fn storage_must_be_unambiguous() {
    assert!(err(&[("PULSO_DATA_MODE", "dataset")]).contains("PULSO_DATABASE_URL"), "no url and no explicit memory");
    let e = err(&[("PULSO_DATA_MODE", "dataset"), ("PULSO_STORAGE", "memory"), ("PULSO_DATABASE_URL", DSN)]);
    assert!(e.contains("config_conflict"), "{e}");
    let e = err(&[("PULSO_DATA_MODE", "dataset"), ("PULSO_STORAGE", "postgres")]);
    assert!(e.contains("PULSO_DATABASE_URL"), "{e}");
    let e = err(&[("PULSO_DATA_MODE", "dataset"), ("PULSO_STORAGE", "sqlite")]);
    assert!(e.contains("PULSO_STORAGE"), "{e}");
}

#[test]
fn database_url_must_be_postgres_and_is_never_echoed() {
    let e = err(&[("PULSO_DATA_MODE", "dataset"), ("PULSO_DATABASE_URL", "mysql://u:topsecret@h/db")]);
    assert!(e.contains("PULSO_DATABASE_URL"), "{e}");
    assert!(!e.contains("topsecret") && !e.contains("mysql://"), "{e}");
}

#[test]
fn adapter_must_match_the_data_mode() {
    let e = err(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset"), ("PULSO_SOURCE_ADAPTER", "product-postgres")]);
    assert!(e.contains("config_conflict") && e.contains("dataset") && e.contains("product-postgres"), "{e}");
    let e = err(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_ADAPTER", "e0-raw")]);
    assert!(e.contains("config_conflict"), "{e}");
    assert_eq!(load(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_ADAPTER", "product-sqlite")]).unwrap().adapter, "product-sqlite");
    assert_eq!(load(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset"), ("PULSO_SOURCE_ADAPTER", "e0-augmented")]).unwrap().adapter, "e0-augmented");
    assert!(err(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset"), ("PULSO_SOURCE_ADAPTER", "nope")]).contains("PULSO_SOURCE_ADAPTER"));
}

#[test]
fn non_loopback_needs_explicit_opt_in_and_a_strong_token() {
    let base = [("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset"), ("PULSO_LISTEN_ADDR", "0.0.0.0:8080")];
    let e = err(&base);
    assert!(e.contains("PULSO_ALLOW_NON_LOOPBACK"), "{e}");
    let mut with_opt = base.to_vec();
    with_opt.push(("PULSO_ALLOW_NON_LOOPBACK", "1"));
    let e = err(&with_opt);
    assert!(e.contains("PULSO_DEBUG_TOKEN"), "{e}");
    let mut weak = with_opt.clone();
    weak.push(("PULSO_DEBUG_TOKEN", "short"));
    assert!(err(&weak).contains("PULSO_DEBUG_TOKEN"));
    let mut ok = with_opt.clone();
    ok.push(("PULSO_DEBUG_TOKEN", TOKEN));
    let c = load(&ok).unwrap();
    assert_eq!(c.listen_addr.to_string(), "0.0.0.0:8080");
    // an admin token on a non-loopback bind must be strong and distinct from the debug token
    ok.push(("PULSO_ADMIN_TOKEN", TOKEN));
    assert!(err(&ok).contains("PULSO_ADMIN_TOKEN"));
}

#[test]
fn loopback_may_run_without_a_token() {
    let c = load(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset"), ("PULSO_LISTEN_ADDR", "[::1]:9000")]).unwrap();
    assert!(c.listen_addr.ip().is_loopback());
    assert!(err(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset"), ("PULSO_LISTEN_ADDR", "not-an-addr")]).contains("PULSO_LISTEN_ADDR"));
}

#[test]
fn numeric_bounds_and_prefix_rules() {
    let b = [("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset")];
    let with = |k: &'static str, v: &'static str| {
        let mut p = b.to_vec();
        p.push((k, v));
        p
    };
    assert_eq!(load(&with("PULSO_POLL_INTERVAL_MS", "250")).unwrap().poll_interval, Duration::from_millis(250));
    assert!(err(&with("PULSO_POLL_INTERVAL_MS", "0")).contains("PULSO_POLL_INTERVAL_MS"));
    assert!(err(&with("PULSO_POLL_INTERVAL_MS", "abc")).contains("PULSO_POLL_INTERVAL_MS"));
    assert_eq!(load(&with("PULSO_BATCH_CAP", "5")).unwrap().batch_cap, 5);
    assert!(err(&with("PULSO_BATCH_CAP", "0")).contains("PULSO_BATCH_CAP"));
    assert!(err(&with("PULSO_BATCH_CAP", "100001")).contains("PULSO_BATCH_CAP"));
    assert_eq!(load(&with("PULSO_SHUTDOWN_GRACE_SECS", "5")).unwrap().grace, Duration::from_secs(5));
    assert!(err(&with("PULSO_SHUTDOWN_GRACE_SECS", "0")).contains("PULSO_SHUTDOWN_GRACE_SECS"));
    assert_eq!(load(&with("PULSO_STORAGE_PREFIX", "pulso/prod")).unwrap().storage_prefix.as_deref(), Some("pulso/prod"));
    assert!(err(&with("PULSO_STORAGE_PREFIX", "../escape")).contains("PULSO_STORAGE_PREFIX"));
    assert!(err(&with("PULSO_STORAGE_PREFIX", "/abs")).contains("PULSO_STORAGE_PREFIX"));
    assert!(load(&with("PULSO_EXIT_ON_STDIN_EOF", "1")).unwrap().exit_on_stdin_eof);
}

#[test]
fn no_rendering_of_the_config_leaks_a_secret() {
    let c = load(&[("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset"), ("PULSO_DEBUG_TOKEN", TOKEN), ("PULSO_ADMIN_TOKEN", "fedcba9876543210fedcba9876543210")]).unwrap();
    let dbg = format!("{c:?}");
    for secret in ["hunter2", "db.internal", TOKEN, "fedcba98"] {
        assert!(!dbg.contains(secret), "{secret} leaked in {dbg}");
    }
    assert_eq!(c.database_url.as_ref().unwrap().expose(), DSN);
}

#[test]
fn base_path_is_normalised_and_validated() {
    let b = [("PULSO_DATABASE_URL", DSN), ("PULSO_DATA_MODE", "dataset")];
    let with = |v: &'static str| {
        let mut p = b.to_vec();
        p.push(("PULSO_BASE_PATH", v));
        p
    };
    assert_eq!(load(&b).unwrap().base_path, "");
    assert_eq!(load(&with("/pulso")).unwrap().base_path, "/pulso");
    assert_eq!(load(&with("/pulso/")).unwrap().base_path, "/pulso");
    assert_eq!(load(&with("/")).unwrap().base_path, "");
    assert_eq!(load(&with("/a/b-c_d")).unwrap().base_path, "/a/b-c_d");
    for bad in ["pulso", "/pul so", "/../x", "/a//b", "/a?b", "/%2e"] {
        assert!(err(&with(bad)).contains("PULSO_BASE_PATH"), "{bad}");
    }
}

// ---- adversarial review (CL): bind/token combinations that must never yield an unauthenticated public listener ----

#[test]
fn every_exposed_bind_shape_demands_opt_in_and_a_token() {
    let base = |addr: &'static str| vec![("PULSO_STORAGE", "memory"), ("PULSO_DATA_MODE", "dataset"), ("PULSO_LISTEN_ADDR", addr)];
    // exposed shapes: IPv4/IPv6 wildcards, v4-mapped (even of loopback: not treated as loopback), private/public addresses
    for addr in ["0.0.0.0:8080", "[::]:8080", "[::ffff:127.0.0.1]:8080", "[::ffff:0.0.0.0]:8080", "10.1.2.3:80", "192.168.0.5:80", "8.8.8.8:80"] {
        assert!(err(&base(addr)).contains("PULSO_ALLOW_NON_LOOPBACK"), "{addr}: needs opt-in");
        let mut b = base(addr);
        b.push(("PULSO_ALLOW_NON_LOOPBACK", "1"));
        assert!(err(&b).contains("PULSO_DEBUG_TOKEN"), "{addr}: needs token");
        // 15 bytes, whitespace-only and whitespace-padded short tokens never count
        for t in ["123456789012345", "                ", "  short   "] {
            let mut w = b.clone();
            w.push(("PULSO_DEBUG_TOKEN", t));
            assert!(err(&w).contains("PULSO_DEBUG_TOKEN"), "{addr}: weak token {t:?}");
        }
        b.push(("PULSO_DEBUG_TOKEN", TOKEN));
        assert!(load(&b).is_ok(), "{addr}: opt-in plus strong token is accepted");
    }
    // hostnames are not resolved: they are refused rather than silently bound somewhere unexpected
    for addr in ["localhost:8080", "example.com:80", ":8080", "8080", "0.0.0.0", "[::1]"] {
        assert!(err(&base(addr)).contains("PULSO_LISTEN_ADDR"), "{addr}");
    }
    // all of 127.0.0.0/8 and ::1 are loopback
    for addr in ["127.0.0.1:1", "127.9.9.9:1", "[::1]:1"] {
        assert!(load(&base(addr)).is_ok(), "{addr}");
    }
    // the opt-in flag spelled loosely does not count
    for flag in ["0", "false", "no", "TRUE", "on", "2"] {
        let mut b = base("0.0.0.0:8080");
        b.push(("PULSO_ALLOW_NON_LOOPBACK", flag));
        b.push(("PULSO_DEBUG_TOKEN", TOKEN));
        assert!(err(&b).contains("PULSO_ALLOW_NON_LOOPBACK"), "flag {flag:?}");
    }
}
