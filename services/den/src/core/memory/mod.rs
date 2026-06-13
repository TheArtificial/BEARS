//! Per-Bear canonical memory ([ADR-0031](../../../docs/decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)).

pub mod admin_inspect;
pub mod bear_observations;
pub mod curate_executor;
pub mod curation;
pub mod manager_head;
pub mod prompt_block_store;
pub mod prompt_blocks;
pub mod proposals;
pub mod tools;

#[cfg(test)]
mod manager_head_append_markdown_tests;

/// Per-Bear SQLite store, extracted to the `den-memory` crate. Re-exported here
/// so existing `crate::core::memory::store::…` paths keep resolving.
pub use den_memory as store;

#[cfg(test)]
mod store_round_trip_tests;

pub use admin_inspect::{
    bear_memory_admin_stats, bear_sqlite_db_path, get_memory_record_by_id, list_all_logical_paths,
    list_recent_memory_records, BearMemoryAdminStats,
};
pub use curation::{
    create_observation, create_proposal, get_observation, get_proposal, list_proposals,
    mark_observation_review_queued_for_bear, promote_core_content, record_reflection_outcome_complete,
    record_reflection_outcome_start, resolve_proposal, uses_sqlite_curation,
};
pub use store::{
    has_work_surface_canonical_anchor, head_record_for_logical_path, list_profile_local_head_records,
    memory_sequence_high_water, BearMemoryStore, LogicalMemoryPath, MemoryRecordRow,
    MemoryScopeType, MemoryStoreManager,
};
