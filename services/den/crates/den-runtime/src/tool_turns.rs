//! Compatibility re-export for process-local tool turn coordination.
//!
//! The concrete coordinator lives in `den-service` so HTTP edges can share app
//! state without depending on the runtime execution crate.

pub use den_service::tool_turns::*;
