//! ACP (Agent Client Protocol) HTTP edge of the den binary.
//!
//! Carved out of `den-api` (v1.5+ den-acp sub-split). Owns the ACP protocol HTTP
//! surface (`acp/**`), the residual native ACP protocol modules (`core/acp/*`:
//! sessions / tokens / runtime / turn_runner), the shared [`service::ApiState`]
//! (Option B: the lower edge owns the state both surfaces share), and the
//! `/internal/den-tools/invoke` endpoint.
//!
//! It sits below `den-api` (which mounts `acp::router()` + `internal::router()`)
//! and above den-oauth / den-http / den-runtime / den-docket / den-core.

// Self-alias so the migrated edge keeps resolving its historical `crate::api::*`
// paths (this code is the old `den::api::acp` / `den::api::core` tree).
extern crate self as api;

pub mod acp;
pub mod core;
pub mod internal;
pub mod service;

#[cfg(test)]
mod acp_turn_state_alignment_tests;
#[cfg(test)]
mod acp_workflow_state_tests;

pub use service::ApiState;

// Foundation re-export shims so the migrated ACP edge resolves `crate::config`,
// `crate::errors`, `crate::auth_backend`, `crate::build_info`, and the OAuth
// bearer-auth layer (`crate::auth` / `crate::oauth`) unchanged. Canonical homes
// are den-core (config), den-http (errors/auth_backend/build_info), den-oauth.
pub use den_core::config;
pub use den_http::{auth_backend, build_info, errors};
pub use den_oauth::{auth, oauth};

// The builtin-Den-tool invoker registry lives in den-runtime; re-exported here so
// `internal.rs` and `acp/stream/runtime.rs` resolve `crate::tool_invoker()`.
pub use den_runtime::native_runtime::{set_tool_invoker, tool_invoker};
