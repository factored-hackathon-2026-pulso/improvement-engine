//! The HTTP surface of `pulso run`: `/healthz` and `/readyz` in front of the embedded debug-api / console.
//! Probes are unauthenticated (an ECS or load-balancer check carries no token); every other route is the debug-api's,
//! behind its bearer token. Non-loopback binds were already vetted by `RunConfig` (opt-in plus mandatory token).
use crate::config::RunConfig;
use crate::health::Health;
use crate::run::supervisor::{StopToken, Task};
use debug_api::server::{Front, serve_with};
use debug_api::automation::{Automation, TriggerAdmitter};
use debug_api::{App, Config, Resp, Store};
use pg::repo::JobRepository;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;

pub struct HttpTask {
    server: Option<tiny_http::Server>,
    app: Arc<App>,
    health: Arc<Health>,
    addr: SocketAddr,
    base: String,
}

/// The automation trigger endpoint admits into the real job store: same keyed, idempotent contract as the monitor hand-over.
pub struct RepoAdmitter(pub Arc<dyn JobRepository>);

impl TriggerAdmitter for RepoAdmitter {
    fn admit_keyed(&self, tenant: &str, key: &str) -> Result<String, String> {
        self.0.admit_keyed(tenant, key).map_err(|e| format!("{e:?}"))
    }
}

impl HttpTask {
    /// Without a job repository the automation surface does not exist (404), as before.
    pub fn bind(cfg: &RunConfig, health: Arc<Health>, store: Arc<Store>) -> Result<HttpTask, String> {
        HttpTask::bind_with(cfg, health, store, None)
    }

    /// With a repository, `POST /internal/v1/automation/triggers` admits `trigger:<key>` jobs into it (the runner executes them).
    pub fn bind_with(cfg: &RunConfig, health: Arc<Health>, store: Arc<Store>, repo: Option<Arc<dyn JobRepository>>) -> Result<HttpTask, String> {
        let automation = match repo {
            Some(r) => Some(Arc::new(Automation::from_json(None, None, None)?.with_admitter(Arc::new(RepoAdmitter(r))))),
            None => None,
        };
        let server = tiny_http::Server::http(cfg.listen_addr).map_err(|e| format!("bind {}: {e}", cfg.listen_addr))?;
        let addr = server.server_addr().to_ip().ok_or("listener is not an IP socket")?;
        let console = cfg.console_dir.clone().filter(|d| d.join("index.html").is_file());
        let config_json = json!({"provider": "stand-in", "dataProvider": "http", "apiBase": cfg.base_path, "sseHeartbeatMs": 5000, "traceLinkOrigins": []}).to_string();
        let dcfg = Config {
            tenant: cfg.tenant.clone(),
            token: cfg.debug_token.as_ref().map(|t| t.expose().to_string()),
            admin_token: cfg.admin_token.as_ref().map(|t| t.expose().to_string()),
            automation,
            static_dir: console,
            config_json: Some(config_json),
            ..Config::default()
        };
        Ok(HttpTask { server: Some(server), app: Arc::new(App::new(store, dcfg)), health, addr, base: cfg.base_path.clone() })
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

fn json_resp_pair((s, b): (u16, serde_json::Value)) -> Resp {
    json_resp(s, b)
}

fn json_resp(status: u16, body: serde_json::Value) -> Resp {
    Resp { status, headers: vec![("Content-Type".into(), "application/json".into()), ("Cache-Control".into(), "no-store".into())], body: body.to_string().into_bytes() }
}

impl Task for HttpTask {
    fn name(&self) -> String {
        "http".into()
    }
    fn run(&mut self, stop: &StopToken) -> Result<(), String> {
        let server = self.server.take().ok_or("http task already ran")?;
        let health = self.health.clone();
        let base = self.base.clone();
        let front: Front = Arc::new(move |r| {
            let probe = |p: &str| -> Option<Resp> {
                match p {
                    "/healthz" => Some(json_resp_pair(health.healthz())),
                    "/readyz" => Some(json_resp_pair(health.readyz())),
                    _ => None,
                }
            };
            let read = r.method == "GET" || r.method == "HEAD";
            if base.is_empty() {
                return if read { probe(&r.path) } else { None };
            }
            // Behind a proxy prefix: only paths under it reach the debug-api (prefix stripped); bare probes stay open.
            let rel = if r.path == base { Some("/".to_string()) } else { r.path.strip_prefix(&base).filter(|t| t.starts_with('/')).map(String::from) };
            match rel {
                Some(rel) if rel.split('/').any(|s| s == ".." || s == ".") => Some(json_resp(404, json!({"code": "not_found"}))),
                Some(rel) => {
                    if read && let Some(resp) = probe(&rel) {
                        return Some(resp);
                    }
                    r.path = rel;
                    None
                }
                None => Some(if read { probe(&r.path) } else { None }.unwrap_or_else(|| json_resp(404, json!({"code": "not_found"})))),
            }
        });
        let st = stop.clone();
        serve_with(server, self.app.clone(), Some(front), Arc::new(move || st.is_stopped()));
        Ok(())
    }
}
