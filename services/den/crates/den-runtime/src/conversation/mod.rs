//! Compatibility re-export for canonical conversation storage/read models.
//!
//! Conversation persistence lives in `den-service` so HTTP/read-model edges can
//! use it without depending on runtime execution internals.

pub use den_service::conversation::*;

// Integration test lives here (not in den-service): it exercises `bears::db` plus the
// flattened crate-root conversation paths, which only resolve in this crate.
#[cfg(test)]
mod persistence_idempotency_integration_tests {
    include!("persistence_idempotency_integration_tests.rs");
}
