//! Seam client to the Core bridge `/internal/v1`. Talks over HTTP only and never links `crates/core`.
//! K1 scope: transport (Ed25519 service JWT), error classifier, idempotency, generated pin constants.
//! K2 scope: typed operations (`ops`), hand-typed 1.3.0 DTOs (`dto`) and wire-format helpers (`canon`).
pub mod arms;
pub mod canon;
pub mod client;
pub mod dto;
pub mod errors;
mod generated;
pub mod http;
pub mod jwt;
pub mod ops;
pub mod pins;
pub mod routes;

pub use client::{CallError, ClientConfig, CoreClient, Response};
pub use ops::OpError;
