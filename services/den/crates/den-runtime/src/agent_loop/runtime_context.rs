use den_core::DenError;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_service::prompt_memory_block_store::{
    select_prompt_memory_blocks_for_runtime, PromptMemoryBlockQuery,
    PromptMemoryRuntimeSelection,
};
use den_service::prompt_memory_blocks::{
    compile_prompt_memory_blocks, render_prompt_memory_block_context, PromptMemoryCompilationInput,
};

use crate::runtime::compaction::TurnCompactionState;

pub fn runtime_context_already_includes_den_owned_blocks(runtime_context: &str) -> bool {
    let trimmed = runtime_context.trim();
    !trimmed.is_empty()
        && (trimmed.contains("Prompt memory blocks are Den-owned")
            || trimmed.contains("Runtime compaction context is Den-owned"))
}

async fn load_prompt_memory_runtime_text(
    pool: &PgPool,
    bear_id: Uuid,
    profile_slug: &str,
    session_id: &str,
    workspace_roots: &[String],
) -> Result<String, DenError> {
    let selection = match select_prompt_memory_blocks_for_runtime(
        pool,
        PromptMemoryBlockQuery {
            bear_id: Some(bear_id),
            profile_slug,
            session_id,
            work_surfaces: workspace_roots,
        },
    )
    .await
    {
        Ok(selection) => selection,
        Err(err) => PromptMemoryRuntimeSelection {
            diagnostic: json!({
                "source": "prompt_memory_blocks",
                "persisted": true,
                "status": "selection_error",
                "session_id": session_id,
                "work_surfaces": workspace_roots,
                "error": err.to_string(),
                "matched_count": 0,
            }),
            blocks: Vec::new(),
        },
    };
    let compilation = compile_prompt_memory_blocks(
        &selection.blocks,
        PromptMemoryCompilationInput {
            role: profile_slug,
            work_surfaces: workspace_roots,
            session_id,
            max_blocks: 6,
        },
    );
    Ok(render_prompt_memory_block_context(&compilation))
}

pub async fn assemble_den_owned_runtime_supplement(
    pool: &PgPool,
    bear_id: Uuid,
    profile_slug: &str,
    session_id: &str,
    workspace_roots: &[String],
    _client_context: &Value,
    _compaction_state: Option<&TurnCompactionState>,
) -> Result<String, DenError> {
    let mut parts = Vec::new();
    let prompt_memory = load_prompt_memory_runtime_text(
        pool,
        bear_id,
        profile_slug,
        session_id,
        workspace_roots,
    )
    .await?;
    if !prompt_memory.trim().is_empty() {
        parts.push(prompt_memory);
    }
    Ok(parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_context_already_includes_den_owned_blocks_detects_compaction() {
        assert!(runtime_context_already_includes_den_owned_blocks(
            "Runtime compaction context is Den-owned."
        ));
        assert!(!runtime_context_already_includes_den_owned_blocks("plain runtime notes"));
    }
}
