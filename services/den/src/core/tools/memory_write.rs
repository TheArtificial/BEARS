//! `den`-side re-exports for `memory_write_entry`.
//!
//! Orchestration (role gating, validation, source merging, entry construction)
//! lives in `den-tools` and is wired into the dispatcher via the concrete
//! [`RoleMemoryStore`](crate::core::tools::memory_read::DenRoleMemoryStore) under
//! `DenToolContext`.

#[cfg(test)]
pub(crate) use den_core::tools::memory::MemoryWriteEntryArguments;
