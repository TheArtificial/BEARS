//! `den-memory`: per-Bear canonical memory store (per-Bear SQLite), per ADR-0031.
//!
//! Service-layer leaf crate (see `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`):
//! depends only on `den-core`. Higher-level curation/tools live in `den`.

pub mod access;
pub mod descriptors;
pub mod entity;
pub mod import;
mod logical_path;
mod manager;
mod migrate;
mod observations;
mod promotions;
mod proposals;
mod records;
pub mod reflection_outcomes;
pub mod relations;
pub mod resolver;

#[cfg(test)]
mod test_support;

pub use access::{record_visible, AccessContext};
pub use descriptors::{EntityTrust, HandleStrength, RecallEffect, RelationClass, ResolutionState};
pub use entity::{
    attach_handle, create_entity, detach_handle, find_entity_by_handle, get_entity, list_entities,
    list_handles, merge_entities, resolve_live_entity, set_canonical_ref, set_resolution,
    split_entity, EntityHandleRow, EntityRow,
};
pub use import::{import_memfs_bundle, MemfsBranchReport, MemfsImportOptions, MemfsImportReport};
pub use logical_path::{LogicalMemoryPath, MemoryScopeType};
pub use manager::MemoryStoreManager;
pub use observations::{
    create_memory_observation, get_memory_observation, mark_observation_review_queued,
    SqliteMemoryObservation,
};
pub use promotions::{append_memory_promotion, promote_to_shared_core};
pub use proposals::{
    create_memory_proposal, list_memory_proposals, resolve_memory_proposal, SqliteMemoryProposal,
};
pub use records::{
    append_memory_record, effective_time_by_ids, fetch_records_min,
    has_work_surface_canonical_anchor, head_record_as_of, head_record_for_logical_path,
    list_profile_local_head_records, list_records_for_logical_path, memory_sequence_high_water,
    superseder_times_by_superseded, BearMemoryStore, MemoryRecordRow, RecallRecordMin,
};
pub use reflection_outcomes::{
    complete_reflection_run_outcome, create_reflection_run_outcome, reflection_outcome_exists,
};
pub use relations::{
    append_relation, bounded_graph_expand, descriptive_entity_ids_for_records,
    list_access_rules_for_source, list_relations_for_entity, list_relations_for_source,
    record_ids_for_entities, GraphReach, RelationRow,
};
pub use resolver::{resolve, resolve_work_surface, Assertion, Resolution, Signal};
