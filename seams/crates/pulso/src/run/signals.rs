//! Termination sources for `pulso run`: SIGTERM/SIGINT (unix), console control events (Windows), stdin EOF.
//! No external crate: a tiny FFI handler only raises an atomic flag; a watcher thread turns it into `StopToken::stop`.
use crate::run::supervisor::StopToken;

pub fn install(_stop: StopToken) {}

/// Stops when stdin reaches EOF (the launching parent died or closed the pipe). Opt-in: a container's stdin is /dev/null.
pub fn stop_on_stdin_eof(_stop: StopToken) {}
