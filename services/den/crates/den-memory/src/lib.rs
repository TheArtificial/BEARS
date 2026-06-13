//! `den-memory`: per-Bear canonical memory store (per-Bear SQLite), per ADR-0031.
//!
//! Service-layer leaf crate (see `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`):
//! depends only on `den-core`. Higher-level curation/tools live in `den`.

mod links;
mod logical_path;
mod manager;
mod migrate;
mod observations;
mod promotions;
mod proposals;
mod records;
pub mod reflection_outcomes;

pub use links::{
    append_memory_link, list_memory_links_for_bear, list_memory_links_for_source, MemoryLinkRow,
};
pub use logical_path::{LogicalMemoryPath, MemoryScopeType};
pub use manager::MemoryStoreManager;
pub use observations::{
    create_memory_observation, get_memory_observation, mark_observation_review_queued,
    SqliteMemoryObservation,
};
pub use promotions::{append_memory_promotion, promote_to_shared_core};
pub use proposals::{create_memory_proposal, list_memory_proposals, resolve_memory_proposal, SqliteMemoryProposal};
pub use records::{
    append_memory_record, has_work_surface_canonical_anchor, head_record_for_logical_path,
    list_profile_local_head_records, list_records_for_logical_path, memory_sequence_high_water,
    MemoryRecordRow, BearMemoryStore,
};
pub use reflection_outcomes::{
    complete_reflection_run_outcome, create_reflection_run_outcome, reflection_outcome_exists,
};
