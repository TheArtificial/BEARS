//! Compatibility surface for the den-side subsystems the web edge still references
//! as `crate::core::*`.
//!
//! The canonical homes are the foundation/runtime crates: identity (`user`,
//! `email`) and web policy live in den-http, ACP tokens in den-api, work plans /
//! docket in den-docket, and the tool descriptors/constants in den-core. Only S3
//! media storage (`s3`) is web-local. Builtin Den tool *execution* is injected via
//! `den_api::tool_invoker`, so no tool composition lives here.
pub mod s3;

pub use den_api::core::acp_tokens;
pub use den_docket as docket;
pub use den_docket as work_plans;
pub use den_http::{email, user, web_policy};

/// Re-exports of den-core tool descriptor/constant/argument modules plus the
/// per-call invocation context (`session::DenToolInvocationContext`) the web chat
/// path threads through the injected invoker.
pub mod tools {
    pub use den_core::tools::*;

    pub mod session {
        pub use den_core::tools::context::DenToolInvocationContext;
    }
}
