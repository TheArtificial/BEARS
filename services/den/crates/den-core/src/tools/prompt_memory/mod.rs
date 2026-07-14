//! Prompt-memory tools (`upsert`, `list`, `patch`) — orchestration layer.
//!
//! Runtime-agnostic: depends only on the [`PromptMemoryStore`] capability seam,
//! the shared validators, and `den-core::BearProfile`. The `den` crate provides
//! the concrete store and thin `CustomError`-mapping wrappers.

pub mod store;
pub mod types;

pub use store::PromptMemoryStore;
pub use types::{
    PromptMemoryBlock, PromptMemoryBlockPatch, PromptMemoryBlockScope, PromptMemoryBlockState,
    PromptMemoryBlockType, PromptMemoryBlockWrite,
};

use crate::{BearProfile, DenError};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::tools::validation::{validate_bounded_text, validate_optional_object};

/// Validate that a prompt-memory scope carries the qualifier it requires.
pub fn validate_prompt_memory_scope(
    scope: PromptMemoryBlockScope,
    work_surface: Option<&str>,
    session_id: Option<&str>,
) -> Result<(), DenError> {
    match scope {
        PromptMemoryBlockScope::WorkSurface if work_surface.is_none() => {
            Err(DenError::ValidationError(
                "prompt memory scope `work_surface` requires `work_surface`".to_string(),
            ))
        }
        PromptMemoryBlockScope::Session if session_id.is_none() => Err(DenError::ValidationError(
            "prompt memory scope `session` requires `session_id`".to_string(),
        )),
        _ => Ok(()),
    }
}

#[derive(Debug, Deserialize)]
pub struct PromptMemoryUpsertArguments {
    pub block_id: String,
    pub scope: PromptMemoryBlockScope,
    pub block_type: PromptMemoryBlockType,
    #[serde(default = "default_prompt_memory_state")]
    pub state: PromptMemoryBlockState,
    #[serde(default)]
    pub work_surface: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub supersedes_block_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct PromptMemoryPatchArguments {
    pub block_id: String,
    #[serde(default = "default_prompt_memory_state")]
    pub state: PromptMemoryBlockState,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub supersedes_block_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct PromptMemoryListArguments {
    #[serde(default)]
    pub include_archived: bool,
    #[serde(default)]
    pub scope: Option<PromptMemoryBlockScope>,
    #[serde(default)]
    pub block_type: Option<PromptMemoryBlockType>,
    #[serde(default)]
    pub work_surface: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

pub fn default_prompt_memory_state() -> PromptMemoryBlockState {
    PromptMemoryBlockState::Active
}

fn empty_json_object() -> Value {
    json!({})
}

pub async fn prompt_memory_upsert(
    store: &impl PromptMemoryStore,
    bear_id: Uuid,
    user_id: i32,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if role != BearProfile::Pair {
        return Err(DenError::Authorization(
            "den.prompt_memory.upsert is currently available only to the pair role".to_string(),
        ));
    }
    let args: PromptMemoryUpsertArguments = serde_json::from_value(arguments)?;
    let title = validate_bounded_text("title", &args.title, 1, 200)?;
    let body = validate_bounded_text("body", &args.body, 1, 50_000)?;
    let block_id = validate_bounded_text("block_id", &args.block_id, 1, 200)?;
    validate_prompt_memory_scope(
        args.scope,
        args.work_surface.as_deref(),
        args.session_id.as_deref(),
    )?;
    let priority = args.priority.unwrap_or(0).clamp(-1000, 1000);
    validate_optional_object("metadata", &args.metadata)?;
    let write = PromptMemoryBlockWrite {
        block_id,
        bear_id: Some(bear_id),
        profile_slug: Some(role.as_str().to_string()),
        scope: args.scope,
        block_type: args.block_type,
        state: args.state,
        work_surface: args.work_surface,
        session_id: args.session_id,
        title,
        body,
        priority,
        created_by_user_id: Some(user_id),
        supersedes_block_id: args.supersedes_block_id,
        metadata: args.metadata.unwrap_or_else(empty_json_object),
    };
    let conflicting_archived = if write.state == PromptMemoryBlockState::Active {
        store.archive_conflicting(&write).await?
    } else {
        0
    };
    store.upsert_block(&write).await?;
    let superseded_archived =
        if let Some(supersedes_block_id) = write.supersedes_block_id.as_deref() {
            store
                .archive_superseded_by(bear_id, role.as_str(), supersedes_block_id)
                .await?
        } else {
            0
        };
    Ok(json!({
        "status": "ok",
        "block_id": write.block_id,
        "state": write.state,
        "title": write.title,
        "priority": write.priority,
        "metadata": write.metadata,
        "conflicting_archived": conflicting_archived,
        "superseded_archived": superseded_archived,
        "source": "prompt_memory_blocks"
    }))
}

pub async fn prompt_memory_list(
    store: &impl PromptMemoryStore,
    bear_id: Uuid,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if role != BearProfile::Pair {
        return Err(DenError::Authorization(
            "den.prompt_memory.list is currently available only to the pair role".to_string(),
        ));
    }
    let args: PromptMemoryListArguments = serde_json::from_value(arguments)?;
    let mut blocks = store.list_blocks(bear_id, role.as_str()).await?;
    if !args.include_archived {
        blocks.retain(|block| block.state != PromptMemoryBlockState::Archived);
    }
    if let Some(scope) = args.scope {
        blocks.retain(|block| block.scope == scope);
    }
    if let Some(block_type) = args.block_type {
        blocks.retain(|block| block.block_type == block_type);
    }
    if let Some(work_surface) = args.work_surface.as_deref() {
        let normalized = work_surface.trim();
        blocks.retain(|block| block.work_surface.as_deref() == Some(normalized));
    }
    if let Some(session_id) = args.session_id.as_deref() {
        let normalized = session_id.trim();
        blocks.retain(|block| block.session_id.as_deref() == Some(normalized));
    }
    Ok(json!({
        "status": "ok",
        "source": "prompt_memory_blocks",
        "count": blocks.len(),
        "filters": {
            "include_archived": args.include_archived,
            "scope": args.scope,
            "block_type": args.block_type,
            "work_surface": args.work_surface,
            "session_id": args.session_id,
        },
        "blocks": blocks,
    }))
}

pub async fn prompt_memory_patch(
    store: &impl PromptMemoryStore,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if role != BearProfile::Pair {
        return Err(DenError::Authorization(
            "den.prompt_memory.patch is currently available only to the pair role".to_string(),
        ));
    }
    let args: PromptMemoryPatchArguments = serde_json::from_value(arguments)?;
    let title = validate_bounded_text("title", &args.title, 1, 200)?;
    let body = validate_bounded_text("body", &args.body, 1, 50_000)?;
    let block_id = validate_bounded_text("block_id", &args.block_id, 1, 200)?;
    let priority = args.priority.unwrap_or(0).clamp(-1000, 1000);
    validate_optional_object("metadata", &args.metadata)?;
    let patch = PromptMemoryBlockPatch {
        state: args.state,
        title,
        body,
        priority,
        supersedes_block_id: args.supersedes_block_id,
        metadata: args.metadata.unwrap_or_else(empty_json_object),
    };
    store.patch_block(&block_id, &patch).await?;
    Ok(json!({
        "status": "ok",
        "block_id": block_id,
        "state": patch.state,
        "title": patch.title,
        "priority": patch.priority,
        "metadata": patch.metadata,
        "source": "prompt_memory_blocks"
    }))
}
