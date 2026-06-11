//! Per-Bear canonical memory ([ADR-0031](../../../docs/decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)).

pub mod admin_inspect;
pub mod governance;
pub mod store;
pub mod tools;

pub use admin_inspect::{
    bear_memory_admin_stats, bear_sqlite_db_path, get_memory_record_by_id, list_all_logical_paths,
    list_recent_memory_records, BearMemoryAdminStats,
};
pub use governance::{
    create_observation, create_proposal, get_observation, get_proposal, list_proposals,
    mark_observation_review_queued_for_bear, promote_core_content, record_reflection_outcome_complete,
    record_reflection_outcome_start, resolve_proposal, uses_sqlite_governance,
};
pub use store::{
    has_work_surface_canonical_anchor, head_record_for_logical_path, list_profile_local_head_records,
    memory_sequence_high_water, BearMemoryStore, LogicalMemoryPath, MemoryRecordRow,
    MemoryScopeType, MemoryStoreManager,
};
