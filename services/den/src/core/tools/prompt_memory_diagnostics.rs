use serde_json::{json, Value};

use crate::core::prompt_memory_blocks::{
    PromptMemoryBlock, PromptMemoryBlockScope, PromptMemoryBlockState, PromptMemoryBlockType,
};

pub(crate) fn prompt_memory_diagnostic_summary_for_bear_role(
    blocks: &[PromptMemoryBlock],
) -> Value {
    let mut active_by_scope = serde_json::Map::new();
    let mut active_by_type = serde_json::Map::new();
    for scope in [
        PromptMemoryBlockScope::BearWide,
        PromptMemoryBlockScope::RoleLocal,
        PromptMemoryBlockScope::WorkSurface,
        PromptMemoryBlockScope::Session,
    ] {
        let key = serde_json::to_string(&scope)
            .unwrap_or_else(|_| "unknown".to_string())
            .trim_matches('"')
            .to_string();
        let count = blocks
            .iter()
            .filter(|block| block.state == PromptMemoryBlockState::Active && block.scope == scope)
            .count();
        active_by_scope.insert(key, json!(count));
    }
    for block_type in [
        PromptMemoryBlockType::RoleGuidance,
        PromptMemoryBlockType::WorkSurfaceContext,
        PromptMemoryBlockType::SessionFocus,
        PromptMemoryBlockType::UserInstruction,
    ] {
        let key = serde_json::to_string(&block_type)
            .unwrap_or_else(|_| "unknown".to_string())
            .trim_matches('"')
            .to_string();
        let count = blocks
            .iter()
            .filter(|block| {
                block.state == PromptMemoryBlockState::Active && block.block_type == block_type
            })
            .count();
        active_by_type.insert(key, json!(count));
    }
    let active_blocks = blocks
        .iter()
        .filter(|block| block.state == PromptMemoryBlockState::Active)
        .map(|block| {
            json!({
                "id": block.id,
                "scope": block.scope,
                "block_type": block.block_type,
                "title": block.title,
                "priority": block.priority,
                "work_surface": block.work_surface,
                "session_id": block.session_id,
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
