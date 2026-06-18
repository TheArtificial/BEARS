//! ACP (Agent Client Protocol) HTTP edge of the den binary.
//!
//! Carved out of `den-api` (v1.5+ den-acp sub-split). Owns the ACP protocol HTTP
//! surface (`acp/**`), the residual native ACP protocol modules (`core/acp/*`:
//! sessions / tokens / runtime / turn_runner), the shared [`service::DenState`]
//! (Option B: the lower edge owns the state both surfaces share), and the
//! `/internal/den-tools/invoke` endpoint.
//!
//! It sits below `den-api` (which mounts `acp::router()` + `internal::router()`)
//! and above den-oauth / den-http / den-runtime / den-docket / den-core.

pub mod acp;
pub mod bearwire;
pub mod core;
pub mod internal;
pub mod service;

#[cfg(test)]
mod acp_turn_state_alignment_tests;
#[cfg(test)]
mod acp_workflow_state_tests;

pub use service::DenState;
