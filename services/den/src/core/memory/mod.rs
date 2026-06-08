//! Per-Bear canonical memory ([ADR-0031](../../../docs/decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)).

pub mod store;
pub mod tools;

pub use store::{
    BearMemoryStore, LogicalMemoryPath, MemoryRecordRow, MemoryScopeType, MemoryStoreManager,
};
