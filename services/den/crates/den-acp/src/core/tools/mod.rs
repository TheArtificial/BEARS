//! Tool surface for the api/ACP edge.
//!
//! Re-exports the canonical den-core tool modules (constants, descriptor, aliases,
//! arguments, context, support, …) and provides the thin den-side glue the edge
//! needs that is not itself in den-core: the `CustomError`-returning invocation
//! wrapper (`session`), the native-runtime legacy memory client-tool guards,
//! re-exported from den-runtime), and the `memory_write` argument re-export used by
//! tests. This mirrors the binary's former `core::tools` shim layer; the canonical
//! logic lives in den-core / den-runtime.
pub use den_core::tools::*;

/// Native-runtime legacy memory client-tool guards — canonical home is
/// `den_runtime::native_runtime::legacy_memory_tools`.
pub mod legacy_memory_tools {
    pub use den_runtime::native_runtime::legacy_memory_tools::*;
}

pub mod session;

/// `den`-side re-exports for `memory_write_entry` argument shapes (test-only here).
pub mod memory_write {
    #[cfg(test)]
    pub use den_core::tools::memory::MemoryWriteEntryArguments;
}
