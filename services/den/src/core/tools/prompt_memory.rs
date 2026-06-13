//! `den`-side wiring for the prompt-memory tools.
//!
//! The orchestration (role gating, validation, result shaping) lives in
//! `den-tools`; here we provide the Postgres-backed [`PromptMemoryStore`] and
//! thin wrappers that adapt `DenToolInvocationContext` primitives and map
//! `DenError` back to the web-boundary `CustomError`.

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use den_tools::prompt_memory::{
    PromptMemoryBlock, PromptMemoryBlockPatch, PromptMemoryBlockWrite, PromptMemoryStore,
};

use crate::{
    core::{
        bears::BearProfile,
        prompt_memory_block_store::{
            archive_conflicting_prompt_memory_blocks,
            archive_prompt_memory_blocks_superseded_by, list_prompt_memory_blocks_for_bear_profile,
            patch_prompt_memory_block, upsert_prompt_memory_block,
        },
        tools::session::DenToolInvocationContext,
    },
    errors::{CustomError, DenError},
};

/// Postgres-backed [`PromptMemoryStore`] over a pool reference.
struct DenPromptMemoryStore<'a> {
    pool: &'a PgPool,
}

#[async_trait]
impl PromptMemoryStore for DenPromptMemoryStore<'_> {
    async fn list_blocks(
        &self,
        bear_id: Uuid,
        profile_slug: &str,
    ) -> Result<Vec<PromptMemoryBlock>, DenError> {
        list_prompt_memory_blocks_for_bear_profile(self.pool, bear_id, profile_slug)
            .await
            .map_err(CustomError::into_den)
    }

    async fn upsert_block(&self, write: &PromptMemoryBlockWrite) -> Result<(), DenError> {
        upsert_prompt_memory_block(self.pool, write)
            .await
            .map_err(CustomError::into_den)
    }

    async fn patch_block(
        &self,
        block_id: &str,
        patch: &PromptMemoryBlockPatch,
    ) -> Result<(), DenError> {
        patch_prompt_memory_block(self.pool, block_id, patch)
            .await
            .map_err(CustomError::into_den)
    }

    async fn archive_conflicting(&self, write: &PromptMemoryBlockWrite) -> Result<u64, DenError> {
        archive_conflicting_prompt_memory_blocks(self.pool, write)
            .await
            .map_err(CustomError::into_den)
    }

    async fn archive_superseded_by(
        &self,
        bear_id: Uuid,
        profile_slug: &str,
        supersedes_block_id: &str,
    ) -> Result<u64, DenError> {
        archive_prompt_memory_blocks_superseded_by(
            self.pool,
            bear_id,
            profile_slug,
            supersedes_block_id,
        )
        .await
        .map_err(CustomError::into_den)
    }
}

pub(crate) async fn prompt_memory_upsert(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let store = DenPromptMemoryStore { pool };
    den_tools::prompt_memory::prompt_memory_upsert(
        &store,
        context.bear_id,
        context.user_id,
        role,
        arguments,
    )
    .await
    .map_err(CustomError::from)
}

pub(crate) async fn prompt_memory_list(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let store = DenPromptMemoryStore { pool };
    den_tools::prompt_memory::prompt_memory_list(&store, context.bear_id, role, arguments)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn prompt_memory_patch(
    pool: &PgPool,
    _context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let store = DenPromptMemoryStore { pool };
    den_tools::prompt_memory::prompt_memory_patch(&store, role, arguments)
        .await
        .map_err(CustomError::from)
}
