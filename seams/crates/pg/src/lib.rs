//! Postgres seam (MIG0): migration runner, role bootstrap and schema snapshot; job repository port,
//! its conformance suite, and the Postgres-backed implementations (`PgRepo`, engine `PgJobStore`).

pub mod conformance;
pub mod migrate;
pub mod pgrepo;
pub mod pgstore;
pub mod repo;
pub mod roles;
pub mod schema;

pub use migrate::{Error, Migration, Report};
