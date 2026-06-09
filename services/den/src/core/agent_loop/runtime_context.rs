use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    core::{
        prompt_memory_block_store::{
            select_prompt_memory_blocks_for_runtime, PromptMemoryBlockQuery,
            PromptMemoryRuntimeSelection,
        },
        prompt_memory_blocks::{
            compile_prompt_memory_blocks, render_prompt_memory_block_context,
            PromptMemoryCompilationInput,
        },
        runtime::compaction::{
            build_runtime_context_envelope, semantic_groups_from_runtime_messages,
            RuntimeCompactionPolicy, RuntimeContextEnvelopeInput,
        },
        runtime_compaction_observability::{
            build_compaction_skipped_event, RuntimeCompactionEventStatus,
        },
        runtime_conversations::RuntimeCompactionTriggerKind,
    },
    errors::CustomError,
};

pub fn runtime_context_already_includes_den_owned_blocks(runtime_context: &str) -> bool {
    let trimmed = runtime_context.trim();
    !trimmed.is_empty()
        && (trimmed.contains("Prompt memory blocks are Den-owned")
            || trimmed.contains("Runtime compaction context is Den-owned"))
}

pub fn native_runtime_compaction_prompt_context(
    session_id: &str,
    client_context: &Value,
) -> String {
    let messages = client_context
        .get("messages")
        .or_else(|| client_context.get("history"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let groups = semantic_groups_from_runtime_messages(&messages);
    let envelope = build_runtime_context_envelope(RuntimeContextEnvelopeInput {
        active_instructions: vec![format!("session:{session_id}")],
        workflow_state: Vec::new(),
        recent_groups: groups.iter().rev().take(3).cloned().collect::<Vec<_>>().into_iter().rev().collect(),
        compacted_summary: None,
    });
    let compacted = envelope.compacted_context.unwrap_or_default();
    let policy = RuntimeCompactionPolicy {
        policy_version: "native-agent-loop-v1".to_string(),
        protected_recent_group_count: 3,
        max_groups_before_compaction: 6,
    };
    let event = build_compaction_skipped_event(
        session_id.to_string(),
        RuntimeCompactionTriggerKind::SemanticGroupCount,
        &policy,
        "native agent loop context assembly",
    );
    let decision_status = match event.status {
        RuntimeCompactionEventStatus::Applied => "applied",
        RuntimeCompactionEventStatus::Skipped => "skipped",
        RuntimeCompactionEventStatus::Failed => "failed",
    };
    format!(
        "Runtime compaction context is Den-owned. Treat active instructions, workflow state, recent uncompacted groups, and compacted summary state as distinct context layers. Current compacted summary signals: goals={} constraints={} decisions={} artifacts={} workflow_refs={} followups={}. Current compaction evaluation: status={} policy_version={} source_range={:?}-{:?} diagnostic={}.",
        compacted.active_user_goals.len(),
        compacted.important_constraints.len(),
        compacted.decisions_made.len(),
        compacted.artifact_refs.len(),
        compacted.workflow_state_refs.len(),
        compacted.unresolved_followups.len(),
        decision_status,
        event.policy_version,
        event.source_group_start,
        event.source_group_end,
        event.diagnostic.as_deref().unwrap_or("none"),
    )
}

async fn load_prompt_memory_runtime_text(
    pool: &PgPool,
    bear_id: Uuid,
    role_slug: &str,
    session_id: &str,
    workspace_roots: &[String],
) -> Result<String, CustomError> {
    let selection = match select_prompt_memory_blocks_for_runtime(
        pool,
        PromptMemoryBlockQuery {
            bear_id: Some(bear_id),
            role_slug,
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
            role: role_slug,
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
    role_slug: &str,
    session_id: &str,
    workspace_roots: &[String],
    client_context: &Value,
) -> Result<String, CustomError> {
    let mut parts = Vec::new();
    let prompt_memory = load_prompt_memory_runtime_text(
        pool,
        bear_id,
        role_slug,
        session_id,
        workspace_roots,
    )
    .await?;
    if !prompt_memory.trim().is_empty() {
        parts.push(prompt_memory);
    }
    parts.push(native_runtime_compaction_prompt_context(session_id, client_context));
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

    #[test]
    fn native_runtime_compaction_prompt_context_is_non_empty() {
        let text = native_runtime_compaction_prompt_context("sess-1", &json!({}));
        assert!(text.contains("Runtime compaction context is Den-owned"));
    }
}
