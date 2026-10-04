//! Seam client to the Core bridge `/internal/v1`. Talks over HTTP only and never links `crates/core`.
//! K1 scope: transport (Ed25519 service JWT), error classifier, idempotency, generated pin constants.
//! Typed DTOs per operation arrive in K2.
pub mod client;
pub mod dto;
pub mod errors;
mod generated;
pub mod http;
pub mod jwt;
pub mod pins;
pub mod routes;

pub use client::{CallError, ClientConfig, CoreClient, Response};
