//! control-api LITE: artifacts, authorization-checks and core-task-bindings over `tiny_http`.
//! `App::handle` is a pure request -> response function (testable without sockets); `server` is the thin tiny_http glue.
//! The store sits behind `store::Store`; only an in-memory implementation exists (a Postgres store is a later package).
pub mod app;
pub mod auth;
pub mod debug;
pub mod ingest;
pub mod lab;
pub mod server;
pub mod sse;
pub mod store;
