//! `E2E_VERIFY_KEYS` (public keys only; same JSON as the Python double), `E2E_PORT` (default 8700),
//! `CONTROL_API_LABS` (JSON `{tenant: path-to-ED0L-sqlite}`, optional), `CONTROL_API_MIN_K` (default 10), `CONTROL_API_MIN_CELL` (default 0),
//! `CONTROL_API_DATABASE_URL` (Postgres; durable `PgStore`, migration 0052 applied at start; the URL is never logged) else in-memory `MemStore`,
//! `CONTROL_API_ADMIN=1` enables the `/_e2e/config` test channel. Binds 127.0.0.1 unless `CONTROL_API_HOST` is set.
use control_api::{
    app::{App, Config},
    auth::KeyRing,
    server,
    pgstore::{DatabaseUrl, PgStore},
    store::{MemStore, Store},
};
use std::sync::Arc;

fn main() {
    let raw = std::env::var("E2E_VERIFY_KEYS").expect("E2E_VERIFY_KEYS");
    let cfg: serde_json::Value = serde_json::from_str(&raw).expect("E2E_VERIFY_KEYS json");
    let ring = Arc::new(KeyRing::from_json(&cfg["ring"]).expect("ring"));
    let upload_pin = cfg["ingest"]
        .as_object()
        .and_then(|i| Some((i.get("binding_ref")?.as_str()?.to_string(), i.get("tenant_id")?.as_str()?.to_string())));
    let admin = std::env::var("CONTROL_API_ADMIN").is_ok_and(|v| v == "1");
    let host = std::env::var("CONTROL_API_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    // The admin channel is unauthenticated (seeds artifacts, wiki, bindings): never expose it beyond loopback.
    assert!(!admin || matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]"), "CONTROL_API_ADMIN=1 requires a loopback CONTROL_API_HOST");
    let store: Box<dyn Store> = match std::env::var("CONTROL_API_DATABASE_URL") {
        Ok(url) => match PgStore::connect_url(&DatabaseUrl::new(url)) {
            Ok(s) => {
                eprintln!("control-api: store = postgres (durable)");
                Box::new(s)
            }
            Err(e) => {
                eprintln!("control-api: cannot open the Postgres store: {e}"); // never contains the URL
                std::process::exit(2);
            }
        },
        Err(_) => {
            eprintln!("control-api: store = memory (state is lost on restart)");
            Box::new(MemStore::default())
        }
    };
    let app = Arc::new(App::new({
        let mut cfg = Config::new(ring);
        cfg.upload_pin = upload_pin;
        cfg.admin = admin;
        if let Ok(raw) = std::env::var("CONTROL_API_LABS") {
            let m: std::collections::HashMap<String, std::path::PathBuf> = serde_json::from_str(&raw).expect("CONTROL_API_LABS json");
            cfg.labs = m;
        }
        if let Some(k) = std::env::var("CONTROL_API_MIN_CELL").ok().and_then(|v| v.parse().ok()) {
            cfg.min_cell = k;
        }
        if let Some(k) = std::env::var("CONTROL_API_MIN_K").ok().and_then(|v| v.parse().ok()) {
            cfg.min_k = k;
        }
        cfg
    }, store));
    let port = std::env::var("E2E_PORT").unwrap_or_else(|_| "8700".into());
    let server = tiny_http::Server::http(format!("{host}:{port}")).expect("bind");
    server::serve(server, app);
}
