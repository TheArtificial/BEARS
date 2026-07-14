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

const OBJECTIVE_ORIENTATION_PREAMBLE: &str =
    "Den objective orientation is Den-owned runtime context.";
const FREEFORM_BASE_GUIDANCE: &str =
    "No concrete task or Job outcome is active. Keep the turn bounded: answer directly, ask a clarifying question, or stop.";
const FREEFORM_TASK_DEFINITION_GUIDANCE: &str =
    "If the request needs sustained work, define a concrete task with completion criteria; the runtime may then continue task-oriented or delegate through available execution policy.";
const FREEFORM_CLOSED_TASK_POLICY_GUIDANCE: &str =
    "Task-definition tools are unavailable in closed freeform orientation; answer directly or ask before defining durable work.";
const PAIR_FREEFORM_TASK_ORIENTATION_HINT: &str =
    "Pair task-orientation hint: For work-like requests, proactively define concrete task(s) with completion criteria and move toward oriented work. If the user points you at a plan, roadmap, issue list, or repository checklist, capture it as a task list rather than only choosing the next task. Prefer task lists; create a Job only when durable job-level criteria, delegation, handoff, or commit/work-branch tracking are needed. Do not taskify ordinary Q&A; ask one clarifying question if the outcome is unclear.";
const ORIENTED_TASK_GUIDANCE: &str =
    "A concrete task is active. Keep working toward its completion criteria, but pause when the user asked only to plan or when proceeding would exceed the requested scope. Do not claim completion until criteria are met. Ask necessary clarifying questions; otherwise proceed within the task boundary.";
const ORIENTED_DECOMPOSITION_GUIDANCE: &str =
    "Task creation is bounded to oriented_root_task_id={root_task_id}, max_children={max_children}, and max_depth_below_oriented_task={max_depth}.";
const FOCUSED_JOB_PROGRESS_GUIDANCE_PREFIX: &str =
    "Keep working toward the Job's completion criteria by";
const FOCUSED_COMPLETION_GUIDANCE: &str =
    "Do not claim Job completion until criteria are met. Ask only necessary clarifying questions; otherwise proceed within the Job boundary.";
const FOCUSED_ACTIVE_TASK_GUIDANCE: &str = "advancing the active task";
const FOCUSED_MUTABLE_NEXT_TASK_GUIDANCE: &str = "choosing or creating the next concrete task";
const FOCUSED_IMMUTABLE_NEXT_TASK_GUIDANCE: &str = "choosing the next existing concrete task";
const FOCUSED_MUTABLE_STRUCTURE_GUIDANCE: &str = "Add child tasks when useful.";
const FOCUSED_IMMUTABLE_STRUCTURE_GUIDANCE: &str =
    "Task-definition edits are unavailable while focused job_mutable=false; choose existing tasks and update status/results instead.";

fn system_reminder(body: String) -> String {
    format!("<system-reminder>\n{body}\n</system-reminder>")
}

fn oriented_root_task_id(task_ref: &crate::agent_loop::OrientationTaskRef) -> &str {
    match task_ref {
        crate::agent_loop::OrientationTaskRef::TaskListItem { item_id, .. } => item_id,
        crate::agent_loop::OrientationTaskRef::DocketTask { task_id, .. } => task_id,
    }
}

pub fn runtime_context_already_includes_den_owned_blocks(runtime_context: &str) -> bool {
    let trimmed = runtime_context.trim();
    !trimmed.is_empty()
        && (trimmed.contains("Prompt memory blocks are Den-owned")
            || trimmed.contains("Runtime compaction context is Den-owned")
            || trimmed.contains("Den objective orientation is Den-owned"))
}

