//! Re-export shim. The native-runtime MemFS client-tool guards now live in
//! `den_runtime::native_runtime::memfs`; this keeps existing
//! `crate::core::tools::memfs::*` callers resolving unchanged.

pub use den_runtime::native_runtime::memfs::{
    filter_client_tools_for_native_runtime, is_memfs_client_tool_name,
};
