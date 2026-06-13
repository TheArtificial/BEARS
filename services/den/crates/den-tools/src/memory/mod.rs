//! Memory read tools (`memory_read`, `memory_browse`, `memory_search`,
//! `memory_status`) — orchestration layer.
//!
//! Runtime-agnostic: depends only on [`RoleMemoryStore`] (and, for the status
//! diagnostic, [`PromptMemoryStore`]) plus `den-core` types. The `den` crate
//! provides the concrete store (native SQLite + legacy MemFS branch) and thin
//! `CustomError`-mapping wrappers.

pub mod store;

pub use store::RoleMemoryStore;

use den_core::{BearProfile, DenError};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::prompt_memory::{PromptMemoryBlock, PromptMemoryBlockState, PromptMemoryStore};

#[derive(Debug, Deserialize)]
pub struct MemoryReadArguments {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct MemorySearchArguments {
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryWriteEntryArguments {
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub refs: Option<Value>,
    #[serde(default)]
    pub lifecycle: Option<Value>,
    #[serde(default)]
    pub source: Option<Value>,
    #[serde(default)]
    pub content_class: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub semantic_confirmation_token: Option<String>,
}

/// Summarize active prompt-memory blocks for the `memory_status` diagnostic.
pub fn prompt_memory_diagnostic_summary(blocks: &[PromptMemoryBlock]) -> Value {
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
        let scope_count = active_by_scope.entry(scope_key).or_insert_with(|| json!(0));
        *scope_count = json!(scope_count.as_i64().unwrap_or(0) + 1);

        let type_key = serde_json::to_string(&block.block_type)
            .unwrap_or_else(|_| "\"unknown\"".to_string())
            .trim_matches('"')
            .to_string();
        let type_count = active_by_type.entry(type_key).or_insert_with(|| json!(0));
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

pub async fn memory_status(
    memory: &impl RoleMemoryStore,
    prompt: &impl PromptMemoryStore,
    bear_id: Uuid,
    role: BearProfile,
) -> Result<Value, DenError> {
    let mut base = memory.status_base(bear_id, role).await?;
    let blocks = prompt.list_blocks(bear_id, role.as_str()).await?;
    let diagnostic = prompt_memory_diagnostic_summary(&blocks);
    if let Some(obj) = base.as_object_mut() {
        obj.insert("prompt_memory_diagnostic".to_string(), diagnostic);
        obj.insert("bear_id".to_string(), json!(bear_id));
    }
    Ok(base)
}

pub async fn memory_browse(
    memory: &impl RoleMemoryStore,
    bear_id: Uuid,
    role: BearProfile,
) -> Result<Value, DenError> {
    memory.browse(bear_id, role).await
}

pub async fn memory_read(
    memory: &impl RoleMemoryStore,
    bear_id: Uuid,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    let args: MemoryReadArguments = serde_json::from_value(arguments)?;
    let path = args.path.trim();
    if path.is_empty() {
        return Err(DenError::ValidationError("path must not be empty".to_string()));
    }
    memory.read(bear_id, role, path).await
}

pub async fn memory_search(
    memory: &impl RoleMemoryStore,
    bear_id: Uuid,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    let args: MemorySearchArguments = serde_json::from_value(arguments)?;
    let query = args.query.trim();
    if query.is_empty() {
        return Err(DenError::ValidationError("query must not be empty".to_string()));
    }
    let limit = args.limit.map(|n| n.clamp(1, 50) as i64).unwrap_or(10);
    memory.search(bear_id, role, query, limit).await
}
