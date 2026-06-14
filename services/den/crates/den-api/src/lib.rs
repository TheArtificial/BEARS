//! Standalone API service (`src/api/`)
//!
//! This module provides an independent API service for apps built from this starter that can run
//! separately from or alongside the web service. It implements OAuth 2.0
//! authorization server functionality and external API access for third parties.
//!
//! # Architecture
//!
//! **HTTP surface in this starter:**
//! - **`src/api/`** (this module): standalone OAuth 2.0 authorization server, OpenAPI docs, and JSON API (`/v1.0/`, etc.).
//! - **`src/web/`**: server-rendered UI (Axum routes under `public`, `user`, `admin`, `home`, …)—add product-specific routes here, not a separate `web/api` tree.
//!
//! The API service is designed with the same architectural patterns as the web
//! service but maintains complete independence. This allows for:
//! - Separate deployment and scaling (port 3001 vs web service port 3000)
//! - Independent authentication and session management
//! - Dedicated API-focused middleware and error handling
//! - Future distribution across multiple crates
//!
//! # Endpoints Provided
//!
//! - **OAuth 2.0** (`/oauth/`): Complete authorization server (RFC 6749, RFC 7636)
//! - **API Documentation** (`/api-docs/`): OpenAPI documentation for external API
//! - **v1.0 API** (`/v1.0/`): External API endpoints for third-party access
//!
//! # OAuth 2.0 Provider
//!
//! The API service implements a complete OAuth 2.0 authorization server following
//! RFC 6749, including:
//! - Authorization endpoint for user consent
//! - Token endpoint for access token exchange
//! - User info endpoint for profile data
//! - PKCE support for enhanced security (RFC 7636)
//!
//! # Security
//!
//! - Integrates with existing axum-login authentication system
//! - Uses PostgreSQL session store shared with web service
//! - Implements proper CORS for cross-origin OAuth flows
//! - Follows "safe code" principles with comprehensive error handling
//!
//! # Deployment
//!
//! Can be run independently or alongside the web service:
//! - `SERVER_MODE=api` - Run only API service (port 3001)
//! - `SERVER_MODE=web` - Run only web service (port 3000)
//! - `SERVER_MODE=both` - Run both services simultaneously

// Self-alias so the migrated edge keeps resolving its historical `crate::api::*`
// paths (this crate *is* the old `den::api` module tree). Lets the v1.5 extraction
// land without rewriting ~200 `api::`-prefixed paths; a v2 cleanup can drop these.
extern crate self as api;

pub mod acp;
#[cfg(test)]
mod acp_turn_state_alignment_tests;
#[cfg(test)]
mod acp_workflow_state_tests;
pub mod auth;
// Residual native ACP protocol modules, kept under `crate::core::acp` so the
// migrated `acp` call sites resolve unchanged (v1.5 den-api extraction).
pub mod core;
pub mod docs;
pub mod internal;
pub mod oauth;
pub mod service;
pub mod templates;
pub mod v1;

// Re-export main API service creation function
pub use service::create_api_app;

// Foundation re-export shims (v1.5 den-api extraction): keep the migrated call
// sites resolving `crate::errors`, `crate::config`, `crate::auth_backend`, and
// `crate::build_info` unchanged. These types live in den-http / den-core now.
pub use den_core::config;
pub use den_http::{auth_backend, build_info, errors};

use std::sync::{Arc, OnceLock};

static TOOL_INVOKER: OnceLock<Arc<dyn den_runtime::native_runtime::RuntimeToolInvoker>> =
    OnceLock::new();

/// Install the process-wide builtin-Den-tool invoker.
///
/// The api/ACP edge executes builtin Den tools (the `/internal/den-tools/invoke`
/// endpoint and ACP runtime-local tool calls) but depends only on the
/// [`den_runtime::native_runtime::RuntimeToolInvoker`] trait — not on the concrete
/// den-side tool composition (`DenToolContext` + executors), which lives in the
/// `den` binary. The binary injects its `DenRuntimeToolInvoker` here at startup, so
/// the edge stays free of that dependency. Idempotent; the first installation wins.
pub fn set_tool_invoker(invoker: Arc<dyn den_runtime::native_runtime::RuntimeToolInvoker>) {
    let _ = TOOL_INVOKER.set(invoker);
}

/// The installed [`set_tool_invoker`] invoker, if any. `None` before the binary
/// installs one (e.g. in unit tests that never execute a builtin Den tool).
pub fn tool_invoker() -> Option<Arc<dyn den_runtime::native_runtime::RuntimeToolInvoker>> {
    TOOL_INVOKER.get().cloned()
}
