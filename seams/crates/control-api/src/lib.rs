//! control-api LITE: artifacts, authorization-checks and core-task-bindings over `tiny_http`.
//! `App::handle` is a pure request -> response function (testable without sockets); `server` is the thin tiny_http glue.
//! The store sits behind `store::Store`: `store::MemStore` (memory) or `pgstore::PgStore` (Postgres, migration 0052, durable).
pub mod app;
pub mod auth;
pub mod correlation;
pub mod debug;
pub mod ingest;
pub mod lab;
pub mod pgstore;
pub mod server;
pub mod sse;
pub mod store;
