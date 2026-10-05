//! How the loop authenticates to the agent-core registry (`/v1/registry/*`): a static token (the old path) or credentials MINTED from the
//! engine's service seed (agent-core `serve`).
//!
//! - `static`: `PULSO_REGISTRY_TOKEN` is a ready-made builder principal. The local stack and the integrated rig use it. Unchanged.
//! - `mint`: `PULSO_SERVICE_SEED_HEX` + `PULSO_SERVICE_KID` (what infra renders into `pulso.env`) sign a short-lived `builder` principal for
//!   every request (`core_client::service_identity`); it is refreshed before expiry and renewed once on a 401. `serve` expires builder
//!   credentials in minutes, so a long job (the regression proof takes minutes per finding) never holds one credential for its whole life.
//!
//! Selection: `PULSO_REGISTRY_AUTH=static|mint`; unset means `static` when `PULSO_REGISTRY_TOKEN` is set, else `mint` when the seed is set,
//! else a refusal naming the variables. Errors name variables, never values.
use crate::config::Secret;
use core_client::authorizer::Jws;
use core_client::service_identity::{self, ServiceIdentity};
use registry_writer::{MintingTransport, Transport};
use std::sync::Arc;

pub type Lookup<'a> = dyn Fn(&str) -> Option<String> + 'a;

pub enum RegistryAuth {
    Static(Secret),
    Minted(Arc<ServiceIdentity>),
}

impl std::fmt::Debug for RegistryAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryAuth::Static(_) => f.write_str("RegistryAuth::Static(<redacted>)"),
            RegistryAuth::Minted(i) => write!(f, "RegistryAuth::Minted({i:?})"),
        }
    }
}

fn set(get: &Lookup<'_>, k: &str) -> Option<String> {
    get(k).filter(|v| !v.trim().is_empty())
}

impl RegistryAuth {
    pub fn from_lookup(get: &Lookup<'_>) -> Result<RegistryAuth, String> {
        let token = set(get, "PULSO_REGISTRY_TOKEN");
        let seed = set(get, "PULSO_SERVICE_SEED_HEX");
        let mode = match set(get, "PULSO_REGISTRY_AUTH").as_deref() {
            Some("static") => "static",
            Some("mint") => "mint",
            Some(_) => return Err("PULSO_REGISTRY_AUTH is not static|mint".into()),
            None if token.is_some() => "static",
            None if seed.is_some() => "mint",
            None => return Err("PULSO_REGISTRY_TOKEN (a builder principal) or PULSO_SERVICE_SEED_HEX + PULSO_SERVICE_KID (minted credentials) is required with PULSO_CELLS_NDJSON".into()),
        };
        if mode == "static" {
            let t = token.ok_or("PULSO_REGISTRY_AUTH=static needs PULSO_REGISTRY_TOKEN")?;
            return Ok(RegistryAuth::Static(Secret::new(t)));
        }
        let hex = seed.ok_or("PULSO_REGISTRY_AUTH=mint needs PULSO_SERVICE_SEED_HEX")?;
        let kid = set(get, "PULSO_SERVICE_KID").ok_or("PULSO_REGISTRY_AUTH=mint needs PULSO_SERVICE_KID (the key id in the Core's staff-keys file)")?;
        let seed = service_identity::parse_seed_hex(hex.trim()).map_err(|e| format!("PULSO_SERVICE_SEED_HEX is invalid: {e}"))?;
        let mut id = ServiceIdentity::new(kid.trim(), seed);
        if let Some(p) = set(get, "PULSO_SERVICE_PRINCIPAL_ID") {
            id = id.with_principal_id(p.trim());
        }
        if let Some(t) = set(get, "PULSO_SERVICE_CRED_TTL_S") {
            let n: i64 = t.trim().parse().map_err(|_| "PULSO_SERVICE_CRED_TTL_S is not a whole number of seconds")?;
            if !(service_identity::MIN_TTL_S..=service_identity::MAX_TTL_S).contains(&n) {
                return Err(format!("PULSO_SERVICE_CRED_TTL_S must be {}..={} seconds (credentials live minutes, never hours)", service_identity::MIN_TTL_S, service_identity::MAX_TTL_S));
            }
            id = id.with_ttl(n);
        }
        Ok(RegistryAuth::Minted(Arc::new(id)))
    }

    pub fn is_minted(&self) -> bool {
        matches!(self, RegistryAuth::Minted(_))
    }

    /// The `Config::registry_token`: the real token, or a placeholder that `MintingTransport` replaces on every request.
    pub fn config_token(&self) -> Jws {
        match self {
            RegistryAuth::Static(t) => Jws::new(t.expose().to_string()),
            RegistryAuth::Minted(_) => Jws::new(String::new()),
        }
    }

