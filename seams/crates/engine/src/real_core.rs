//! Config-driven selection of the Core port. `PULSO_CORE_PORT` = `double` (default: the caller keeps its offline double) |
//! `live` (the `LiveCore` over core-client against the local Core stack). Constructing the live port reaches nothing; the first
//! call (or `LiveCore::readiness`) does. Secrets (service and human seeds) are read from env, never echoed in an error.
//!
//! Env for `live`: `PULSO_BRIDGE_ADDR`, `PULSO_SERVICE_KID`, `PULSO_SERVICE_SEED_HEX`, `PULSO_CORE_ADDR` (registry),
//! `PULSO_HUMAN_KID`, `PULSO_HUMAN_SEED_HEX` (the SIMULATED human issuer), `PULSO_LIVE_TENANT`, `PULSO_LIVE_AGENT` (default
//! `atencion-tarea`), `PULSO_LIVE_WRITER_RELEASE`, `PULSO_E2E_FX_ADDR` (the e2e fixtures double that seals artifacts and issues
//! bindings: a stand-in, BRG1 gap), `PULSO_LIVE_WORLD` (path of the live world fixture JSON), optional `PULSO_LIVE_BUDGET`.
use crate::live::CorePort;
use crate::live_core::{E2eFixtures, LiveCore, LiveCoreConfig, LiveWorld};
use core_client::authorizer::LocalSimAuthorizer;
use core_client::registry::RegistryClient;
use core_client::{ClientConfig, CoreClient};
use std::rc::Rc;
use std::time::Duration;

fn seed(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut s = [0u8; 32];
    for (i, b) in s.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(s)
}

/// `Ok(None)` = keep the offline double. `Ok(Some(port))` = the live port. An unknown selection, a missing variable or a
/// malformed value is an error naming the variable (never its value).
pub fn core_from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<Option<Rc<dyn CorePort>>, String> {
    match get("PULSO_CORE_PORT").as_deref() {
        None | Some("") | Some("double") => return Ok(None),
        Some("live") => {}
        Some(o) => return Err(format!("PULSO_CORE_PORT {o:?} is not double|live")),
    }
    const NEED: [&str; 10] = [
        "PULSO_BRIDGE_ADDR",
        "PULSO_SERVICE_KID",
        "PULSO_SERVICE_SEED_HEX",
        "PULSO_CORE_ADDR",
        "PULSO_HUMAN_KID",
        "PULSO_HUMAN_SEED_HEX",
        "PULSO_LIVE_TENANT",
        "PULSO_LIVE_WRITER_RELEASE",
        "PULSO_E2E_FX_ADDR",
        "PULSO_LIVE_WORLD",
    ];
    let missing: Vec<&str> = NEED.iter().copied().filter(|k| get(k).is_none_or(|v| v.trim().is_empty())).collect();
    if !missing.is_empty() {
        return Err(format!("PULSO_CORE_PORT=live needs {}", missing.join(", ")));
    }
    let v = |k: &str| get(k).unwrap_or_default();
    let (svc, human) = (seed(&v("PULSO_SERVICE_SEED_HEX")), seed(&v("PULSO_HUMAN_SEED_HEX")));
    let bad: Vec<&str> = [("PULSO_SERVICE_SEED_HEX", svc.is_none()), ("PULSO_HUMAN_SEED_HEX", human.is_none())].iter().filter(|(_, b)| *b).map(|(k, _)| *k).collect();
    if !bad.is_empty() {
        return Err(format!("{} must be 64 hex characters (value not shown)", bad.join(", ")));
    }
    let (svc, human) = (svc.unwrap_or_default(), human.unwrap_or_default());
    let world_path = v("PULSO_LIVE_WORLD");
    let world = LiveWorld::from_fixture(&std::fs::read_to_string(&world_path).map_err(|e| format!("PULSO_LIVE_WORLD {world_path}: {e}"))?).map_err(|e| format!("PULSO_LIVE_WORLD {world_path}: {e}"))?;
    let tenant = v("PULSO_LIVE_TENANT");
    let mut cc = ClientConfig::new(&v("PULSO_BRIDGE_ADDR"), &v("PULSO_SERVICE_KID"), svc, "pulso-engine");
    cc.timeout = Duration::from_secs(120);
    let agent = get("PULSO_LIVE_AGENT").filter(|a| !a.is_empty()).unwrap_or_else(|| "atencion-tarea".into());
    let budget = get("PULSO_LIVE_BUDGET").filter(|a| !a.is_empty()).unwrap_or_else(|| "bud-pulso".into());
    Ok(Some(Rc::new(LiveCore::new(
        CoreClient::new(cc),
        RegistryClient::new(&v("PULSO_CORE_ADDR"), Duration::from_secs(60)),
        LocalSimAuthorizer::new(&v("PULSO_HUMAN_KID"), human, &tenant, "local-supervisor"),
        Box::new(E2eFixtures { addr: v("PULSO_E2E_FX_ADDR"), tenant: tenant.clone() }),
        LiveCoreConfig { tenant, agent_id: agent, writer_release_id: v("PULSO_LIVE_WRITER_RELEASE"), budget_ref: budget, world },
    ))))
}
