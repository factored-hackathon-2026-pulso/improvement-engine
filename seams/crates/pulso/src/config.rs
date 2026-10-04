//! `pulso run` configuration: environment only (twelve-factor), validated up front, refusing with a named reason.
//! Nothing here ever prints a secret: `Secret` redacts in Debug and errors name the variable, never its value.
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataMode {
    Dataset,
    Platform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    Postgres,
    Memory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Missing(&'static str),
    Invalid { var: &'static str, reason: String },
    Conflict(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Missing(v) => write!(f, "config_missing: {v} is required"),
            ConfigError::Invalid { var, reason } => write!(f, "config_invalid: {var}: {reason}"),
            ConfigError::Conflict(r) => write!(f, "config_conflict: {r}"),
        }
    }
}
impl std::error::Error for ConfigError {}

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub storage: Storage,
    pub database_url: Option<Secret>,
    pub data_mode: DataMode,
    pub adapter: String,
    pub poll_interval: Duration,
    pub batch_cap: u32,
    pub listen_addr: SocketAddr,
    pub debug_token: Option<Secret>,
    pub admin_token: Option<Secret>,
    pub storage_prefix: Option<String>,
    pub store_dir: Option<PathBuf>,
    pub console_dir: Option<PathBuf>,
    pub grace: Duration,
    pub tenant: String,
    pub worker_id: String,
    pub exit_on_stdin_eof: bool,
}

impl RunConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(&|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let var = |k: &str| get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let invalid = |var: &'static str, reason: &str| ConfigError::Invalid { var, reason: reason.into() };
        let flag = |k: &str| matches!(var(k).as_deref(), Some("1" | "true" | "yes"));

        let data_mode = match var("PULSO_DATA_MODE").as_deref() {
            None => return Err(ConfigError::Missing("PULSO_DATA_MODE")),
            Some("dataset") => DataMode::Dataset,
            Some("platform") => DataMode::Platform,
            Some(_) => return Err(invalid("PULSO_DATA_MODE", "must be dataset or platform")),
        };

        let url = var("PULSO_DATABASE_URL");
        let storage = match (var("PULSO_STORAGE").as_deref(), &url) {
            (None, Some(_)) | (Some("postgres"), Some(_)) => Storage::Postgres,
            (Some("memory"), None) => Storage::Memory,
            (Some("memory"), Some(_)) => return Err(ConfigError::Conflict("PULSO_STORAGE=memory with PULSO_DATABASE_URL set: pick one".into())),
            (None, None) | (Some("postgres"), None) => {
                return Err(ConfigError::Missing("PULSO_DATABASE_URL (or PULSO_STORAGE=memory for an ephemeral local run)"));
            }
            (Some(_), _) => return Err(invalid("PULSO_STORAGE", "must be postgres or memory")),
        };
        if let Some(u) = &url
            && !(u.starts_with("postgres://") || u.starts_with("postgresql://"))
        {
            return Err(invalid("PULSO_DATABASE_URL", "must be a postgres:// or postgresql:// URL (value not shown)"));
        }

        let adapter = var("PULSO_SOURCE_ADAPTER").unwrap_or_else(|| "stub".into());
        let (allowed, other): (&[&str], &[&str]) = match data_mode {
            DataMode::Dataset => (&["stub", "e0-raw", "e0-augmented"], &["product-sqlite", "product-postgres"]),
            DataMode::Platform => (&["stub", "product-sqlite", "product-postgres"], &["e0-raw", "e0-augmented"]),
        };
        if other.contains(&adapter.as_str()) {
            let mode = if data_mode == DataMode::Dataset { "dataset" } else { "platform" };
            return Err(ConfigError::Conflict(format!("PULSO_DATA_MODE={mode} cannot use PULSO_SOURCE_ADAPTER={adapter}")));
        }
        if !allowed.contains(&adapter.as_str()) {
            return Err(invalid("PULSO_SOURCE_ADAPTER", &format!("unknown adapter; one of {}", allowed.join(", "))));
        }

        let num = |k: &'static str, default: u64, min: u64, max: u64| -> Result<u64, ConfigError> {
            match var(k) {
                None => Ok(default),
                Some(v) => match v.parse::<u64>() {
                    Ok(n) if (min..=max).contains(&n) => Ok(n),
                    _ => Err(invalid(k, &format!("must be a whole number in {min}..={max}"))),
                },
            }
        };
        let poll_interval = Duration::from_millis(num("PULSO_POLL_INTERVAL_MS", 30_000, 1, 86_400_000)?);
        let batch_cap = num("PULSO_BATCH_CAP", 100, 1, 100_000)? as u32;
        let grace = Duration::from_secs(num("PULSO_SHUTDOWN_GRACE_SECS", 50, 1, 3_600)?);

        let listen_addr: SocketAddr = var("PULSO_LISTEN_ADDR")
            .unwrap_or_else(|| "127.0.0.1:4020".into())
            .parse()
            .map_err(|_| invalid("PULSO_LISTEN_ADDR", "must be ip:port"))?;
        let debug_token = var("PULSO_DEBUG_TOKEN").map(Secret);
        let admin_token = var("PULSO_ADMIN_TOKEN").map(Secret);
        if !listen_addr.ip().is_loopback() {
            if !flag("PULSO_ALLOW_NON_LOOPBACK") {
                return Err(invalid("PULSO_LISTEN_ADDR", "non-loopback bind requires PULSO_ALLOW_NON_LOOPBACK=1"));
            }
            match &debug_token {
                Some(t) if t.expose().len() >= MIN_TOKEN => {}
                _ => return Err(invalid("PULSO_DEBUG_TOKEN", &format!("a token of at least {MIN_TOKEN} characters is mandatory on a non-loopback bind"))),
            }
            if let Some(a) = &admin_token
                && (a.expose().len() < MIN_TOKEN || Some(a) == debug_token.as_ref())
            {
                return Err(invalid("PULSO_ADMIN_TOKEN", &format!("on a non-loopback bind it must be at least {MIN_TOKEN} characters and differ from PULSO_DEBUG_TOKEN")));
            }
        }

        let storage_prefix = match var("PULSO_STORAGE_PREFIX") {
            None => None,
            Some(p) => {
                let ok = !p.starts_with('/')
                    && p.len() <= 256
                    && p.split('/').all(|s| s != "..")
                    && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'));
                if !ok {
                    return Err(invalid("PULSO_STORAGE_PREFIX", "relative key prefix of [A-Za-z0-9/_.-] without .."));
                }
                Some(p)
            }
        };

        Ok(RunConfig {
            storage,
            database_url: url.map(Secret),
            data_mode,
            adapter,
            poll_interval,
            batch_cap,
            listen_addr,
            debug_token,
            admin_token,
            storage_prefix,
            store_dir: var("PULSO_STORE_DIR").map(PathBuf::from),
            console_dir: var("PULSO_CONSOLE_DIR").map(PathBuf::from),
            grace,
            tenant: var("PULSO_TENANT").unwrap_or_else(|| "tenant-local".into()),
            worker_id: var("PULSO_WORKER_ID").unwrap_or_else(|| format!("pulso-{}", std::process::id())),
            exit_on_stdin_eof: flag("PULSO_EXIT_ON_STDIN_EOF"),
        })
    }
}

/// Minimum length of a bearer token accepted on a non-loopback bind.
pub const MIN_TOKEN: usize = 16;