fn render_objective_orientation_context(
    profile_slug: &str,
    orientation: &ObjectiveOrientation,
) -> String {
    match orientation {
        ObjectiveOrientation::Freeform { policy } => {
            let task_definition_tools = if policy.may_define_task {
                "available"
            } else {
                "unavailable"
            };
            let task_definition_guidance = if policy.may_define_task {
                FREEFORM_TASK_DEFINITION_GUIDANCE
            } else {
                FREEFORM_CLOSED_TASK_POLICY_GUIDANCE
            };
            let pair_task_orientation_guidance = if profile_slug == "pair" && policy.may_define_task
            {
                format!(" {PAIR_FREEFORM_TASK_ORIENTATION_HINT}")
            } else {
                String::new()
            };
            system_reminder(format!(
                "{OBJECTIVE_ORIENTATION_PREAMBLE} orientation=freeform may_define_task={} task_definition_tools={task_definition_tools}. {FREEFORM_BASE_GUIDANCE} {task_definition_guidance}{}",
                policy.may_define_task, pair_task_orientation_guidance
            ))
        }
        ObjectiveOrientation::Oriented { task } => {
            let task_ref = serde_json::to_string(&task.task_ref)
                .unwrap_or_else(|_| "{\"kind\":\"unknown\"}".to_string());
            let root_task_id = oriented_root_task_id(&task.task_ref);
            let decomposition_guidance = ORIENTED_DECOMPOSITION_GUIDANCE
                .replace("{root_task_id}", root_task_id)
                .replace(
                    "{max_children}",
                    &task.child_policy.max_children.to_string(),
                )
                .replace(
                    "{max_depth}",
                    &task.child_policy.max_depth_below_oriented_task.to_string(),
                );
            system_reminder(format!(
                "{OBJECTIVE_ORIENTATION_PREAMBLE} orientation=oriented task_ref={task_ref} oriented_root_task_id={root_task_id} max_children={} max_depth_below_oriented_task={}. {ORIENTED_TASK_GUIDANCE} {decomposition_guidance}",
                task.child_policy.max_children,
                task.child_policy.max_depth_below_oriented_task
            ))
        }
        ObjectiveOrientation::Focused { job } => {
            let active_task_ref = job
                .active_task_ref
                .as_ref()
                .and_then(|task| serde_json::to_string(task).ok())
                .unwrap_or_else(|| "null".to_string());
            let task_guidance = if job.active_task_ref.is_some() {
                FOCUSED_ACTIVE_TASK_GUIDANCE
            } else if job.mutable {
                FOCUSED_MUTABLE_NEXT_TASK_GUIDANCE
            } else {
                FOCUSED_IMMUTABLE_NEXT_TASK_GUIDANCE
            };
            let structure_guidance = if job.mutable {
                FOCUSED_MUTABLE_STRUCTURE_GUIDANCE
            } else {
                FOCUSED_IMMUTABLE_STRUCTURE_GUIDANCE
            };
            let task_definition_tools = if job.mutable {
                "available"
            } else {
                "unavailable"
            };
            system_reminder(format!(
                "{OBJECTIVE_ORIENTATION_PREAMBLE} orientation=focused job_id={} job_mutable={} task_definition_tools={task_definition_tools} active_task_ref={active_task_ref}. {FOCUSED_JOB_PROGRESS_GUIDANCE_PREFIX} {task_guidance}. {FOCUSED_COMPLETION_GUIDANCE} {structure_guidance}",
                job.job_id,
                job.mutable
            ))
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
    parts.push(render_objective_orientation_context(
        profile_slug,
        objective_orientation,
    ));
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
    use crate::agent_loop::{FreeformPolicy, JobOrientation, OrientationTaskRef};

    #[test]
    fn runtime_context_already_includes_den_owned_blocks_detects_compaction() {
        assert!(runtime_context_already_includes_den_owned_blocks(
            "Runtime compaction context is Den-owned."
        ));
        assert!(!runtime_context_already_includes_den_owned_blocks(
            "plain runtime notes"
        ));
    }

    #[test]
    fn pair_freeform_guidance_prefers_task_lists_over_jobs() {
        let pair = render_objective_orientation_context(
            "pair",
            &ObjectiveOrientation::Freeform {
                policy: FreeformPolicy::task_definition_permitted(),
            },
        );
        assert!(pair.contains("Pair task-orientation hint"));
        assert!(pair.contains("capture it as a task list"));
        assert!(pair.contains("Prefer task lists; create a Job only when durable job-level criteria, delegation, handoff, or commit/work-branch tracking are needed."));

        let chat = render_objective_orientation_context(
            "chat",
            &ObjectiveOrientation::Freeform {
                policy: FreeformPolicy::task_definition_permitted(),
            },
        );
        assert!(!chat.contains("Pair task-orientation hint"));

        let pair_closed = render_objective_orientation_context(
            "pair",
            &ObjectiveOrientation::Freeform {
                policy: FreeformPolicy::closed(),
            },
        );
        assert!(!pair_closed.contains("Pair task-orientation hint"));
        assert!(pair_closed.contains("task_definition_tools=unavailable"));
        assert!(pair_closed
            .contains("Task-definition tools are unavailable in closed freeform orientation"));
    }

    #[test]
    fn focused_guidance_branches_on_active_task_and_mutability() {
        let active_task = OrientationTaskRef::DocketTask {
            job_id: Some("job-1".to_string()),
            task_id: "task-1".to_string(),
            title: Some("Do it".to_string()),
        };

        let active_mutable = render_objective_orientation_context(
            "pair",
            &ObjectiveOrientation::Focused {
                job: JobOrientation {
                    job_id: "job-1".to_string(),
                    active_task_ref: Some(active_task),
                    mutable: true,
                },
            },
        );
        assert!(active_mutable.contains("by advancing the active task"));
        assert!(active_mutable.contains("Add child tasks when useful."));
        assert!(!active_mutable.contains("If active_task_ref"));

        let no_active_mutable = render_objective_orientation_context(
            "pair",
            &ObjectiveOrientation::Focused {
                job: JobOrientation {
                    job_id: "job-1".to_string(),
                    active_task_ref: None,
                    mutable: true,
                },
            },
        );
        assert!(no_active_mutable.contains("by choosing or creating the next concrete task"));
        assert!(no_active_mutable.contains("Add child tasks when useful."));

        let no_active_immutable = render_objective_orientation_context(
            "pair",
            &ObjectiveOrientation::Focused {
                job: JobOrientation {
                    job_id: "job-1".to_string(),
                    active_task_ref: None,
                    mutable: false,
                },
            },
        );
        assert!(no_active_immutable.contains("by choosing the next existing concrete task"));
        assert!(no_active_immutable.contains("task_definition_tools=unavailable"));
        assert!(no_active_immutable
            .contains("Task-definition edits are unavailable while focused job_mutable=false"));
    }

    #[test]
    fn oriented_guidance_allows_planning_pause() {
        let oriented = render_objective_orientation_context(
            "pair",
            &ObjectiveOrientation::Oriented {
                task: crate::agent_loop::TaskOrientation {
                    task_ref: OrientationTaskRef::DocketTask {
                        job_id: None,
                        task_id: "task-1".to_string(),
                        title: Some("Plan the work".to_string()),
                    },
                    child_policy: crate::agent_loop::OrientedChildTaskPolicy {
                        max_children: 3,
                        max_depth_below_oriented_task: 1,
                    },
                },
            },
        );

        assert!(oriented.contains("pause when the user asked only to plan"));
        assert!(oriented.contains("exceed the requested scope"));
        assert!(oriented.contains("oriented_root_task_id=task-1"));
        assert!(oriented.contains("max_children=3"));
        assert!(oriented.contains("max_depth_below_oriented_task=1"));
    }
}
