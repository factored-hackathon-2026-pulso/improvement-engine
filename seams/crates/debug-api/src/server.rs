//! tiny_http glue: one thread per request; open feeds hold a thread each (bounded). Loopback only.
use crate::app::App;
use std::sync::Arc;

/// Binds `addr`, refusing anything that is not a loopback address (the debug surface has no real authentication).
pub fn bind_loopback(addr: &str) -> Result<tiny_http::Server, String> {
    let _ = addr;
    todo!()
}

pub fn serve(server: tiny_http::Server, app: Arc<App>) {
    let _ = (server, app);
    todo!()
}
