//! Compatibility re-export for Den model registry helpers.
//!
//! Model registry data lives in `den-llm` so UI/service crates can use it without
//! depending on runtime execution internals.

pub use den_llm::model_registry::*;
