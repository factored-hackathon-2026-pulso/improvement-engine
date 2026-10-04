//! `App::handle`: a pure request -> response function over the store (testable without sockets); `server` is the tiny_http glue.
use crate::store::Store;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub struct Config {
    pub tenant: String,
    /// Static local-dev bearer token for the debug routes; `None` = open (loopback only).
    pub token: Option<String>,
    /// Admin (append/ingest/purge) token; `None` = the admin surface does not exist (404).
    pub admin_token: Option<String>,
    pub heartbeat: Duration,
}

impl Default for Config {
    fn default() -> Config {
        Config { tenant: "tenant-local".into(), token: None, admin_token: None, heartbeat: Duration::from_secs(5) }
    }
}

pub struct Req {
    pub method: String,
    pub path: String,
    pub query: String,
    /// Lower-cased names.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct StreamPlan {
    pub run: String,
    pub after: i64,
}

pub struct App {
    pub(crate) store: Arc<Store>,
    pub(crate) cfg: Config,
}

impl App {
    pub fn new(store: Arc<Store>, cfg: Config) -> App {
        App { store, cfg }
    }
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }
    pub fn heartbeat(&self) -> Duration {
        self.cfg.heartbeat
    }
    pub fn handle(&self, r: &Req) -> Resp {
        let _ = r;
        todo!()
    }
    /// `None`: not a stream route. `Some(Err)`: answer this response. `Some(Ok)`: the server should stream.
    pub fn stream_route(&self, r: &Req) -> Option<Result<StreamPlan, Resp>> {
        let _ = r;
        todo!()
    }
}
