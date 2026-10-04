//! debug-api: the run-event store the engine writes (`RunEventSink`) and the `/internal/v1/debug` read surface the
//! debug-console consumes (JSON routes plus SSE with `Last-Event-ID` resume and 410 snapshot recovery).
//! Read-only by default; loopback only; never depends on `crates/core`.
pub mod event;
pub mod project;
pub mod store;

pub use event::{NewEvent, RunEventSink};
pub use store::Store;
