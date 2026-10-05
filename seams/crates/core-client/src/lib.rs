//! Seam client to the Core bridge `/internal/v1`. Talks over HTTP only and never links `crates/core`.
//! K1 scope: transport (Ed25519 service JWT), error classifier, idempotency, generated pin constants.
//! K2 scope: typed operations (`ops`), hand-typed 1.3.0 DTOs (`dto`) and wire-format helpers (`canon`).
//! K3 scope (PARTIAL): the compile writer path (`writer`): draft plan, sealed commitment, writer-stage invoke and
//! commitment verification, alias readback check. K3 acceptance (Prompt+EvalSuite published to staging on the real
//! image) is NOT met: the Rust registry client + human-JWS authorizer now exist (`registry`, `authorizer`; claude-standin
//! human, fake-registry tested, live test ignored) but were not run on the real image, and a contract route to seal the draft artifact (today the stand-in `/_e2e/config`: a BRG1 gap candidate). Only
//! the ignored live test `live_k3_freeze_draft_matches_the_dry_run` touches a real stack, and it stops at the freeze.
pub mod admission;
pub mod arms;
pub mod authorizer;
pub mod authoring;
pub mod canon;
pub mod client;
pub mod dto;
pub mod errors;
pub mod evaluate;
mod generated;
pub mod http;
pub mod jwt;
pub mod ops;
pub mod reconcile;
pub mod pins;
pub mod registry;
pub mod routes;
pub mod trace;
pub mod writer;

pub use client::{CallError, ClientConfig, CoreClient, Response};
pub use ops::OpError;
