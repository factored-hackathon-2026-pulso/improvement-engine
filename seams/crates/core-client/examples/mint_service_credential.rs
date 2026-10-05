//! Test support for the agent-core verifier contract (scripts/serve-credentials): mints ONE service credential from a seed in the
//! environment and prints `{"credential", "kid", "public_key", "now", "exp"}`. Never use with a real seed: the credential is printed.
//! Env: PULSO_SERVICE_SEED_HEX, PULSO_SERVICE_KID. Args: [--now UNIX] [--ttl SECONDS] [--roles a,b] [--id ID].
use core_client::service_identity::{ServiceIdentity, parse_seed_hex};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let seed = parse_seed_hex(&std::env::var("PULSO_SERVICE_SEED_HEX").unwrap_or_default()).unwrap_or_else(|e| {
        eprintln!("PULSO_SERVICE_SEED_HEX: {e}");
        std::process::exit(2)
    });
    let kid = std::env::var("PULSO_SERVICE_KID").unwrap_or_default();
    let now: i64 = arg("--now").and_then(|v| v.parse().ok()).unwrap_or_else(|| (core_client::service_identity::system_clock())());
    let mut id = ServiceIdentity::new(&kid, seed).with_clock(Arc::new(move || now));
    if let Some(t) = arg("--ttl").and_then(|v| v.parse().ok()) {
        id = id.with_ttl(t);
    }
    if let Some(r) = arg("--roles") {
        let roles: Vec<&str> = r.split(',').collect();
        id = id.with_roles(&roles);
    }
    if let Some(i) = arg("--id") {
        id = id.with_principal_id(&i);
    }
    let jws = id.credential();
    let exp = jws.claims().map(|c| c["exp"].clone()).unwrap_or_default();
    println!("{}", serde_json::json!({"credential": jws.reveal(), "kid": id.kid(), "public_key": id.public_key_b64url(), "now": now, "exp": exp}));
}
