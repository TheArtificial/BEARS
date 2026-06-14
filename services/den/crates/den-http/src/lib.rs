//! Shared HTTP/identity foundation for the den edge crates (den-web / den-api / den-acp).
//!
//! See the crate-level rationale in `Cargo.toml`. Contents:
//! - [`errors`] — `CustomError`, the web-boundary adapter over `den_core::DenError`.
//! - [`auth_backend`] — the axum-login authentication/authorization backend.
//! - [`api_utils`] — small serde (de)serialization helpers for request payloads.
//! - [`user`] / [`email`] — the identity layer the auth backend depends on.
pub mod api_utils;
pub mod auth_backend;
pub mod email;
pub mod errors;
pub mod user;
