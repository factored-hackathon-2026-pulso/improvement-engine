#![allow(dead_code)]
use postgres::{Client, Config, NoTls};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

pub fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../migrations")
}

/// Throw-away database on the admin server; `None` when PULSO_TEST_PG_ADMIN is unset
/// (live parity blocked) unless PULSO_REQUIRE_POSTGRES=1, which makes it a failure.
pub struct TempDb {
    admin: Config,
    pub name: String,
}

static N: AtomicU32 = AtomicU32::new(0);

impl TempDb {
    pub fn create() -> Option<TempDb> {
        let Ok(dsn) = std::env::var("PULSO_TEST_PG_ADMIN") else {
            assert!(
                std::env::var("PULSO_REQUIRE_POSTGRES").is_err(),
                "PULSO_TEST_PG_ADMIN not set"
            );
            eprintln!("SKIP: PULSO_TEST_PG_ADMIN not set (live parity blocked)");
            return None;
        };
        let admin: Config = dsn.parse().expect("admin dsn");
        let name = format!("mig0_{}_{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst));
        admin.connect(NoTls).expect("admin connect").batch_execute(&format!("CREATE DATABASE {name}")).unwrap();
        Some(TempDb { admin, name })
    }
    pub fn connect(&self) -> Client {
        let mut c = self.admin.clone();
        c.dbname(&self.name);
        c.connect(NoTls).expect("connect")
    }
    pub fn admin(&self) -> Client {
        self.admin.connect(NoTls).expect("admin connect")
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = self.admin.connect(NoTls).and_then(|mut c| {
            c.batch_execute(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name))
        });
    }
}
