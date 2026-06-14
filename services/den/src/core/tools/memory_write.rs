//! `den`-side re-exports for `memory_write_entry`.
//!
//! Orchestration (role gating, validation, source merging, entry construction)
//! lives in `den-tools` and is wired into the dispatcher via the concrete
//! [`RoleMemoryStore`](crate::core::tools::memory_read::DenRoleMemoryStore) under
//! `DenToolContext`. These re-exports preserve the `den` paths still referenced
//! across the crate (`source_acp_session_id`) and in tests.

pub(crate) use den_core::tools::memory::source_acp_session_id;
#[cfg(test)]
pub(crate) use den_core::tools::memory::MemoryWriteEntryArguments;
