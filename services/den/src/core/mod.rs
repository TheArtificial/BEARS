// The native agent runtime moved to the `den-runtime` crate during the v1.4 split.
// Modules below depend on it directly via `den_runtime::*` (the former flat
// `pub use den_runtime::*` shims here were dropped in the final flip). What remains
// in `core` are the den-binary-local subsystems.
// `api_utils`, `email`, and `user` moved to the shared edge foundation crate
// `den-http` (v1.5 split); re-exported here so `crate::core::*` call sites are
// unchanged until the edges are extracted.
pub use den_http::{api_utils, email, user};
pub mod sandbox;
pub mod tools;
pub use tools::tool_descriptor_guidance;
pub use tools::web_policy;

// Cross-layer bridge tests: they exercise den_runtime modules together with
// den-only modules (native_runtime / turn_controller), so they live here in the
// `den` crate rather than in den-runtime. Relocated during the v1.4 runtime lift.
#[cfg(test)]
mod conversation_persistence_non_acp_bridge_tests;
#[cfg(test)]
mod reflection_conductor_bridge_tests;
#[cfg(test)]
mod reflection_conversations_bridge_tests;
#[cfg(test)]
mod runtime_bearwire_bridge_tests;
#[cfg(test)]
mod runtime_role_bridge_tests;
