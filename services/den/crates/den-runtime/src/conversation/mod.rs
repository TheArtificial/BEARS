//! Compatibility re-export for canonical conversation storage/read models.
//!
//! Conversation persistence lives in `den-service` so HTTP/read-model edges can
//! use it without depending on runtime execution internals.

pub use den_service::conversation::*;
