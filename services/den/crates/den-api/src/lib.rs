//! JSON/REST + OAuth HTTP edge of the den binary.
//!
//! Owns the public API service (`create_api_app` / `service`), the v1
//! user/profile/oauth routes (`v1`), and the OpenAPI docs (`docs`). Peer HTTP
//! edges (e.g. the ACP edge in `den-acp`) are injected by the binary composition
//! root as peer routers; this crate has no dependency on any sibling edge and
//! shares the protocol-neutral `den_runtime::DenState` (ADR-0043).
//!
//! It sits above den-http (identity/error foundation), den-oauth,
//! den-runtime (tool/runtime execution), den-docket, and den-core.

pub mod docs;
pub mod service;
pub mod v1;

// Public entry point: the binary calls this to build the API/ACP app.
pub use service::create_api_app;
