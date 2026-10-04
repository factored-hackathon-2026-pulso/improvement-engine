//! The HTTP surface of `pulso run`: `/healthz` and `/readyz` in front of the embedded debug-api / console.
use crate::config::RunConfig;
use crate::health::Health;
use crate::run::supervisor::{StopToken, Task};
use debug_api::{App, Store};
use std::net::SocketAddr;
use std::sync::Arc;

pub struct HttpTask {
    addr: SocketAddr,
}

impl HttpTask {
    pub fn bind(_cfg: &RunConfig, _health: Arc<Health>, _store: Arc<Store>) -> Result<HttpTask, String> {
        let _ = std::any::type_name::<App>();
        Err("unimplemented".into())
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Task for HttpTask {
    fn name(&self) -> String {
        "http".into()
    }
    fn run(&mut self, _stop: &StopToken) -> Result<(), String> {
        Ok(())
    }
}