    /// The transport every registry request goes through.
    pub fn wrap(&self, inner: Arc<dyn Transport + Send + Sync>) -> Arc<dyn Transport + Send + Sync> {
        match self {
            RegistryAuth::Static(_) => inner,
            RegistryAuth::Minted(id) => Arc::new(MintingTransport::new(inner, id.clone())),
        }
    }

    /// Label that travels in every delivery record.
    pub fn label(&self) -> &'static str {
        match self {
            RegistryAuth::Static(_) => "engine builder principal",
            RegistryAuth::Minted(_) => "engine builder principal (minted from the service seed, short-lived)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SEED: &str = "0707070707070707070707070707070707070707070707070707070707070707";

    fn lk(v: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let m: HashMap<String, String> = v.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn a_token_alone_keeps_the_old_static_path() {
        let a = RegistryAuth::from_lookup(&lk(&[("PULSO_REGISTRY_TOKEN", "a.b.c")])).unwrap();
        assert!(!a.is_minted());
        assert_eq!(a.config_token().reveal(), "a.b.c");
        assert_eq!(a.label(), "engine builder principal");
    }

    #[test]
    fn the_seed_and_kid_without_a_token_mint() {
        let a = RegistryAuth::from_lookup(&lk(&[("PULSO_SERVICE_SEED_HEX", SEED), ("PULSO_SERVICE_KID", "pulso-engine-1")])).unwrap();
        assert!(a.is_minted());
        assert_eq!(a.config_token().reveal(), "", "the config token is a placeholder, never a credential");
    }

    #[test]
    fn a_token_wins_over_the_seed_unless_mint_is_asked_for() {
        let both = [("PULSO_REGISTRY_TOKEN", "a.b.c"), ("PULSO_SERVICE_SEED_HEX", SEED), ("PULSO_SERVICE_KID", "k")];
        assert!(!RegistryAuth::from_lookup(&lk(&both)).unwrap().is_minted());
        let mut forced = both.to_vec();
        forced.push(("PULSO_REGISTRY_AUTH", "mint"));
        assert!(RegistryAuth::from_lookup(&lk(&forced)).unwrap().is_minted());
    }

    #[test]
    fn the_refusals_name_variables_and_never_values() {
        let e = RegistryAuth::from_lookup(&lk(&[])).unwrap_err();
        assert!(e.contains("PULSO_REGISTRY_TOKEN") && e.contains("PULSO_SERVICE_SEED_HEX"), "{e}");
        let e = RegistryAuth::from_lookup(&lk(&[("PULSO_SERVICE_SEED_HEX", SEED)])).unwrap_err();
        assert!(e.contains("PULSO_SERVICE_KID") && !e.contains(SEED), "{e}");
        let e = RegistryAuth::from_lookup(&lk(&[("PULSO_SERVICE_SEED_HEX", "zz-not-hex-secret-material"), ("PULSO_SERVICE_KID", "k")])).unwrap_err();
        assert!(e.contains("PULSO_SERVICE_SEED_HEX") && !e.contains("secret-material"), "{e}");
        let e = RegistryAuth::from_lookup(&lk(&[("PULSO_REGISTRY_AUTH", "mint"), ("PULSO_REGISTRY_TOKEN", "sekret")])).unwrap_err();
        assert!(e.contains("PULSO_SERVICE_SEED_HEX") && !e.contains("sekret"), "{e}");
        let e = RegistryAuth::from_lookup(&lk(&[("PULSO_REGISTRY_AUTH", "static"), ("PULSO_SERVICE_SEED_HEX", SEED), ("PULSO_SERVICE_KID", "k")])).unwrap_err();
        assert!(e.contains("PULSO_REGISTRY_TOKEN"), "{e}");
        assert!(RegistryAuth::from_lookup(&lk(&[("PULSO_REGISTRY_AUTH", "both")])).unwrap_err().contains("static|mint"));
    }

    #[test]
    fn the_credential_lifetime_is_bounded_to_minutes() {
        let base = [("PULSO_SERVICE_SEED_HEX", SEED), ("PULSO_SERVICE_KID", "k")];
        let with = |t: &'static str| {
            let mut v = base.to_vec();
            v.push(("PULSO_SERVICE_CRED_TTL_S", t));
            RegistryAuth::from_lookup(&lk(&v))
        };
        assert!(with("120").is_ok());
        for bad in ["10", "43200", "abc", "-5"] {
            assert!(with(bad).unwrap_err().contains("PULSO_SERVICE_CRED_TTL_S"), "{bad}");
        }
    }

    #[test]
    fn debug_shows_no_secret() {
        let a = RegistryAuth::from_lookup(&lk(&[("PULSO_REGISTRY_TOKEN", "sekret-token")])).unwrap();
        assert!(!format!("{a:?}").contains("sekret"));
        let m = RegistryAuth::from_lookup(&lk(&[("PULSO_SERVICE_SEED_HEX", SEED), ("PULSO_SERVICE_KID", "k")])).unwrap();
        assert!(!format!("{m:?}").contains("0707"));
    }
}
