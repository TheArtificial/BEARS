//! Compatibility re-export for client/edge tool policy helpers.
//!
//! Stable client-tool vocabulary lives in `den-core` so web/API edges can use it
//! without depending on runtime execution internals.

pub use den_core::client_tools::*;
