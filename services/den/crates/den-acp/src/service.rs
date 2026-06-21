//! Shared application state for the API + ACP HTTP surface.
//!
//! `DenState` now lives in `den-runtime` (below every HTTP edge) so that neither
//! the JSON/REST edge nor the ACP edge owns the state both surfaces share — see
//! ADR-0043 (ACP is an edge adapter; the Den runtime is protocol-agnostic). It is
//! re-exported here so existing `den_acp::service::DenState` paths keep resolving.

pub use den_service::DenState;
