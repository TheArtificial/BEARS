//! Compatibility re-export for Bear registry/domain services.
//!
//! Bear DB/domain/read-model helpers live in `den-service` so HTTP edges can use
//! them without depending on runtime execution internals.

pub use den_service::bears::*;
