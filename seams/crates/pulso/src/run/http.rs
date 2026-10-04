//! The HTTP surface of `pulso run`: `/healthz` and `/readyz` in front of the embedded debug-api / console.
//! Probes are unauthenticated (an ECS or load-balancer check carries no token); every other route is the debug-api's,
//! behind its bearer token. Non-loopback binds were already vetted by `RunConfig` (opt-in plus mandatory token).
use crate::config::RunConfig;
use crate::health::Health;
use crate::run::supervisor::{StopToken, Task};
use debug_api::server::{Front, serve_with};
use debug_api::{App, Config, Resp, Store};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;

pub struct HttpTask {
    server: Option<tiny_http::Server>,
    app: Arc<App>,
    health: Arc<Health>,
    addr: SocketAddr,
}

impl HttpTask {
    pub fn bind(cfg: &RunConfig, health: Arc<Health>, store: Arc<Store>) -> Result<HttpTask, String> {
        let server = tiny_http::Server::http(cfg.listen_addr).map_err(|e| format!("bind {}: {e}", cfg.listen_addr))?;
        let addr = server.server_addr().to_ip().ok_or("listener is not an IP socket")?;
        let console = cfg.console_dir.clone().filter(|d| d.join("index.html").is_file());
        let config_json = json!({"provider": "stand-in", "dataProvider": "http", "apiBase": "", "sseHeartbeatMs": 5000, "traceLinkOrigins": []}).to_string();
        let dcfg = Config {
            tenant: cfg.tenant.clone(),
            token: cfg.debug_token.as_ref().map(|t| t.expose().to_string()),
            admin_token: cfg.admin_token.as_ref().map(|t| t.expose().to_string()),
            static_dir: console,
            config_json: Some(config_json),
            ..Config::default()
        };
        Ok(HttpTask { server: Some(server), app: Arc::new(App::new(store, dcfg)), health, addr })
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
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
        let front: Front = Arc::new(move |r| {
            if r.method != "GET" && r.method != "HEAD" {
                return None;
            }
            match r.path.as_str() {
                "/healthz" => {
                    let (s, b) = health.healthz();
                    Some(json_resp(s, b))
                }
                "/readyz" => {
                    let (s, b) = health.readyz();
                    Some(json_resp(s, b))
                }
                _ => None,
            }
        });
        let st = stop.clone();
        serve_with(server, self.app.clone(), Some(front), Arc::new(move || st.is_stopped()));
        Ok(())
    }
}
