//! JSON/REST + OAuth HTTP edge of the den binary.
//!
//! Owns the public API service (`create_api_app` / `service`), the v1
//! user/profile/oauth routes (`v1`), and the OpenAPI docs (`docs`). It composes
//! the ACP surface by mounting `den_acp::acp` + `den_acp::internal` routers and
//! sharing `den_acp::service::ApiState`.
//!
//! It sits above den-acp, den-http (identity/error foundation), den-oauth,
//! den-runtime (tool/runtime execution), den-docket, and den-core.

pub mod docs;
pub mod service;
pub mod v1;

// Public entry point: the binary calls this to build the API/ACP app.
pub use service::create_api_app;
