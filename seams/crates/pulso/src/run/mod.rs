//! `pulso run`: the single-process runtime entrypoint (supervisor, tasks, health, logs).
pub mod log;
pub mod supervisor;
pub mod http;
pub mod tasks;
pub mod db;
pub mod signals;

/// `pulso run`. Exit 0 clean stop; 1 failed task or startup failure; 2 refused config/usage; 3 cut at the deadline.
pub fn main(_args: &[String]) -> i32 {
    2
}
