//! Per-Bear canonical memory ([ADR-0031](../../../docs/decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)).

pub mod governance;
pub mod store;
pub mod tools;

pub use governance::{
    create_observation, create_proposal, get_observation, get_proposal, list_proposals,
    mark_observation_review_queued_for_bear, promote_core_content, record_reflection_outcome_complete,
    record_reflection_outcome_start, resolve_proposal, uses_sqlite_governance,
};
pub use store::{
    BearMemoryStore, LogicalMemoryPath, MemoryRecordRow, MemoryScopeType, MemoryStoreManager,
};
