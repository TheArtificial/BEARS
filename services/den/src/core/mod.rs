// The native agent runtime moved to the `den-runtime` crate during the v1.4 split.
// Modules below depend on it directly via `den_runtime::*` (the former flat
// `pub use den_runtime::*` shims here were dropped in the final flip). What remains
// in `core` are the den-binary-local subsystems and the den-side ACP edge.
pub mod acp;
pub use acp::runtime as acp_runtime;
pub use acp::tokens as acp_tokens;
pub use acp::turn_runner as acp_turn_runner;
pub mod api_utils;
pub mod sandbox;
pub mod migration;
pub mod tools;
pub mod email;
pub mod docket;
pub mod s3;
pub use tools::tool_descriptor_guidance;
pub mod user;
pub use tools::web_policy;
pub mod work_plans;

// Cross-layer bridge tests: they exercise den_runtime modules together with
// den-only modules (native_runtime / acp_turn_controller), so they live here in the
// `den` crate rather than in den-runtime. Relocated during the v1.4 runtime lift.
#[cfg(test)]
mod runtime_role_bridge_tests;
#[cfg(test)]
mod runtime_bearwire_bridge_tests;
#[cfg(test)]
mod conversation_persistence_non_acp_bridge_tests;
#[cfg(test)]
mod reflection_conductor_bridge_tests;
#[cfg(test)]
mod reflection_conversations_bridge_tests;
