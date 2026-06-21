//! Compatibility re-export for shared Den application state.
//!
//! `DenState` lives in `den-service` so HTTP edges can share app state without
//! depending on the runtime execution crate.

pub use den_service::state::*;
