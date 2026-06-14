//! Tool surface for the api/ACP edge.
//!
//! Re-exports the canonical den-core tool modules (constants, descriptor, aliases,
//! arguments, context, support, …) and provides the thin den-side glue the edge
//! needs that is not itself in den-core: the `CustomError`-returning invocation
//! wrapper (`session`), the native-runtime MemFS client-tool guards (`memfs`,
//! re-exported from den-runtime), and the `memory_write` argument re-export used by
//! tests. This mirrors the binary's former `core::tools` shim layer; the canonical
//! logic lives in den-core / den-runtime.
pub use den_core::tools::*;

/// Native-runtime MemFS client-tool guards (`filter_client_tools_for_native_runtime`,
/// `is_memfs_client_tool_name`, …) — canonical home is `den_runtime::native_runtime::memfs`.
pub mod memfs {
    pub use den_runtime::native_runtime::memfs::*;
}

pub mod session;

/// `den`-side re-exports for `memory_write_entry` argument shapes (test-only here).
pub mod memory_write {
    #[cfg(test)]
    pub use den_core::tools::memory::MemoryWriteEntryArguments;
}
