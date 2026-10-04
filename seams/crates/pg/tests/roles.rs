mod common;
use common::{migrations_dir, TempDb};
use pg::{migrate, roles};

fn secrets() -> roles::RoleSecrets {
    roles::RoleSecrets { core_app: "t-core".into(), core_eval_app: "t-eval".into(), exporter_ro: "t-ro".into() }
}

#[test]
fn from_env_requires_every_password() {
    // unset variables are an error naming the variable; values are never defaults
    let err = roles::RoleSecrets::from_lookup(|k| if k == "PULSO_PG_CORE_APP_PASSWORD" { Some("x".into()) } else { None }).unwrap_err();
    assert!(err.to_string().contains("PULSO_PG_CORE_EVAL_APP_PASSWORD"), "{err}");
    let ok = roles::RoleSecrets::from_lookup(|_| Some("p".into())).unwrap();
    assert_eq!(ok.exporter_ro, "p");
}

#[test]
fn no_password_literals_in_sources() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/roles.rs")).unwrap();
    assert!(!src.to_lowercase().contains("password '"), "password literal in roles.rs");
}

fn has(c: &mut postgres::Client, role: &str, table: &str, privilege: &str) -> bool {
    c.query_one("SELECT has_table_privilege($1, $2, $3)", &[&role, &table, &privilege]).unwrap().get(0)
}

#[test]
fn roles_are_least_privilege_and_bootstrap_is_idempotent() {
    let Some(db) = TempDb::create() else { return };
    let mut c = db.connect();
    migrate::migrate(&mut c, &migrate::discover(&migrations_dir()).unwrap()).unwrap();
    roles::bootstrap(&mut c, &secrets()).unwrap();
    roles::bootstrap(&mut c, &secrets()).unwrap();
    for r in ["core_app", "core_eval_app", "exporter_ro"] {
        let row = c
            .query_one(
                "SELECT rolcanlogin, rolsuper, rolcreatedb, rolcreaterole, rolreplication, rolbypassrls FROM pg_roles WHERE rolname = $1",
                &[&r],
            )
            .unwrap();
        let flags: Vec<bool> = (0..6).map(|i| row.get(i)).collect();
        assert_eq!(flags, [true, false, false, false, false, false], "{r}");
    }
    for t in ["pulso_jobs", "pulso_run_events"] {
        for p in ["SELECT", "INSERT", "UPDATE"] {
            assert!(has(&mut c, "core_app", t, p), "core_app {p} {t}");
        }
        for p in ["DELETE", "TRUNCATE"] {
            assert!(!has(&mut c, "core_app", t, p), "core_app must not {p} {t}");
        }
        assert!(has(&mut c, "exporter_ro", t, "SELECT"));
        for p in ["INSERT", "UPDATE", "DELETE"] {
            assert!(!has(&mut c, "exporter_ro", t, p), "exporter_ro must not {p} {t}");
        }
        assert!(!has(&mut c, "core_eval_app", t, "SELECT"));
    }
    assert!(!has(&mut c, "core_app", "pulso_schema_migrations", "INSERT"));
    let create: bool = c.query_one("SELECT has_schema_privilege('core_app','public','CREATE')", &[]).unwrap().get(0);
    assert!(!create, "core_app must not create objects");
    let n: i64 = c.query_one("SELECT count(*) FROM pg_roles WHERE rolname IN ('core_app','core_eval_app','exporter_ro') AND rolpassword IS NULL", &[]).unwrap().get(0);
    let _ = n; // rolpassword is only visible to superusers via pg_authid
    let pw: i64 = c.query_one("SELECT count(*) FROM pg_authid WHERE rolname IN ('core_app','core_eval_app','exporter_ro') AND rolpassword IS NOT NULL", &[]).unwrap().get(0);
    assert_eq!(pw, 3);
}
