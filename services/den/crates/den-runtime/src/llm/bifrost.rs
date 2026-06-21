//! Compatibility re-export for Bifrost model metadata client.
//!
//! The concrete client lives in `den-service` so shared edge state can sit below
//! the runtime execution crate.

pub use den_service::bifrost::*;
