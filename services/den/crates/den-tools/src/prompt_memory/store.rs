//! The `PromptMemoryStore` capability seam.
//!
//! `den-tools` owns the prompt-memory tool orchestration (role gating, argument
//! validation, write/patch construction, result shaping); the Postgres-backed
//! persistence is inverted behind this trait, implemented by `den-runtime` over
//! `prompt_memory_block_store`. See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`.

use async_trait::async_trait;
use den_core::DenError;
use uuid::Uuid;

use super::types::{PromptMemoryBlock, PromptMemoryBlockPatch, PromptMemoryBlockWrite};

#[async_trait]
pub trait PromptMemoryStore: Send + Sync {
    /// All blocks for a bear/profile (newest first), unfiltered.
    async fn list_blocks(
        &self,
        bear_id: Uuid,
        profile_slug: &str,
    ) -> Result<Vec<PromptMemoryBlock>, DenError>;

    async fn upsert_block(&self, write: &PromptMemoryBlockWrite) -> Result<(), DenError>;

    async fn patch_block(
        &self,
        block_id: &str,
        patch: &PromptMemoryBlockPatch,
    ) -> Result<(), DenError>;

    /// Archive other active blocks that conflict with `write`; returns the count.
    async fn archive_conflicting(&self, write: &PromptMemoryBlockWrite) -> Result<u64, DenError>;

    /// Archive the block(s) the given `supersedes_block_id` replaces; returns the count.
    async fn archive_superseded_by(
        &self,
        bear_id: Uuid,
        profile_slug: &str,
        supersedes_block_id: &str,
    ) -> Result<u64, DenError>;
}
