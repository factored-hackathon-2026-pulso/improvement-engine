//! Postgres seam (MIG0): migration runner, role bootstrap and schema snapshot.

pub mod migrate;
pub mod roles;
pub mod schema;

pub use migrate::{Error, Migration, Report};
