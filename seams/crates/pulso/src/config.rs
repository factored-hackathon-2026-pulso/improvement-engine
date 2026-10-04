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
    pub grace: Duration,
    pub tenant: String,
    pub worker_id: String,
    pub exit_on_stdin_eof: bool,
}

impl RunConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(&|k| std::env::var(k).ok())
    }

    pub fn from_lookup(_get: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        Err(ConfigError::Missing("PULSO_DATA_MODE"))
    }
}
