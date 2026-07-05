//! Shared HTTP/identity foundation for the Den edge crates.
//!
//! See the crate-level rationale in `Cargo.toml`. Contents:
//! - [`errors`] — `CustomError`, the web-boundary adapter over `den_core::DenError`.
//! - [`auth_backend`] — the axum-login authentication/authorization backend.
//! - [`api_utils`] — small serde (de)serialization helpers for request payloads.
//! - [`user`] / [`email`] — the identity layer the auth backend depends on.
//! - [`web_policy`] — bear web-fetch approval/source policy (shared by the api edge
//!   and the web admin/settings handlers); kept here because it returns `CustomError`.
pub mod api_utils;
pub mod armature_tokens;
pub mod auth_backend;
pub mod build_info;
pub mod email;
pub mod errors;
pub mod user;
pub mod web_policy;
