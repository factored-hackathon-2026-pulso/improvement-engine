//! Termination sources for `pulso run`: SIGTERM/SIGINT (unix), console control events (Windows), stdin EOF.
//! No external crate: a tiny FFI handler only raises an atomic flag; a watcher thread turns it into `StopToken::stop`.
use crate::run::supervisor::StopToken;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static SIGNALLED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
mod os {
    use super::*;
    unsafe extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }
    extern "C" fn on_signal(_: i32) {
        SIGNALLED.store(true, Ordering::SeqCst);
    }
    pub fn hook() {
        // SIGINT = 2, SIGTERM = 15. The handler only stores to an atomic (async-signal-safe).
        unsafe {
            signal(2, on_signal as extern "C" fn(i32) as usize);
            signal(15, on_signal as extern "C" fn(i32) as usize);
        }
    }
}

#[cfg(windows)]
mod os {
    use super::*;
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }
    unsafe extern "system" fn on_ctrl(kind: u32) -> i32 {
        // CTRL_C 0, CTRL_BREAK 1, CTRL_CLOSE 2, CTRL_LOGOFF 5, CTRL_SHUTDOWN 6
        if matches!(kind, 0 | 1 | 2 | 5 | 6) {
            SIGNALLED.store(true, Ordering::SeqCst);
            1
        } else {
            0
        }
    }
    pub fn hook() {
        unsafe {
            SetConsoleCtrlHandler(Some(on_ctrl), 1);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    pub fn hook() {}
}

/// Installs the OS handlers and a watcher thread that stops `stop` when one fires.
pub fn install(stop: StopToken) {
    os::hook();
    std::thread::Builder::new()
        .name("pulso-signals".into())
        .spawn(move || {
            while !stop.is_stopped() {
                if SIGNALLED.load(Ordering::SeqCst) {
                    stop.stop();
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .ok();
}

/// Stops when stdin reaches EOF (the launching parent died or closed the pipe). Opt-in: a container's stdin is /dev/null.
pub fn stop_on_stdin_eof(stop: StopToken) {
    std::thread::Builder::new()
        .name("pulso-stdin".into())
        .spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            stop.stop();
        })
        .ok();
}
