//! Compatibility re-export for runtime protocol contracts.
//!
//! Stable DTOs/events live in `den-protocol` so edge crates can depend on a
//! lightweight protocol boundary instead of the full runtime implementation.

pub use den_protocol::*;
