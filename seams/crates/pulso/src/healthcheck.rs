//! `pulso healthcheck`: container HEALTHCHECK for images without curl/wget.
use std::time::Duration;

/// GET http://127.0.0.1:<port><base_path>/readyz; Ok only on HTTP 200. Errors never carry response bodies or secrets.
pub fn check(_port: u16, _base_path: &str, _timeout: Duration) -> Result<(), String> {
    Err("unimplemented".into())
}

/// Entry for `pulso healthcheck [--port N]`: port from the flag, else PULSO_LISTEN_ADDR, else 8080. Exit 0 healthy.
pub fn main(_args: &[String]) -> i32 {
    2
}
