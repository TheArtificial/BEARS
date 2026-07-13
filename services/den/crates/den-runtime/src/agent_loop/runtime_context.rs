use den_core::DenError;
use den_docket::TaskListProjection;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use den_service::prompt_memory_block_store::{
    select_prompt_memory_blocks_for_runtime, PromptMemoryBlockQuery, PromptMemoryRuntimeSelection,
};
use den_service::prompt_memory_blocks::{
    compile_prompt_memory_blocks, render_prompt_memory_block_context, PromptMemoryCompilationInput,
};

pub fn runtime_context_already_includes_den_owned_blocks(runtime_context: &str) -> bool {
    let trimmed = runtime_context.trim();
    !trimmed.is_empty()
        && (trimmed.contains("Prompt memory blocks are Den-owned")
            || trimmed.contains("Runtime compaction context is Den-owned")
            || trimmed.contains("Den objective orientation is Den-owned"))
}

fn active_task_for_orientation(plan: &TaskListProjection) -> Option<&den_docket::TaskListItem> {
    plan.current_item.as_ref().or_else(|| {
        plan.items.iter().find(|item| {
            matches!(
                item.status,
                den_docket::TaskListItemStatus::InProgress
                    | den_docket::TaskListItemStatus::Pending
            )
        })
    })
}

fn render_objective_orientation_context(plan: Option<&TaskListProjection>) -> String {
    let Some(plan) = plan else {
        // ponytail: freeform task definition is closed until a caller supplies a policy surface;
        // upgrade path is to thread FreeformPolicy into this runtime-context compilation path.
        return "<system-reminder>\nDen objective orientation is Den-owned runtime context. orientation=freeform may_define_task=false. No concrete task or Job outcome is active. Keep the turn bounded: answer directly, ask a clarifying question, or stop.\n</system-reminder>".to_string();
    };

    let task = active_task_for_orientation(plan)
        .map(|item| item.title.as_str())
        .unwrap_or("the next actionable task");
    if let Some(job_id) = plan.source_ref.docket_job_id.as_deref() {
        return format!(
            "<system-reminder>\nDen objective orientation is Den-owned runtime context. orientation=focused job_id={job_id}. Your top priority is to complete the Docket Job. Continue through the active task: {task}. Child tasks may be added when useful unless the Job is immutable.\n</system-reminder>"
        );
    }

    format!(
        "<system-reminder>\nDen objective orientation is Den-owned runtime context. orientation=oriented. A concrete task is active: {task}. Complete that task before final-answering. If you decompose it, stay within 6 child tasks and 1 level below the oriented task.\n</system-reminder>"
    )
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
    active_activity_plan: Option<&TaskListProjection>,
) -> Result<String, DenError> {
    let mut parts = Vec::new();
    parts.push(render_objective_orientation_context(active_activity_plan));
    let prompt_memory =
        load_prompt_memory_runtime_text(pool, bear_id, profile_slug, session_id, workspace_roots)
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
        assert!(!runtime_context_already_includes_den_owned_blocks(
            "plain runtime notes"
        ));
    }
}
