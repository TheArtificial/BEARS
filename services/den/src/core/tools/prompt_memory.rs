use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{
    core::{
        bears::BearAgentRole,
        prompt_memory_block_store::{
            archive_conflicting_prompt_memory_blocks,
            archive_prompt_memory_blocks_superseded_by, list_prompt_memory_blocks_for_bear_role,
            patch_prompt_memory_block, upsert_prompt_memory_block, PromptMemoryBlockPatch,
            PromptMemoryBlockWrite,
        },
        prompt_memory_blocks::{
            PromptMemoryBlockScope, PromptMemoryBlockState, PromptMemoryBlockType,
        },
        tools::{session::DenToolInvocationContext, support::{validate_bounded_text, validate_optional_object, validate_prompt_memory_scope}},
    },
    errors::CustomError,
};

#[derive(Debug, Deserialize)]
pub(crate) struct PromptMemoryUpsertArguments {
    pub(crate) block_id: String,
    pub(crate) scope: PromptMemoryBlockScope,
    pub(crate) block_type: PromptMemoryBlockType,
    #[serde(default = "default_prompt_memory_state")]
    pub(crate) state: PromptMemoryBlockState,
    #[serde(default)]
    pub(crate) work_surface: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    pub(crate) title: String,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) priority: Option<i32>,
    #[serde(default)]
    pub(crate) supersedes_block_id: Option<String>,
    #[serde(default)]
    pub(crate) metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptMemoryPatchArguments {
    pub(crate) block_id: String,
    #[serde(default = "default_prompt_memory_state")]
    pub(crate) state: PromptMemoryBlockState,
    pub(crate) title: String,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) priority: Option<i32>,
    #[serde(default)]
    pub(crate) supersedes_block_id: Option<String>,
    #[serde(default)]
    pub(crate) metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptMemoryListArguments {
    #[serde(default)]
    pub(crate) include_archived: bool,
    #[serde(default)]
    pub(crate) scope: Option<PromptMemoryBlockScope>,
    #[serde(default)]
    pub(crate) block_type: Option<PromptMemoryBlockType>,
    #[serde(default)]
    pub(crate) work_surface: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
}

pub(crate) fn default_prompt_memory_state() -> PromptMemoryBlockState {
    PromptMemoryBlockState::Active
}

pub(crate) fn empty_json_object() -> Value {
    json!({})
}

pub(crate) async fn prompt_memory_upsert(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearAgentRole::Pair {
        return Err(CustomError::Authorization(
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
    let metadata = args.metadata.unwrap_or_else(empty_json_object);
    validate_optional_object("metadata", &Some(metadata.clone()))?;
    let write = PromptMemoryBlockWrite {
        block_id: block_id.clone(),
        bear_id: Some(context.bear_id),
        role_slug: Some(role.as_str().to_string()),
        scope: args.scope,
        block_type: args.block_type,
        state: args.state,
        work_surface: args.work_surface.clone(),
        session_id: args.session_id.clone(),
        title: title.clone(),
        body: body.clone(),
        priority,
        created_by_user_id: Some(context.user_id),
        supersedes_block_id: args.supersedes_block_id.clone(),
        metadata: metadata.clone(),
    };
    let conflicting_archived = if args.state == PromptMemoryBlockState::Active {
        archive_conflicting_prompt_memory_blocks(pool, &write).await?
    } else {
        0
    };
    upsert_prompt_memory_block(pool, &write).await?;
    let superseded_archived = if let Some(supersedes_block_id) = args.supersedes_block_id.as_deref() {
        archive_prompt_memory_blocks_superseded_by(
            pool,
            context.bear_id,
            role.as_str(),
            supersedes_block_id,
        )
        .await?
    } else {
        0
    };
    Ok(json!({
        "status": "ok",
        "block_id": block_id,
        "scope": args.scope,
        "block_type": args.block_type,
        "state": args.state,
        "title": title,
        "priority": priority,
        "work_surface": args.work_surface,
        "session_id": args.session_id,
        "metadata": metadata,
        "supersedes_block_id": args.supersedes_block_id,
        "superseded_archived_count": superseded_archived,
        "conflicting_archived_count": conflicting_archived,
        "source": "prompt_memory_blocks"
    }))
}

pub(crate) async fn prompt_memory_list(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearAgentRole::Pair {
        return Err(CustomError::Authorization(
            "den.prompt_memory.list is currently available only to the pair role".to_string(),
        ));
    }
    let args: PromptMemoryListArguments = serde_json::from_value(arguments)?;
    let mut blocks = list_prompt_memory_blocks_for_bear_role(pool, context.bear_id, role.as_str()).await?;
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

pub(crate) async fn prompt_memory_patch(
    pool: &PgPool,
    _context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearAgentRole::Pair {
        return Err(CustomError::Authorization(
            "den.prompt_memory.patch is currently available only to the pair role".to_string(),
        ));
    }
    let args: PromptMemoryPatchArguments = serde_json::from_value(arguments)?;
    let title = validate_bounded_text("title", &args.title, 1, 200)?;
    let body = validate_bounded_text("body", &args.body, 1, 50_000)?;
    let block_id = validate_bounded_text("block_id", &args.block_id, 1, 200)?;
    let priority = args.priority.unwrap_or(0).clamp(-1000, 1000);
    let metadata = args.metadata.unwrap_or_else(empty_json_object);
    validate_optional_object("metadata", &Some(metadata.clone()))?;
    patch_prompt_memory_block(
        pool,
        &block_id,
        &PromptMemoryBlockPatch {
            state: args.state,
            title: title.clone(),
            body: body.clone(),
            priority,
            supersedes_block_id: args.supersedes_block_id.clone(),
            metadata: metadata.clone(),
        },
    )
    .await?;
    Ok(json!({
        "status": "ok",
        "block_id": block_id,
        "state": args.state,
        "title": title,
        "priority": priority,
        "metadata": metadata,
        "source": "prompt_memory_blocks"
    }))
}
