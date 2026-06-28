//! Residual native ACP protocol modules (sessions / tokens / runtime / turn_runner).
//!
//! Kept under `crate::core::acp` (with the historical `acp_runtime` / `armature_tokens` /
//! `acp_turn_runner` aliases) so the migrated `acp` HTTP edge resolves its
//! `crate::core::acp*` paths unchanged after the v1.5 den-api extraction. The
//! den-acp sub-split (separating this + `acp` into their own crate) is a v2 follow-up.
pub mod acp;
pub use acp::runtime as acp_runtime;
pub use acp::tokens as armature_tokens;
pub use acp::turn_runner as acp_turn_runner;

// Foundation re-export shims (v1.5): keep `crate::core::user` / `crate::core::api_utils`
// / `crate::core::email` resolving for the migrated api edge. They live in den-http now.
pub use den_http::{api_utils, email, user};

// Tool/docket shims so the migrated edge keeps resolving `crate::core::tools*`,
// `crate::core::web_policy`, `crate::core::docket`, and `crate::core::work_plans`.
// Canonical homes are den-core (tools), den-http (web_policy), and den-docket
// (docket / work plans). `tools` re-exports den-core's tool modules plus the thin
// invocation glue the edge needs (see `tools::mod`).
pub mod tools;
pub use den_http::web_policy;
pub use den_docket as docket;
pub use den_docket as work_plans;
