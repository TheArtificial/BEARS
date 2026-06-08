use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{
    config::Config,
    core::{
        bears::BearAgentRole,
        memory::{tools as sqlite_memory, MemoryStoreManager},
        prompt_memory_block_store::list_prompt_memory_blocks_for_bear_role,
        prompt_memory_blocks::{PromptMemoryBlock, PromptMemoryBlockState},
        tools::{
            memfs::{
                fetch_role_memory_file, fetch_role_memory_status, fetch_role_memory_tree,
                memfs_http_client, search_role_memory,
            },
            session::DenToolInvocationContext,
        },
    },
    errors::CustomError,
};

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryReadArguments {
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MemorySearchArguments {
    pub(crate) query: String,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

fn prompt_memory_diagnostic_summary_for_bear_role(blocks: &[PromptMemoryBlock]) -> Value {
    let active_blocks = blocks
        .iter()
        .filter(|block| block.state == PromptMemoryBlockState::Active)
        .collect::<Vec<_>>();
    let mut active_by_scope = serde_json::Map::new();
    let mut active_by_type = serde_json::Map::new();
    for block in &active_blocks {
        let scope_key = serde_json::to_string(&block.scope)
            .unwrap_or_else(|_| "\"unknown\"".to_string())
            .trim_matches('"')
            .to_string();
        let scope_count = active_by_scope
            .entry(scope_key)
            .or_insert_with(|| json!(0));
        *scope_count = json!(scope_count.as_i64().unwrap_or(0) + 1);

        let type_key = serde_json::to_string(&block.block_type)
            .unwrap_or_else(|_| "\"unknown\"".to_string())
            .trim_matches('"')
            .to_string();
        let type_count = active_by_type
            .entry(type_key)
            .or_insert_with(|| json!(0));
        *type_count = json!(type_count.as_i64().unwrap_or(0) + 1);
    }
    let active_blocks = active_blocks
        .into_iter()
        .map(|block| {
            json!({
                "block_id": block.id,
                "scope": block.scope,
                "block_type": block.block_type,
                "title": block.title,
                "work_surface": block.work_surface,
                "session_id": block.session_id,
                "priority": block.priority,
                "updated_at": Value::Null,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "status": if active_blocks.is_empty() { "empty" } else { "ok" },
        "source": "prompt_memory_blocks",
        "active_count": active_blocks.len(),
        "active_by_scope": active_by_scope,
        "active_by_type": active_by_type,
        "active_blocks": active_blocks,
    })
}

pub(crate) async fn memory_status(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
) -> Result<Value, CustomError> {
    if config.uses_native_agent_runtime() {
        let stores = MemoryStoreManager::new(config);
        let store = stores.store_for_bear(context.bear_id).await?;
        let mut response = sqlite_memory::sqlite_memory_status(&store, role.as_str()).await?;
        let prompt_memory_blocks =
            list_prompt_memory_blocks_for_bear_role(pool, context.bear_id, role.as_str()).await?;
        if let Some(obj) = response.as_object_mut() {
            obj.insert(
                "prompt_memory_diagnostic".to_string(),
                prompt_memory_diagnostic_summary_for_bear_role(&prompt_memory_blocks),
            );
            obj.insert("bear_id".to_string(), json!(context.bear_id));
        }
        return Ok(response);
    }
    let http = memfs_http_client("MemFS memory status client build failed")?;
    let response = fetch_role_memory_status(
        &http,
        &config.letta_memfs_service_url,
        context.bear_id,
        role.as_str(),
    )
    .await?;
    let prompt_memory_blocks =
        list_prompt_memory_blocks_for_bear_role(pool, context.bear_id, role.as_str()).await?;
    let prompt_memory_diagnostic =
        prompt_memory_diagnostic_summary_for_bear_role(&prompt_memory_blocks);
    let Some(response) = response else {
        return Ok(json!({
            "configured": false,
            "available": false,
            "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)",
            "prompt_memory_diagnostic": prompt_memory_diagnostic,
        }));
    };
    Ok(json!({
        "configured": true,
        "available": response.ok,
        "bear_id": context.bear_id,
        "role": role.as_str(),
        "canonical_tip": response.canonical_tip,
        "allowed_prefixes": response.allowed_prefixes,
        "file_count": response.file_count,
        "entry_count_by_kind": response.entry_count_by_kind,
        "registered_view_count": response.registered_view_count,
        "prompt_memory_diagnostic": prompt_memory_diagnostic,
    }))
}

pub(crate) async fn memory_status_value(
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    pool: &PgPool,
) -> Result<Value, CustomError> {
    memory_status(pool, config, context, role).await
}

pub(crate) async fn memory_browse(
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
) -> Result<Value, CustomError> {
    if config.uses_native_agent_runtime() {
        let stores = MemoryStoreManager::new(config);
        let store = stores.store_for_bear(context.bear_id).await?;
        return sqlite_memory::sqlite_memory_browse(&store, role.as_str()).await;
    }
    let http = memfs_http_client("MemFS memory browse client build failed")?;
    let response = fetch_role_memory_tree(
        &http,
        &config.letta_memfs_service_url,
        context.bear_id,
        role.as_str(),
    )
    .await?;
    response
        .map(|value| {
            serde_json::to_value(value)
                .map_err(|e| CustomError::Parsing(format!("memory browse JSON: {e}")))
        })
        .unwrap_or_else(|| {
            Ok(json!({
                "ok": false,
                "configured": false,
                "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)"
            }))
        })
}

pub(crate) async fn memory_read(
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: MemoryReadArguments = serde_json::from_value(arguments)?;
    let path = args.path.trim();
    if path.is_empty() {
        return Err(CustomError::ValidationError(
            "path must not be empty".to_string(),
        ));
    }
    if config.uses_native_agent_runtime() {
        let stores = MemoryStoreManager::new(config);
        let store = stores.store_for_bear(context.bear_id).await?;
        return sqlite_memory::sqlite_memory_read(&store, path).await;
    }
    let http = memfs_http_client("MemFS memory read client build failed")?;
    let response = fetch_role_memory_file(
        &http,
        &config.letta_memfs_service_url,
        context.bear_id,
        role.as_str(),
        path,
    )
    .await?;
    response
        .map(|value| {
            serde_json::to_value(value)
                .map_err(|e| CustomError::Parsing(format!("memory file JSON: {e}")))
        })
        .unwrap_or_else(|| {
            Ok(json!({
                "ok": false,
                "configured": false,
                "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)"
            }))
        })
}

pub(crate) async fn memory_search(
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: MemorySearchArguments = serde_json::from_value(arguments)?;
    let query = args.query.trim();
    if query.is_empty() {
        return Err(CustomError::ValidationError(
            "query must not be empty".to_string(),
        ));
    }
    if config.uses_native_agent_runtime() {
        let stores = MemoryStoreManager::new(config);
        let store = stores.store_for_bear(context.bear_id).await?;
        let limit = args.limit.map(|n| n.clamp(1, 50) as i64).unwrap_or(10);
        return sqlite_memory::sqlite_memory_search(&store, role.as_str(), query, limit).await;
    }
    let http = memfs_http_client("MemFS memory search client build failed")?;
    let response = search_role_memory(
        &http,
        &config.letta_memfs_service_url,
        context.bear_id,
        role.as_str(),
        query,
        args.limit.map(|n| n.clamp(1, 50)),
    )
    .await?;
    response
        .map(|value| {
            serde_json::to_value(value)
                .map_err(|e| CustomError::Parsing(format!("memory search JSON: {e}")))
        })
        .unwrap_or_else(|| {
            Ok(json!({
                "ok": false,
                "configured": false,
                "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)"
            }))
        })
}
