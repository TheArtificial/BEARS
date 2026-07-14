use den_core::DenError;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use den_service::prompt_memory_block_store::{
    select_prompt_memory_blocks_for_runtime, PromptMemoryBlockQuery, PromptMemoryRuntimeSelection,
};
use den_service::prompt_memory_blocks::{
    compile_prompt_memory_blocks, render_prompt_memory_block_context, PromptMemoryCompilationInput,
};

use crate::agent_loop::ObjectiveOrientation;

pub fn runtime_context_already_includes_den_owned_blocks(runtime_context: &str) -> bool {
    let trimmed = runtime_context.trim();
    !trimmed.is_empty()
        && (trimmed.contains("Prompt memory blocks are Den-owned")
            || trimmed.contains("Runtime compaction context is Den-owned")
            || trimmed.contains("Den objective orientation is Den-owned"))
}

fn render_objective_orientation_context(orientation: &ObjectiveOrientation) -> String {
    match orientation {
        ObjectiveOrientation::Freeform { policy } => {
            let task_definition_guidance = if policy.may_define_task {
                " If the request needs sustained work, define a concrete task with completion criteria; the runtime may then continue task-oriented or delegate through available execution policy."
            } else {
                ""
            };
            format!(
                "<system-reminder>\nDen objective orientation is Den-owned runtime context. orientation=freeform may_define_task={}. No concrete task or Job outcome is active. Keep the turn bounded: answer directly, ask a clarifying question, or stop.{}\n</system-reminder>",
                policy.may_define_task,
                task_definition_guidance
            )
        }
        ObjectiveOrientation::Oriented { task } => {
            let task_ref = serde_json::to_string(&task.task_ref)
                .unwrap_or_else(|_| "{\"kind\":\"unknown\"}".to_string());
            format!(
                "<system-reminder>\nDen objective orientation is Den-owned runtime context. orientation=oriented task_ref={task_ref}. A concrete task is active. Keep working toward its completion criteria. Do not claim completion until they are met. Ask only necessary clarifying questions; otherwise proceed within the task boundary. If you decompose it, stay within {} child tasks and {} level below the oriented task.\n</system-reminder>",
                task.child_policy.max_children,
                task.child_policy.max_depth_below_oriented_task
            )
        }
        ObjectiveOrientation::Focused { job } => {
            let active_task_ref = job
                .active_task_ref
                .as_ref()
                .and_then(|task| serde_json::to_string(task).ok())
                .unwrap_or_else(|| "null".to_string());
            format!(
                "<system-reminder>\nDen objective orientation is Den-owned runtime context. orientation=focused job_id={} job_mutable={} active_task_ref={active_task_ref}. Your top priority is to complete the Docket Job. Continue through the active task. Child tasks may be added when useful unless the Job is immutable.\n</system-reminder>",
                job.job_id,
                job.mutable
            )
        }
    }
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
    objective_orientation: &ObjectiveOrientation,
) -> Result<String, DenError> {
    let mut parts = Vec::new();
    parts.push(render_objective_orientation_context(objective_orientation));
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
