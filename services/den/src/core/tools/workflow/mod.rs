use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::tools::constants::{
    DEN_JOB_CREATE, DEN_JOB_EVALUATE_CRITERION, DEN_JOB_EXECUTE, DEN_JOB_GET, DEN_JOB_LIST,
    DEN_JOB_UPDATE, DEN_TASK_CREATE, DEN_TASK_LIST, DEN_TASK_LISTS_GET_STATUS, DEN_TASK_LISTS_LIST,
    DEN_TASK_LISTS_UPDATE, DEN_TASK_LIST_CHECKOUT, DEN_TASK_LIST_SYNC, DEN_TASK_UPDATE,
    DEN_TASK_UPDATE_CURRENT_STATUS, DEN_WORK_CATALOG, DEN_WORK_DISPATCH, DEN_WORK_RUN_CANCEL,
    DEN_WORK_RUN_GET, DEN_WORK_RUN_LIST,
};
use den_docket::{
    self as docket, docket_job_status_report, DocketCommitPolicy, DocketCriterionStateUpdate,
    DocketCriterionStatus, DocketEffortHint, DocketExecutionLookup, DocketJobCreate,
    DocketJobCriterionInput, DocketJobExecuteRequest, DocketJobListFilter, DocketJobProjection,
    DocketJobStatus, DocketJobStatusReport, DocketJobUpdate, DocketService, DocketTaskCreate,
    DocketTaskDefinitionPatch, DocketTaskDifficulty, DocketTaskInput, DocketTaskKind,
    DocketTaskListFilter, DocketTaskRunStateUpdate, DocketTaskScope, DocketTaskStatus,
    DocketTaskUpdate, DocketValidationError, PgDocketService, TaskListCheckoutRequest,
    TaskListCheckoutSource, TaskListProjection, TaskListSyncRequest, TaskListVisibility,
};

use crate::{
    config::Config,
    core::tools::{session::DenToolInvocationContext, support::clean_optional},
    errors::{CustomError, DenError},
};
use den_memory::{tools as sqlite_memory, MemoryStoreManager};
use den_runtime::plan_mode;
use den_service::{
    bears::BearProfile, client_sessions, conversation::persistence as conversation_persistence,
};

const FOCUSED_CONVERSATION_TITLE_MAX_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OrientedTaskCreatePolicy {
    root_task_id: Uuid,
    max_children: usize,
    max_depth_below_oriented_task: usize,
}

pub(crate) fn is_workflow_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        DEN_TASK_LISTS_LIST
            | DEN_TASK_LISTS_GET_STATUS
            | DEN_TASK_LISTS_UPDATE
            | DEN_JOB_CREATE
            | DEN_JOB_LIST
            | DEN_JOB_GET
            | DEN_JOB_UPDATE
            | DEN_JOB_EXECUTE
            | DEN_JOB_EVALUATE_CRITERION
            | DEN_TASK_CREATE
            | DEN_TASK_LIST
            | DEN_TASK_UPDATE
            | DEN_TASK_UPDATE_CURRENT_STATUS
            | DEN_TASK_LIST_SYNC
            | DEN_TASK_LIST_CHECKOUT
            | DEN_WORK_DISPATCH
            | DEN_WORK_RUN_LIST
            | DEN_WORK_RUN_GET
            | DEN_WORK_RUN_CANCEL
            | DEN_WORK_CATALOG
    )
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskListListArguments {
    #[serde(default)]
    pub(crate) include_completed: bool,
    #[serde(default)]
    pub(crate) include_plan_mode: Option<bool>,
    #[serde(default)]
    pub(crate) include_artifacts: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketJobCreateArguments {
    pub(crate) goal: String,
    #[serde(default)]
    pub(crate) work_surface_ref: Option<String>,
    #[serde(default)]
    pub(crate) commit_policy: Option<DocketCommitPolicy>,
    #[serde(default)]
    pub(crate) work_branch: Option<String>,
    #[serde(default = "default_job_status")]
    pub(crate) status: DocketJobStatus,
    #[serde(default = "default_job_visibility")]
    pub(crate) visibility: TaskListVisibility,
    #[serde(default)]
    pub(crate) criteria: Vec<DocketJobCriterionInput>,
    #[serde(default)]
    pub(crate) tasks: Vec<DocketTaskInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketJobListArguments {
    #[serde(default, rename = "status")]
    pub(crate) statuses: Option<Vec<DocketJobStatus>>,
    #[serde(default)]
    pub(crate) include_cancelled: bool,
    #[serde(default)]
    pub(crate) limit: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketJobGetArguments {
    pub(crate) job_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketJobUpdateArguments {
    pub(crate) job_id: Uuid,
    #[serde(default)]
    pub(crate) goal: Option<String>,
    #[serde(default)]
    pub(crate) work_surface_ref: Option<String>,
    #[serde(default)]
    pub(crate) clear_work_surface_ref: bool,
    #[serde(default)]
    pub(crate) commit_policy: Option<DocketCommitPolicy>,
    #[serde(default)]
    pub(crate) clear_commit_policy: bool,
    #[serde(default)]
    pub(crate) status: Option<DocketJobStatus>,
    #[serde(default)]
    pub(crate) visibility: Option<TaskListVisibility>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketJobExecuteArguments {
    pub(crate) job_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketCriterionEvaluateArguments {
    pub(crate) job_id: Uuid,
    pub(crate) run_id: Uuid,
    pub(crate) criterion_id: Uuid,
    pub(crate) status: DocketCriterionStatus,
    #[serde(default)]
    pub(crate) evidence: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketTaskCreateArguments {
    #[serde(default)]
    pub(crate) job_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) session_anchor_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) parent_task_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) sibling_order: i32,
    #[serde(default = "default_task_kind")]
    pub(crate) kind: DocketTaskKind,
    #[serde(default = "default_task_scope")]
    pub(crate) scope: DocketTaskScope,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) completion_criteria: Vec<String>,
    #[serde(default)]
    pub(crate) difficulty: Option<DocketTaskDifficulty>,
    #[serde(default)]
    pub(crate) effort_hint: Option<DocketEffortHint>,
    #[serde(default)]
    pub(crate) assigned_to_role: Option<BearProfile>,
    #[serde(default)]
    pub(crate) created_in_run_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketTaskListArguments {
    #[serde(default)]
    pub(crate) job_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) session_anchor_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) parent_task_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) include_descendants: bool,
    #[serde(default)]
    pub(crate) limit: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketTaskUpdateArguments {
    pub(crate) task_id: Uuid,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) body: Option<String>,
    #[serde(default)]
    pub(crate) completion_criteria: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) parent_task_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) clear_parent_task_id: bool,
    #[serde(default)]
    pub(crate) sibling_order: Option<i32>,
    #[serde(default)]
    pub(crate) kind: Option<DocketTaskKind>,
    #[serde(default)]
    pub(crate) scope: Option<DocketTaskScope>,
    #[serde(default)]
    pub(crate) difficulty: Option<DocketTaskDifficulty>,
    #[serde(default)]
    pub(crate) effort_hint: Option<DocketEffortHint>,
    #[serde(default)]
    pub(crate) assigned_to_role: Option<BearProfile>,
    #[serde(default)]
    pub(crate) run_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) status: Option<DocketTaskStatus>,
    #[serde(default)]
    pub(crate) result_refs: Option<Value>,
    #[serde(default)]
    pub(crate) result_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocketCurrentTaskStatusArguments {
    #[serde(default)]
    pub(crate) job_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) run_id: Option<Uuid>,
    pub(crate) task_id: Uuid,
    pub(crate) status: DocketTaskStatus,
    #[serde(default)]
    pub(crate) result_refs: Option<Value>,
    #[serde(default)]
    pub(crate) result_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskListSyncArguments {
    pub(crate) task_list: TaskListProjection,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskListCheckoutArguments {
    #[serde(default)]
    pub(crate) job_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) parent_task_id: Option<Uuid>,
}

fn default_job_status() -> DocketJobStatus {
    DocketJobStatus::Ready
}

fn default_job_visibility() -> TaskListVisibility {
    TaskListVisibility::SameUser
}

fn default_task_kind() -> DocketTaskKind {
    DocketTaskKind::Execution
}

fn default_task_scope() -> DocketTaskScope {
    DocketTaskScope::Template
}

pub(crate) async fn list_task_lists(
    pool: &PgPool,
    _config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
    _activity_payload: fn(Option<&docket::TaskListLocalProjection>) -> Value,
    plan_mode_workplan_payload: fn(&plan_mode::PlanModeSessionRow) -> Value,
) -> Result<Value, CustomError> {
    let args: TaskListListArguments = serde_json::from_value(arguments)?;
    let include_plan_mode = args.include_plan_mode.unwrap_or(true);
    let include_artifacts = args.include_artifacts.unwrap_or(true);
    let plan_mode_gates = if include_plan_mode {
        plan_mode::list_for_bear(pool, context.bear_id, args.include_completed, 50).await?
    } else {
        Vec::new()
    };
    let plan_artifacts = if include_artifacts {
        match stores.store_for_bear(context.bear_id).await {
            Ok(store) => {
                sqlite_memory::sqlite_list_plan_artifacts(&store, BearProfile::Pair.as_str(), 50)
                    .await
                    .unwrap_or_else(|err| json!({ "error": err.to_string() }))
            }
            Err(err) => json!({ "error": err.to_string() }),
        }
    } else {
        json!([])
    };
    let linked_artifact_paths = plan_mode_gates
        .iter()
        .filter_map(|gate| gate.plan_artifact_path.as_deref())
        .collect::<Vec<_>>();
    let activity_plans: Vec<Value> = Vec::new();
    let task_lists: Vec<docket::TaskListProjection> = Vec::new();
    let workplans = plan_mode_gates
        .iter()
        .map(plan_mode_workplan_payload)
        .collect::<Vec<_>>();
    Ok(json!({
        "domain": "activity",
        "bear_id": context.bear_id,
        "viewer_role": role.as_str(),
        "planning_scope": "bear",
        "workplace": {
            "status": "unresolved",
            "note": "Workplace inference is not implemented yet; workspace/session metadata is returned as workplace reference candidates.",
            "reference_candidates": {
                "client_session_id": context.client_session_id,
                "session_id": context.session_id,
                "conversation_id": clean_optional(&context.conversation_id),
                "conversation_selection": context.conversation_selection,
                "runtime_target": context.runtime_target,
                "channel": context.channel,
            }
        },
        "task_lists": task_lists,
        "activities": activity_plans,
        "activity_plans": activity_plans,
        "workplans": workplans,
        "plan_mode_gates": plan_mode_gates,
        "plan_artifacts": plan_artifacts,
        "linked_plan_artifact_paths": linked_artifact_paths,
        "notes": [
            "list_task_lists is a Bear-level task-list/planning view. It includes checked-out Docket task-list projections, submitted/active plan-mode gates, and saved pair plan artifacts when available.",
            "A plan artifact in pair/plans/ may exist even when there is no active task list; this is planning state, not semantic memory.",
            "Role fields are provenance and policy hints, not product ownership. Cross-role visibility is not cross-role execution authority."
        ],
    }))
}

pub(crate) async fn get_task_list_status(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
    activity_payload: fn(Option<&docket::TaskListLocalProjection>) -> Value,
) -> Result<Value, CustomError> {
    let _ignored_arguments: Value = serde_json::from_value(arguments)?;
    let session_anchor_id = match context.client_session_id.as_deref() {
        Some(client_session_id) => client_sessions::find_for_user_bear_session_id(
            pool,
            context.user_id,
            context.bear_id,
            client_session_id,
        )
        .await?
        .map(|session| session.id),
        None => None,
    };

    let tasks = if let Some(session_anchor_id) = session_anchor_id {
        PgDocketService::from_pool(pool)
            .list_tasks(
                context.bear_id,
                DocketTaskListFilter {
                    job_id: None,
                    session_anchor_id: Some(session_anchor_id),
                    parent_task_id: None,
                    include_descendants: false,
                    limit: 500,
                },
            )
            .await?
    } else {
        Vec::new()
    };

    let task_list = session_anchor_id.and_then(|session_anchor_id| {
        docket::task_list_projection_from_session_tasks(
            context.bear_id,
            role,
            clean_optional(&context.conversation_id)
                .as_deref()
                .unwrap_or(""),
            session_anchor_id,
            &tasks,
        )
    });

    let summary = task_list
        .as_ref()
        .map(task_list_summary)
        .unwrap_or_else(|| {
            if session_anchor_id.is_some() {
                "Current session has no anchored tasks.".to_string()
            } else {
                "No current client session anchor is available for session tasks.".to_string()
            }
        });
    let item_counts = task_list
        .as_ref()
        .map(task_list_item_counts)
        .unwrap_or_else(|| {
            json!({
                "total": 0,
                "pending": 0,
                "in_progress": 0,
                "blocked": 0,
                "completed": 0,
                "cancelled": 0,
            })
        });

    Ok(json!({
        "domain": "activity",
        "bear_id": context.bear_id,
        "viewer_role": role.as_str(),
        "content": task_list.as_ref().map(task_list_card_content).unwrap_or_else(|| summary.clone()),
        "summary": summary,
        "found": task_list.is_some(),
        "count": tasks.len(),
        "phase": task_list.as_ref().map(|task_list| task_list.status.as_str()),
        "status": task_list.as_ref().map(|task_list| task_list.status.as_str()),
        "execution_allowed": task_list
            .as_ref()
            .is_some_and(|task_list| task_list.status != "planned"),
        "item_counts": item_counts,
        "task_list": task_list,
        "activity": activity_payload(None),
        "plan": task_list,
        "notes": [
            "Session-anchored tasks are durable Docket tasks associated with the current client session.",
            "A planned session task list is visible for review but is not an instruction to start execution."
        ]
    }))
}

pub(crate) async fn update_task_list(
    _pool: &PgPool,
    _context: &DenToolInvocationContext,
    _role: BearProfile,
    arguments: Value,
    _activity_payload: fn(Option<&docket::TaskListLocalProjection>) -> Value,
) -> Result<Value, CustomError> {
    let _ignored_arguments: Value = serde_json::from_value(arguments)?;
    Err(DenError::ValidationError(
        "update_task_list is unavailable; use Docket job/task tools for durable task and job state"
            .to_string(),
    )
    .into())
}

/// Resolve a `work_surface_ref` against managed work surfaces. When the ref
/// names one, the bear must be assigned to it, and the canonical name + id
/// come back; names matching no managed surface pass through unchanged (the
/// sandbox provider is the final validator for those).

fn truncate_focused_conversation_title(title: &str) -> String {
    title
        .chars()
        .take(FOCUSED_CONVERSATION_TITLE_MAX_CHARS)
        .collect()
}

fn focused_conversation_title(goal: &str, status_report: &DocketJobStatusReport) -> String {
    let suffix = status_report
        .current_task_title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .or_else(|| {
            if status_report.task_counts.blocked > 0 || status_report.next_action == "blocked" {
                Some("blocked".to_string())
            } else if status_report.next_action == "done"
                || status_report.job_status == "completed"
                || (status_report.tasks_complete && status_report.criteria_complete)
            {
                Some("complete".to_string())
            } else if status_report.run_id.is_some() || status_report.job_status == "running" {
                Some("selecting next task".to_string())
            } else {
                None
            }
        });

    let goal = goal.trim();
    let title = match suffix {
        Some(suffix) if !goal.is_empty() => format!("{goal} - {suffix}"),
        Some(suffix) => suffix,
        None => goal.to_string(),
    };
    truncate_focused_conversation_title(&title)
}

fn oriented_task_create_policy(runtime: Option<&Value>) -> Option<OrientedTaskCreatePolicy> {
    let orientation = runtime?.get("objective_orientation")?;
    if orientation.get("kind").and_then(Value::as_str) != Some("oriented") {
        return None;
    }

    let task = orientation.get("task")?;
    let task_ref = task.get("task_ref")?;
    let root_task_id = task_ref
        .get("task_id")
        .or_else(|| task_ref.get("item_id"))
        .and_then(Value::as_str)
        .and_then(|task_id| Uuid::parse_str(task_id).ok())?;
    let child_policy = task.get("child_policy")?;
    Some(OrientedTaskCreatePolicy {
        root_task_id,
        max_children: child_policy
            .get("max_children")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        max_depth_below_oriented_task: child_policy
            .get("max_depth_below_oriented_task")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
    })
}

fn oriented_new_child_depth(
    root_task_id: Uuid,
    parent_task_id: Uuid,
    descendants: &[den_docket::DocketTaskProjection],
) -> Option<usize> {
    if parent_task_id == root_task_id {
        return Some(1);
    }

    let mut depth = 1;
    let mut current_parent_id = parent_task_id;
    // ponytail: the oriented decomposition tree is deliberately tiny; if the
    // cap grows large, replace this linear ancestor walk with an id->parent map.
    loop {
        let parent = descendants
            .iter()
            .find(|task| task.task.id == current_parent_id)?;
        depth += 1;
        match parent.task.parent_task_id {
            Some(grandparent_id) if grandparent_id == root_task_id => return Some(depth),
            Some(grandparent_id) => current_parent_id = grandparent_id,
            None => return None,
        }
    }
}

async fn enforce_oriented_task_create_policy(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    args: &DocketTaskCreateArguments,
) -> Result<(), CustomError> {
    let Some(parent_task_id) = args.parent_task_id else {
        return Ok(());
    };
    let Some(policy) = oriented_task_create_policy(context.runtime.as_ref()) else {
        return Ok(());
    };

    let docket = PgDocketService::from_pool(pool);
    let descendants = docket
        .list_tasks(
            context.bear_id,
            DocketTaskListFilter {
                job_id: None,
                session_anchor_id: None,
                parent_task_id: Some(policy.root_task_id),
                include_descendants: true,
                limit: 500,
            },
        )
        .await?;
    let Some(new_child_depth) =
        oriented_new_child_depth(policy.root_task_id, parent_task_id, &descendants)
    else {
        return Err(DenError::ValidationError(
            "oriented task decomposition can only create child tasks under the oriented task"
                .to_string(),
        )
        .into());
    };
    let direct_children = docket
        .list_tasks(
            context.bear_id,
            DocketTaskListFilter {
                job_id: None,
                session_anchor_id: None,
                parent_task_id: Some(parent_task_id),
                include_descendants: false,
                limit: (policy.max_children as i64).saturating_add(1).max(1),
            },
        )
        .await?;
    validate_oriented_child_capacity(policy, new_child_depth, direct_children.len())?;

    Ok(())
}

fn validate_oriented_child_capacity(
    policy: OrientedTaskCreatePolicy,
    new_child_depth: usize,
    direct_child_count: usize,
) -> Result<(), DenError> {
    if new_child_depth > policy.max_depth_below_oriented_task {
        return Err(DenError::ValidationError(format!(
            "oriented task decomposition allows at most {} level(s) below the oriented task",
            policy.max_depth_below_oriented_task
        )));
    }
    if direct_child_count >= policy.max_children {
        return Err(DenError::ValidationError(format!(
            "oriented task decomposition allows at most {} child tasks under the selected parent",
            policy.max_children
        )));
    }

    Ok(())
}

fn count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

fn docket_job_summary(job: &DocketJobProjection) -> String {
    let task_label = count_label(job.tasks.len(), "task", "tasks");
    let criterion_label = count_label(job.criteria.len(), "criterion", "criteria");
    format!(
        "Docket job '{}' is {} with {task_label} and {criterion_label}.",
        job.job.goal, job.job.status
    )
}

fn docket_job_card_content(
    job: &DocketJobProjection,
    status_report: &DocketJobStatusReport,
) -> String {
    let tasks = &status_report.task_counts;
    let criteria = &status_report.criteria_counts;
    let mut lines = vec![
        format!("Job: {}", job.job.goal),
        format!("Status: {}", status_report.job_status),
        format!(
            "Tasks: {} pending, {} in progress, {} done, {} blocked, {} cancelled",
            tasks.pending, tasks.in_progress, tasks.done, tasks.blocked, tasks.cancelled
        ),
        format!(
            "Criteria: {} unmet, {} met, {} waived",
            criteria.unmet, criteria.met, criteria.waived
        ),
        format!("Next action: {}", status_report.next_action),
    ];
    if let Some(title) = status_report.current_task_title.as_deref() {
        lines.push(format!("Current task: {title}"));
    }
    lines.join("\n")
}

fn human_task_status_label(status: &str) -> &'static str {
    match status {
        "pending" => "pending",
        "in_progress" => "in progress",
        "done" => "done",
        "blocked" => "blocked",
        "cancelled" => "cancelled",
        _ => "updated",
    }
}

fn task_status_card_content(
    task: &docket::DocketTaskProjection,
    status: &str,
    status_report: Option<&DocketJobStatusReport>,
) -> String {
    let status_label = human_task_status_label(status);
    let mut lines = vec![
        format!("Task marked {status_label}: {}", task.task.title),
        format!("Status: {status_label}"),
    ];
    if let Some(result_summary) = task
        .run_state
        .as_ref()
        .and_then(|state| state.result_summary.as_deref())
        .filter(|summary| !summary.trim().is_empty())
    {
        lines.push(format!("Result: {result_summary}"));
    }
    if task
        .run_state
        .as_ref()
        .and_then(|state| state.result_refs.as_ref())
        .is_some()
    {
        lines.push("Result references recorded.".to_string());
    }
    if let Some(report) = status_report {
        let counts = &report.task_counts;
        lines.push(format!("Job next action: {}", report.next_action));
        lines.push(format!(
            "Job tasks: {} pending, {} in progress, {} done, {} blocked, {} cancelled",
            counts.pending, counts.in_progress, counts.done, counts.blocked, counts.cancelled
        ));
    }
    lines.join("\n")
}

fn docket_job_rows_summary(jobs: &[docket::DocketJobRow]) -> String {
    if jobs.is_empty() {
        "No Docket jobs matched the filters.".to_string()
    } else {
        format!(
            "Found {}.",
            count_label(jobs.len(), "Docket job", "Docket jobs")
        )
    }
}

fn task_projection_status(task: &docket::DocketTaskProjection) -> &str {
    task.run_state
        .as_ref()
        .map(|state| state.status.as_str())
        .unwrap_or("pending")
}

fn docket_task_row_summary(task: &docket::DocketTaskRow) -> String {
    format!("Task '{}' was created.", task.title)
}

fn docket_task_summary(task: &docket::DocketTaskProjection) -> String {
    format!(
        "Task '{}' is {}.",
        task.task.title,
        task_projection_status(task)
    )
}

fn docket_tasks_summary(tasks: &[docket::DocketTaskProjection]) -> String {
    if tasks.is_empty() {
        "No Docket tasks matched the filters.".to_string()
    } else {
        format!(
            "Found {}.",
            count_label(tasks.len(), "Docket task", "Docket tasks")
        )
    }
}

fn docket_tasks_card_content(tasks: &[docket::DocketTaskProjection]) -> String {
    if tasks.is_empty() {
        return "No Docket tasks matched the filters.".to_string();
    }
    let counts = docket_task_counts(tasks);
    let mut lines = vec![format!(
        "Found {}.",
        count_label(tasks.len(), "Docket task", "Docket tasks")
    )];
    lines.push(format!(
        "Status: {} pending, {} in progress, {} done, {} blocked, {} cancelled.",
        counts["pending"],
        counts["in_progress"],
        counts["done"],
        counts["blocked"],
        counts["cancelled"]
    ));
    for task in tasks.iter().take(5) {
        lines.push(format!(
            "- {} — {}",
            task.task.title,
            human_task_status_label(task_projection_status(task))
        ));
    }
    if tasks.len() > 5 {
        lines.push(format!("…and {} more.", tasks.len() - 5));
    }
    lines.join("\n")
}

fn docket_task_counts(tasks: &[docket::DocketTaskProjection]) -> Value {
    let mut pending = 0usize;
    let mut in_progress = 0usize;
    let mut done = 0usize;
    let mut blocked = 0usize;
    let mut cancelled = 0usize;
    for task in tasks {
        match task_projection_status(task) {
            "in_progress" => in_progress += 1,
            "done" => done += 1,
            "blocked" => blocked += 1,
            "cancelled" => cancelled += 1,
            _ => pending += 1,
        }
    }
    json!({
        "total": tasks.len(),
        "pending": pending,
        "in_progress": in_progress,
        "done": done,
        "blocked": blocked,
        "cancelled": cancelled,
    })
}

fn task_list_summary(task_list: &TaskListProjection) -> String {
    format!(
        "Task list '{}' is {} with {}.",
        task_list.title,
        task_list.status,
        count_label(task_list.items.len(), "item", "items")
    )
}

fn task_list_card_content(task_list: &TaskListProjection) -> String {
    let counts = task_list_item_counts(task_list);
    let mut lines = vec![task_list_summary(task_list)];
    lines.push(format!(
        "Items: {} pending, {} in progress, {} completed, {} blocked, {} cancelled.",
        counts["pending"],
        counts["in_progress"],
        counts["completed"],
        counts["blocked"],
        counts["cancelled"]
    ));
    if task_list.status == "planned" {
        lines.push("Execution has not started.".to_string());
    }
    if let Some(item) = &task_list.current_item {
        lines.push(format!("Current item: {} — {}.", item.title, item.status));
    }
    lines.join("\n")
}

fn task_list_item_counts(task_list: &TaskListProjection) -> Value {
    let mut pending = 0usize;
    let mut in_progress = 0usize;
    let mut blocked = 0usize;
    let mut completed = 0usize;
    let mut cancelled = 0usize;
    for item in &task_list.items {
        match item.status {
            docket::TaskListItemStatus::InProgress => in_progress += 1,
            docket::TaskListItemStatus::Blocked => blocked += 1,
            docket::TaskListItemStatus::Completed => completed += 1,
            docket::TaskListItemStatus::Cancelled => cancelled += 1,
            docket::TaskListItemStatus::Pending => pending += 1,
        }
    }
    json!({
        "total": task_list.items.len(),
        "pending": pending,
        "in_progress": in_progress,
        "blocked": blocked,
        "completed": completed,
        "cancelled": cancelled,
    })
}

async fn session_anchored_task_list_projection(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    session_anchor_id: Uuid,
) -> Result<Option<TaskListProjection>, CustomError> {
    let tasks = PgDocketService::from_pool(pool)
        .list_tasks(
            context.bear_id,
            DocketTaskListFilter {
                job_id: None,
                session_anchor_id: Some(session_anchor_id),
                parent_task_id: None,
                include_descendants: false,
                limit: 500,
            },
        )
        .await?;
    Ok(docket::task_list_projection_from_session_tasks(
        context.bear_id,
        role,
        clean_optional(&context.conversation_id)
            .as_deref()
            .unwrap_or(""),
        session_anchor_id,
        &tasks,
    ))
}

fn refresh_runtime_session_activity_plan(
    context: &DenToolInvocationContext,
    task_list: Option<TaskListProjection>,
) {
    let (Some(conversation_id), Some(client_session_id)) = (
        clean_optional(&context.conversation_id),
        context.client_session_id.as_deref(),
    ) else {
        return;
    };
    den_runtime::native_runtime::update_native_client_session_active_activity_plan(
        &conversation_id,
        client_session_id,
        task_list,
    );
}

async fn update_focused_conversation_title(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    job: &DocketJobProjection,
    status_report: &DocketJobStatusReport,
) -> Result<(), CustomError> {
    let Some(conversation_id) = clean_optional(&context.conversation_id) else {
        return Ok(());
    };
    let title = focused_conversation_title(&job.job.goal, status_report);
    conversation_persistence::set_conversation_title_and_sync_client_sessions(
        pool,
        context.bear_id,
        &conversation_id,
        &title,
    )
    .await?;
    Ok(())
}

async fn resolve_surface_ref(
    pool: &PgPool,
    bear_id: Uuid,
    work_surface_ref: Option<String>,
) -> Result<(Option<String>, Option<Uuid>), CustomError> {
    let Some(ref_name) = work_surface_ref
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return Ok((None, None));
    };
    match den_service::work_surfaces::surface_by_name(pool, ref_name).await? {
        Some(surface) => {
            if !den_service::work_surfaces::bear_may_use_surface(pool, bear_id, surface.id).await? {
                return Err(DenError::ValidationError(format!(
                    "bear is not assigned to work surface '{}'; pick a surface listed by get_work_catalog or ask a surface manager to assign this bear",
                    surface.name
                ))
                .into());
            }
            Ok((Some(surface.name), Some(surface.id)))
        }
        None => Ok((Some(ref_name.to_string()), None)),
    }
}

pub(crate) async fn create_job(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    if !matches!(role, BearProfile::Chat | BearProfile::Pair) {
        return Err(
            DenError::from(DocketValidationError::InvalidJobCreatorRole {
                role: role.as_str().to_string(),
            })
            .into(),
        );
    }
    let args: DocketJobCreateArguments = serde_json::from_value(arguments)?;
    let (work_surface_ref, work_surface_id) =
        resolve_surface_ref(pool, context.bear_id, args.work_surface_ref).await?;
    let job = PgDocketService::from_pool(pool)
        .create_job(DocketJobCreate {
            bear_id: context.bear_id,
            created_by_user_id: context.user_id,
            created_by_role: role.as_str().to_string(),
            goal: args.goal,
            work_surface_ref,
            work_surface_id,
            commit_policy: args.commit_policy,
            work_branch: args.work_branch,
            status: args.status,
            visibility: args.visibility,
            source_conversation_id: clean_optional(&context.conversation_id),
            objective_kind: None,
            criteria: args.criteria,
            tasks: args.tasks,
        })
        .await?;
    let task_list = docket::task_list_projection_from_docket_job(&job, None);
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "summary": docket_job_summary(&job),
        "job": job,
        "task_list": task_list,
        "notes": [
            "Created durable Docket job state; execution is not started by this tool.",
            "Use get_job for canonical job/task state; checkout_task_list exposes a Docket job as a session task-list projection."
        ]
    }))
}

pub(crate) async fn list_jobs(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketJobListArguments = serde_json::from_value(arguments)?;
    let jobs = PgDocketService::from_pool(pool)
        .list_jobs(
            context.bear_id,
            DocketJobListFilter {
                statuses: args.statuses,
                include_cancelled: args.include_cancelled,
                limit: args.limit,
            },
        )
        .await?;
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "summary": docket_job_rows_summary(&jobs),
        "count": jobs.len(),
        "jobs": jobs,
    }))
}

pub(crate) async fn get_job(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketJobGetArguments = serde_json::from_value(arguments)?;
    let job = PgDocketService::from_pool(pool)
        .get_job(context.bear_id, args.job_id)
        .await?;
    let task_list = job
        .as_ref()
        .map(|job| docket::task_list_projection_from_docket_job(job, None));
    let status_report = job.as_ref().map(docket_job_status_report);

    // Work-run visibility: recent runs (with queue placement) and
    // latest-attempt failures that need triage, so a bear checking on its
    // job sees blocked reasons without a second tool call.
    let (work_runs, work_attention) = if job.is_some() {
        let runs = den_docket::work_runs::list_work_runs(
            pool,
            den_docket::work_runs::WorkRunListFilter {
                bear_id: Some(context.bear_id),
                job_id: Some(args.job_id),
                limit: 20,
                ..den_docket::work_runs::WorkRunListFilter::default()
            },
        )
        .await?;
        let queue_by_run = work_run_queue_map(pool, &runs).await?;
        let items: Vec<Value> = runs
            .iter()
            .map(|run| {
                let mut item = work_run_summary_json(run);
                if let Some(queue) = queue_by_run.get(&run.id) {
                    item["queue"] = queue.clone();
                }
                item
            })
            .collect();
        let attention = den_docket::work_runs::attention_work_runs(
            pool,
            context.bear_id,
            Some(args.job_id),
            10,
        )
        .await?;
        (Some(items), Some(attention))
    } else {
        (None, None)
    };

    let summary = job
        .as_ref()
        .map(docket_job_summary)
        .unwrap_or_else(|| format!("No Docket job found for {}.", args.job_id));
    let content = job
        .as_ref()
        .zip(status_report.as_ref())
        .map(|(job, report)| docket_job_card_content(job, report))
        .unwrap_or_else(|| summary.clone());
    let task_counts = status_report.as_ref().map(|report| &report.task_counts);
    let criteria_counts = status_report.as_ref().map(|report| &report.criteria_counts);
    let next_action = status_report
        .as_ref()
        .map(|report| report.next_action.as_str());
    let current_task_title = status_report
        .as_ref()
        .and_then(|report| report.current_task_title.as_deref());

    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "content": content,
        "summary": summary,
        "found": job.is_some(),
        "job_status": status_report.as_ref().map(|report| report.job_status.as_str()),
        "task_counts": task_counts,
        "criteria_counts": criteria_counts,
        "next_action": next_action,
        "current_task_title": current_task_title,
        "job": job,
        "task_list": task_list,
        "item_counts": task_list.as_ref().map(task_list_item_counts),
        "status_report": status_report,
        "work_runs": work_runs,
        "work_attention": work_attention,
    }))
}

pub(crate) async fn update_job(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketJobUpdateArguments = serde_json::from_value(arguments)?;
    let (work_surface_ref, work_surface_id) = if args.clear_work_surface_ref {
        (Some(None), Some(None))
    } else if args.work_surface_ref.is_some() {
        let (surface_ref, surface_id) =
            resolve_surface_ref(pool, context.bear_id, args.work_surface_ref).await?;
        (Some(surface_ref), Some(surface_id))
    } else {
        (None, None)
    };
    let job = PgDocketService::from_pool(pool)
        .update_job(DocketJobUpdate {
            bear_id: context.bear_id,
            job_id: args.job_id,
            actor_role: role,
            actor_user_id: Some(context.user_id),
            actor_agent_id: clean_optional(&context.binding_id),
            goal: args.goal,
            work_surface_ref,
            work_surface_id,
            commit_policy: args
                .clear_commit_policy
                .then_some(None)
                .or_else(|| args.commit_policy.map(Some)),
            status: args.status,
            visibility: args.visibility,
        })
        .await?;
    let status_report = docket_job_status_report(&job);
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "job": job,
        "status_report": status_report,
    }))
}

pub(crate) async fn execute_job(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketJobExecuteArguments = serde_json::from_value(arguments)?;
    if role != BearProfile::Pair {
        return Err(DenError::Authorization(
            "execute_job is currently limited to pair stance".to_string(),
        )
        .into());
    }
    if let Some(mode_label) = context
        .session_policy
        .as_ref()
        .and_then(|policy| policy.get("mode_label"))
        .and_then(Value::as_str)
    {
        if !mode_label.eq_ignore_ascii_case("write") {
            return Err(DenError::Authorization(format!(
                "Docket execution is active, but client mode is {mode_label}; switch the session to Write mode before proceeding"
            ))
            .into());
        }
    }
    let outcome = PgDocketService::from_pool(pool)
        .execute_job(DocketJobExecuteRequest {
            bear_id: context.bear_id,
            job_id: args.job_id,
            actor_role: role,
            actor_user_id: Some(context.user_id),
            actor_agent_id: clean_optional(&context.binding_id),
            session_id: Some(context.session_id.clone()),
            source_conversation_id: clean_optional(&context.conversation_id),
            source_client_session_id: context.client_session_id.clone(),
        })
        .await?;
    let status_report = docket_job_status_report(&outcome.job);
    update_focused_conversation_title(pool, context, &outcome.job, &status_report).await?;
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "execution": outcome,
        "status_report": status_report,
    }))
}

pub(crate) async fn evaluate_criterion(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketCriterionEvaluateArguments = serde_json::from_value(arguments)?;
    let job = PgDocketService::from_pool(pool)
        .evaluate_criterion(DocketCriterionStateUpdate {
            bear_id: context.bear_id,
            job_id: args.job_id,
            run_id: args.run_id,
            criterion_id: args.criterion_id,
            status: args.status,
            evidence: args.evidence,
            actor_role: role,
            actor_user_id: Some(context.user_id),
            actor_agent_id: clean_optional(&context.binding_id),
        })
        .await?;
    let status_report = docket_job_status_report(&job);
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "job": job,
        "status_report": status_report,
    }))
}

async fn default_task_session_anchor_id(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    args: &DocketTaskCreateArguments,
) -> Result<Option<Uuid>, CustomError> {
    if args.job_id.is_some() || args.session_anchor_id.is_some() {
        return Ok(args.session_anchor_id);
    }

    let Some(client_session_id) = context.client_session_id.as_deref() else {
        return Err(DenError::ValidationError(
            "create_task without job_id needs the current client session, but no client session id is available in this tool context".to_string(),
        )
        .into());
    };

    let Some(session) = client_sessions::find_for_user_bear_session_id(
        pool,
        context.user_id,
        context.bear_id,
        client_session_id,
    )
    .await?
    else {
        return Err(DenError::ValidationError(
            "create_task without job_id could not resolve the current client session anchor"
                .to_string(),
        )
        .into());
    };

    Ok(Some(session.id))
}

pub(crate) async fn create_task(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketTaskCreateArguments = serde_json::from_value(arguments)?;
    let session_anchor_id = default_task_session_anchor_id(pool, context, &args).await?;
    enforce_oriented_task_create_policy(pool, context, &args).await?;
    let task = PgDocketService::from_pool(pool)
        .create_task(DocketTaskCreate {
            bear_id: context.bear_id,
            job_id: args.job_id,
            session_anchor_id,
            parent_task_id: args.parent_task_id,
            sibling_order: args.sibling_order,
            kind: args.kind,
            scope: args.scope,
            title: args.title,
            body: args.body,
            completion_criteria: args.completion_criteria,
            difficulty: args.difficulty,
            effort_hint: args.effort_hint,
            assigned_to_role: args.assigned_to_role,
            created_by_role: role.as_str().to_string(),
            created_by_user_id: Some(context.user_id),
            created_by_agent_id: clean_optional(&context.binding_id),
            created_in_run_id: args.created_in_run_id,
        })
        .await?;
    let task_list = if args.job_id.is_none() {
        if let Some(session_anchor_id) = session_anchor_id {
            session_anchored_task_list_projection(pool, context, role, session_anchor_id).await?
        } else {
            None
        }
    } else {
        None
    };
    if args.job_id.is_none() {
        refresh_runtime_session_activity_plan(context, task_list.clone());
    }
    let task_list_phase = task_list
        .as_ref()
        .map(|task_list| task_list.status.as_str());
    let execution_allowed = task_list_phase.is_some_and(|phase| phase != "planned");
    let summary = if args.job_id.is_none() && matches!(task_list_phase, Some("planned")) {
        format!(
            "Planned Docket task '{}'. Execution has not started.",
            task.title
        )
    } else {
        docket_task_row_summary(&task)
    };
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "content": summary,
        "summary": summary,
        "task": task,
        "task_list": task_list,
        "task_list_phase": task_list_phase,
        "execution_allowed": execution_allowed,
        "item_counts": task_list.as_ref().map(task_list_item_counts),
        "notes": if args.job_id.is_none() && matches!(task_list_phase, Some("planned")) {
            vec![
                "Created a durable session-anchored Docket task definition.",
                "The session task list is planned; wait for an explicit start/activation request before executing pending tasks."
            ]
        } else {
            vec![
                "Created durable Docket task definition. Status and results remain run-scoped."
            ]
        }
    }))
}

pub(crate) async fn list_tasks(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketTaskListArguments = serde_json::from_value(arguments)?;
    let tasks = PgDocketService::from_pool(pool)
        .list_tasks(
            context.bear_id,
            DocketTaskListFilter {
                job_id: args.job_id,
                session_anchor_id: args.session_anchor_id,
                parent_task_id: args.parent_task_id,
                include_descendants: args.include_descendants,
                limit: args.limit,
            },
        )
        .await?;
    let content = docket_tasks_card_content(&tasks);
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "content": content,
        "summary": docket_tasks_summary(&tasks),
        "count": tasks.len(),
        "counts": docket_task_counts(&tasks),
        "tasks": tasks,
    }))
}

pub(crate) async fn update_task(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketTaskUpdateArguments = serde_json::from_value(arguments)?;
    if args.run_id.is_some()
        || args.status.is_some()
        || args.result_refs.is_some()
        || args.result_summary.is_some()
    {
        return Err(DenError::ValidationError(
            "update_task only edits durable task definition fields; use update_current_task_status for run-scoped status/results in the active run".to_string(),
        )
        .into());
    }
    let task = PgDocketService::from_pool(pool)
        .update_task(DocketTaskUpdate {
            bear_id: context.bear_id,
            job_id: None,
            task_id: args.task_id,
            actor_role: role,
            actor_user_id: Some(context.user_id),
            actor_agent_id: clean_optional(&context.binding_id),
            definition: DocketTaskDefinitionPatch {
                title: args.title,
                body: args.body,
                completion_criteria: args.completion_criteria,
                parent_task_id: args
                    .clear_parent_task_id
                    .then_some(None)
                    .or_else(|| args.parent_task_id.map(Some)),
                sibling_order: args.sibling_order,
                kind: args.kind,
                scope: args.scope,
                difficulty: args.difficulty.map(Some),
                effort_hint: args.effort_hint.map(Some),
                assigned_to_role: args.assigned_to_role.map(Some),
            },
            run_state: None,
        })
        .await?;
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "summary": format!("Updated {}", docket_task_summary(&task)),
        "task": task,
        "notes": [
            "Updated durable Docket task definition. Use update_current_task_status for active-run status/results."
        ]
    }))
}

pub(crate) async fn update_current_task_status(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketCurrentTaskStatusArguments = serde_json::from_value(arguments)?;
    if args.job_id.is_some() != args.run_id.is_some() {
        return Err(DenError::ValidationError(
            "update_current_task_status requires job_id and run_id together; pass both for explicit scope or neither to use the active Docket run"
                .to_string(),
        )
        .into());
    }
    let execution = if args.job_id.is_some() {
        None
    } else {
        let lookup = DocketExecutionLookup {
            session_id: Some(context.session_id.clone()),
            source_conversation_id: clean_optional(&context.conversation_id),
            source_client_session_id: context.client_session_id.clone(),
        };
        PgDocketService::from_pool(pool)
            .get_active_execution_session(context.bear_id, role, lookup)
            .await?
    };
    let job_id = args
        .job_id
        .or_else(|| execution.as_ref().map(|execution| execution.job_id))
        .ok_or_else(|| {
            DenError::ValidationError(
                "update_current_task_status needs an active Docket run for this session; call execute_job or checkout_task_list for the job first"
                    .to_string(),
            )
        })?;
    let run_id = args
        .run_id
        .or_else(|| execution.as_ref().map(|execution| execution.run_id))
        .ok_or_else(|| {
            DenError::ValidationError(
                "update_current_task_status needs an active Docket run for this session; call execute_job or checkout_task_list for the job first"
                    .to_string(),
            )
        })?;
    let task = PgDocketService::from_pool(pool)
        .update_task(DocketTaskUpdate {
            bear_id: context.bear_id,
            job_id: Some(job_id),
            task_id: args.task_id,
            actor_role: role,
            actor_user_id: Some(context.user_id),
            actor_agent_id: clean_optional(&context.binding_id),
            definition: DocketTaskDefinitionPatch::default(),
            run_state: Some(DocketTaskRunStateUpdate {
                run_id,
                status: args.status,
                result_refs: args.result_refs,
                result_summary: args.result_summary,
            }),
        })
        .await?;
    let job = PgDocketService::from_pool(pool)
        .get_job(context.bear_id, job_id)
        .await?;
    let status_report = job.as_ref().map(docket_job_status_report);
    if let (Some(job), Some(status_report)) = (&job, &status_report) {
        update_focused_conversation_title(pool, context, job, status_report).await?;
    }
    let task_list = job
        .as_ref()
        .map(|job| den_docket::task_list_projection_from_docket_job(job, None));
    let status = task_projection_status(&task).to_string();
    let status_label = human_task_status_label(&status);
    let content = task_status_card_content(&task, &status, status_report.as_ref());
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "content": content,
        "summary": format!("Task '{}' is now {status_label}.", task.task.title),
        "task_title": task.task.title,
        "task_status": status,
        "task_status_label": status_label,
        "result_summary": task.run_state.as_ref().and_then(|state| state.result_summary.as_deref()),
        "has_result_refs": task.run_state.as_ref().and_then(|state| state.result_refs.as_ref()).is_some(),
        "task_counts": status_report.as_ref().map(|report| &report.task_counts),
        "next_action": status_report.as_ref().map(|report| report.next_action.as_str()),
        "item_counts": task_list.as_ref().map(task_list_item_counts),
        "task": task,
        "docket": {
            "active_job_id": job_id,
            "active_run_id": run_id,
            "active_task_id": execution.as_ref().and_then(|execution| execution.task_id).unwrap_or(args.task_id),
            "source": if execution.is_some() { "docket_execution_session" } else { "explicit_task_status_scope" }
        },
        "task_list": task_list,
        "status_report": status_report,
    }))
}

pub(crate) async fn sync_task_list(pool: &PgPool, arguments: Value) -> Result<Value, CustomError> {
    let args: TaskListSyncArguments = serde_json::from_value(arguments)?;
    let outcome = PgDocketService::from_pool(pool)
        .sync_task_list(TaskListSyncRequest {
            task_list: args.task_list,
        })
        .await?;
    let summary = if outcome.applied {
        format!("Synced {}", task_list_summary(&outcome.task_list))
    } else if outcome.review_required {
        format!(
            "Task list '{}' needs review before sync.",
            outcome.task_list.title
        )
    } else if !outcome.conflicts.is_empty() {
        format!(
            "Task list '{}' has {}.",
            outcome.task_list.title,
            count_label(outcome.conflicts.len(), "sync conflict", "sync conflicts")
        )
    } else {
        outcome.message.clone()
    };
    Ok(json!({
        "domain": "docket",
        "summary": summary,
        "item_counts": task_list_item_counts(&outcome.task_list),
        "sync": outcome,
    }))
}

pub(crate) async fn checkout_task_list(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: TaskListCheckoutArguments = serde_json::from_value(arguments)?;
    let Some(job_id) = args.job_id else {
        return Err(DenError::ValidationError(
            "checkout_task_list requires job_id; provide a Docket job id to project as a task list"
                .to_string(),
        )
        .into());
    };
    let source = TaskListCheckoutSource::DocketJob {
        job_id,
        parent_task_id: args.parent_task_id,
    };
    let task_list = PgDocketService::from_pool(pool)
        .checkout_task_list(
            context.bear_id,
            role,
            context.user_id,
            TaskListCheckoutRequest { source },
        )
        .await?;
    let summary = task_list
        .as_ref()
        .map(task_list_summary)
        .unwrap_or_else(|| format!("No Docket job found for {job_id}."));
    let item_counts = task_list.as_ref().map(task_list_item_counts);
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "summary": summary,
        "found": task_list.is_some(),
        "item_counts": item_counts,
        "task_list": task_list,
    }))
}

#[cfg(test)]
mod test {
    use super::*;

    fn status_report_with(
        current_task_title: Option<&str>,
        next_action: &str,
    ) -> DocketJobStatusReport {
        DocketJobStatusReport {
            job_id: Uuid::nil(),
            run_id: Some(Uuid::nil()),
            job_status: "running".to_string(),
            run_state: Some("running".to_string()),
            current_task_id: None,
            current_task_title: current_task_title.map(str::to_string),
            task_counts: den_docket::DocketCountByStatus {
                pending: 0,
                in_progress: 0,
                done: 0,
                blocked: 0,
                cancelled: 0,
            },
            criteria_counts: den_docket::DocketCriteriaCountByStatus {
                unmet: 0,
                met: 0,
                waived: 0,
            },
            tasks_complete: false,
            criteria_complete: true,
            next_action: next_action.to_string(),
        }
    }

    #[test]
    fn focused_conversation_title_uses_current_task() {
        let report = status_report_with(Some("Implement the slice"), "continue");
        assert_eq!(
            focused_conversation_title("Roadmap cleanup", &report),
            "Roadmap cleanup - Implement the slice"
        );
    }

    #[test]
    fn focused_conversation_title_truncates_to_title_limit() {
        let report = status_report_with(Some("Task"), "continue");
        let title = focused_conversation_title(&"g".repeat(200), &report);
        assert_eq!(title.chars().count(), FOCUSED_CONVERSATION_TITLE_MAX_CHARS);
    }

    #[test]
    fn oriented_task_create_policy_parses_runtime_snapshot() {
        let task_id = Uuid::new_v4();
        let runtime = json!({
            "objective_orientation": {
                "kind": "oriented",
                "task": {
                    "task_ref": {
                        "kind": "docket_task",
                        "job_id": null,
                        "task_id": task_id.to_string(),
                        "title": "Root"
                    },
                    "child_policy": {
                        "max_children": 3,
                        "max_depth_below_oriented_task": 1
                    }
                }
            }
        });

        assert_eq!(
            oriented_task_create_policy(Some(&runtime)),
            Some(OrientedTaskCreatePolicy {
                root_task_id: task_id,
                max_children: 3,
                max_depth_below_oriented_task: 1,
            })
        );
    }

    #[test]
    fn oriented_new_child_depth_counts_depth_below_root() {
        let root = Uuid::new_v4();
        let child = Uuid::new_v4();
        let grandchild_parent = Uuid::new_v4();
        let descendants = vec![
            task_projection(child, Some(root)),
            task_projection(grandchild_parent, Some(child)),
        ];

        assert_eq!(oriented_new_child_depth(root, root, &descendants), Some(1));
        assert_eq!(oriented_new_child_depth(root, child, &descendants), Some(2));
        assert_eq!(
            oriented_new_child_depth(root, grandchild_parent, &descendants),
            Some(3)
        );
    }

    #[test]
    fn oriented_capacity_rejects_child_count_and_depth_caps() {
        let policy = OrientedTaskCreatePolicy {
            root_task_id: Uuid::new_v4(),
            max_children: 2,
            max_depth_below_oriented_task: 1,
        };

        assert!(validate_oriented_child_capacity(policy, 1, 1).is_ok());

        let child_cap_error = validate_oriented_child_capacity(policy, 1, 2)
            .expect_err("child count at cap should reject");
        assert!(child_cap_error
            .to_string()
            .contains("at most 2 child tasks"));

        let depth_cap_error = validate_oriented_child_capacity(policy, 2, 0)
            .expect_err("depth beyond cap should reject");
        assert!(depth_cap_error.to_string().contains("at most 1 level(s)"));
    }

    fn task_projection(id: Uuid, parent_task_id: Option<Uuid>) -> den_docket::DocketTaskProjection {
        let now = time::OffsetDateTime::now_utc();
        den_docket::DocketTaskProjection {
            task: den_docket::DocketTaskRow {
                id,
                bear_id: Uuid::nil(),
                job_id: None,
                session_anchor_id: None,
                parent_task_id,
                sibling_order: 0,
                kind: "execution".to_string(),
                scope: "template".to_string(),
                title: "Task".to_string(),
                body: "Body".to_string(),
                completion_criteria: sqlx::types::Json(vec!["Done".to_string()]),
                difficulty: None,
                effort_hint: None,
                assigned_to_role: None,
                created_by_role: "pair".to_string(),
                created_by_user_id: None,
                created_by_agent_id: None,
                created_in_run_id: None,
                created_at: now,
                updated_at: now,
            },
            run_state: None,
        }
    }

    #[test]
    fn docket_tasks_card_content_includes_counts_and_titles() {
        let mut task = task_projection(Uuid::new_v4(), None);
        task.task.title = "Improve cards".to_string();
        let content = docket_tasks_card_content(&[task]);

        assert!(content.contains("Found 1 Docket task."));
        assert!(content.contains("1 pending"));
        assert!(content.contains("Improve cards"));
    }

    #[test]
    fn task_list_card_content_marks_planned_lists_as_not_started() {
        let now = time::OffsetDateTime::now_utc();
        let item = den_docket::TaskListItem {
            id: "item-1".to_string(),
            title: "Review plan".to_string(),
            summary: None,
            status: den_docket::TaskListItemStatus::Pending,
            blocked_reason: None,
            source_ref: den_docket::TaskListSourceRef::local(vec!["local:item-1".to_string()]),
            sync_state: den_docket::TaskListSyncState::Clean,
        };
        let task_list = TaskListProjection {
            id: Uuid::new_v4(),
            bear_id: Uuid::nil(),
            title: "Session tasks".to_string(),
            summary: "Tasks anchored to the current client session".to_string(),
            owner_profile: "pair".to_string(),
            visibility: "private_to_profile".to_string(),
            status: "planned".to_string(),
            version: 1,
            source_ref: den_docket::TaskListSourceRef::local(vec![
                "session_anchor:test".to_string()
            ]),
            items: vec![item.clone()],
            current_item: Some(item),
            source_conversation_id: None,
            source_client_session_id: None,
            handoff_intent_path: None,
            handoff_task_id: None,
            created_at: now,
            updated_at: now,
        };
        let content = task_list_card_content(&task_list);

        assert!(content.contains("Task list 'Session tasks' is planned"));
        assert!(content.contains("1 pending"));
        assert!(content.contains("Execution has not started."));
        assert!(content.contains("Review plan"));
    }

    mod state_tests {
        include!("../tests/workflow_state.rs");
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkDispatchArguments {
    task_id: Uuid,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    git_ref: Option<String>,
    /// Catalog image name on the sandbox provider (see get_work_catalog).
    #[serde(default)]
    image: Option<String>,
}

pub(crate) async fn dispatch_work(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    if !matches!(role, BearProfile::Chat | BearProfile::Pair) {
        return Err(CustomError::ValidationError(format!(
            "dispatch_work is available to chat and pair stances, not {}",
            role.as_str()
        )));
    }
    let args: WorkDispatchArguments = serde_json::from_value(arguments)?;
    let run = den_docket::work_runs::enqueue_work_run(
        pool,
        den_docket::work_runs::WorkRunEnqueue {
            bear_id: context.bear_id,
            task_id: args.task_id,
            root_name: args.root.as_deref().and_then(clean_optional),
            git_ref: args.git_ref.as_deref().and_then(clean_optional),
            image_name: args.image.as_deref().and_then(clean_optional),
            requested_by_user_id: Some(context.user_id),
        },
    )
    .await?;
    // Runs serialize per job: tell the model where this run sits in the
    // job's queue and what it is waiting behind.
    let queue = den_docket::work_runs::queued_run_positions(pool, &[run.id])
        .await?
        .into_iter()
        .next();
    let note = match &queue {
        Some(info) if info.waiting_on_run_id.is_some() => format!(
            "queued at position {} in the job's queue, behind active run {}; \
             runs within a job execute one at a time in dispatch order",
            info.position,
            info.waiting_on_run_id.unwrap_or_default()
        ),
        Some(info) if info.position > 1 => format!(
            "queued at position {} in the job's queue; runs within a job execute \
             one at a time in dispatch order",
            info.position
        ),
        _ => "queued for the dispatch worker; inspect progress with get_work_run".to_string(),
    };
    Ok(json!({
        "ok": true,
        "work_run_id": run.id,
        "state": run.state,
        "attempt": run.attempt,
        "task_id": run.task_id,
        "job_id": run.job_id,
        "queue": queue.map(|info| json!({
            "position": info.position,
            "waiting_on_run_id": info.waiting_on_run_id,
        })),
        "note": note,
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkRunListArguments {
    #[serde(default)]
    job_id: Option<Uuid>,
    #[serde(default)]
    task_id: Option<Uuid>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

pub(crate) async fn list_work_runs(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: WorkRunListArguments = serde_json::from_value(arguments)?;
    let runs = den_docket::work_runs::list_work_runs(
        pool,
        den_docket::work_runs::WorkRunListFilter {
            bear_id: Some(context.bear_id),
            job_id: args.job_id,
            task_id: args.task_id,
            state: args.state.as_deref().and_then(clean_optional),
            limit: args.limit.unwrap_or(50),
        },
    )
    .await?;
    let queue_by_run = work_run_queue_map(pool, &runs).await?;
    let items: Vec<Value> = runs
        .iter()
        .map(|run| {
            let mut item = work_run_summary_json(run);
            if let Some(queue) = queue_by_run.get(&run.id) {
                item["queue"] = queue.clone();
            }
            item
        })
        .collect();
    Ok(json!({ "ok": true, "work_runs": items }))
}

/// Queue placement for the queued runs in `runs`, keyed by run id (runs
/// serialize per job; see den-docket's `queued_run_positions`).
async fn work_run_queue_map(
    pool: &PgPool,
    runs: &[den_docket::work_runs::WorkRunRow],
) -> Result<std::collections::HashMap<Uuid, Value>, CustomError> {
    let queued_ids: Vec<Uuid> = runs
        .iter()
        .filter(|run| run.state == "queued")
        .map(|run| run.id)
        .collect();
    let infos = den_docket::work_runs::queued_run_positions(pool, &queued_ids).await?;
    Ok(infos
        .into_iter()
        .map(|info| {
            (
                info.run_id,
                json!({
                    "position": info.position,
                    "waiting_on_run_id": info.waiting_on_run_id,
                }),
            )
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkRunGetArguments {
    work_run_id: Uuid,
}

pub(crate) async fn get_work_run(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: WorkRunGetArguments = serde_json::from_value(arguments)?;
    let run = den_docket::work_runs::get_work_run(pool, args.work_run_id)
        .await?
        .filter(|run| run.bear_id == context.bear_id)
        .ok_or_else(|| {
            CustomError::NotFound(format!("work run not found: {}", args.work_run_id))
        })?;
    let mut value = work_run_summary_json(&run);
    value["work_surface"] = run.work_surface.clone().unwrap_or(Value::Null);
    // Result refs already carry the bounded log tail / diff captured at
    // harvest time; live logs for active runs are on the /work web UI.
    value["result_refs"] = run.result_refs.clone().unwrap_or(Value::Null);
    if let Some(queue) = work_run_queue_map(pool, std::slice::from_ref(&run))
        .await?
        .remove(&run.id)
    {
        value["queue"] = queue;
    }
    value["usage"] = run.usage.unwrap_or(Value::Null);
    Ok(json!({ "ok": true, "work_run": value }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkRunCancelArguments {
    work_run_id: Uuid,
}

pub(crate) async fn cancel_work_run(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    if !matches!(role, BearProfile::Chat | BearProfile::Pair) {
        return Err(CustomError::ValidationError(format!(
            "cancel_work_run is available to chat and pair stances, not {}",
            role.as_str()
        )));
    }
    let args: WorkRunCancelArguments = serde_json::from_value(arguments)?;
    let requested =
        den_docket::work_runs::request_work_run_cancel(pool, args.work_run_id, context.bear_id)
            .await?;
    Ok(json!({
        "ok": true,
        "cancel_requested": requested,
        "note": if requested {
            "the dispatch worker will tear the sandbox down and record the task as blocked"
        } else {
            "run is already terminal or unknown; nothing to cancel"
        },
    }))
}

/// Read the sandbox provider's roots + image catalog so the model can choose
/// a root and toolchain image before dispatching.
pub(crate) async fn get_work_catalog(
    pool: &PgPool,
    config: &crate::config::Config,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, CustomError> {
    let _ignored_arguments: Value = serde_json::from_value(arguments)?;

    // Managed surfaces assigned to this bear come from Den's database and are
    // available even when the provider is unreachable.
    let surfaces: Vec<Value> =
        den_service::work_surfaces::list_surfaces_for_bears(pool, &[context.bear_id])
            .await?
            .into_iter()
            .map(|surface| {
                json!({
                    "name": surface.name,
                    "description": surface.description,
                    "default_ref": surface.default_ref,
                    "default_image": surface.default_image,
                    "assigned": true,
                })
            })
            .collect();

    let Some(url) = config
        .sandbox_server_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        return Err(CustomError::ValidationError(
            "no sandbox provider configured (SANDBOX_SERVER_URL unset); work dispatch is unavailable"
                .to_string(),
        ));
    };
    let client = den_sandbox::SandboxClient::new(url, &config.sandbox_server_token);
    let catalog = client.catalog().await.map_err(|err| {
        CustomError::System(format!("sandbox provider catalog fetch failed: {err}"))
    })?;
    Ok(json!({
        "ok": true,
        "surfaces": surfaces,
        "images": catalog.images,
        "roots": catalog.roots,
        "notes": [
            "Prefer a `surfaces` name (managed work surfaces assigned to this bear) as create_job's work_surface_ref / dispatch_work's root.",
            "Pass an image name from `images` as dispatch_work's `image` to select a toolchain; omit it to use the surface's default.",
            "Roots with has_upstream=true are git-backed; pushable jobs publish to their upstream work branch."
        ],
    }))
}

fn work_run_summary_json(run: &den_docket::work_runs::WorkRunRow) -> Value {
    fn ts(value: Option<time::OffsetDateTime>) -> Value {
        value
            .and_then(|t| {
                t.format(&time::format_description::well_known::Rfc3339)
                    .ok()
            })
            .map_or(Value::Null, Value::String)
    }
    json!({
        "work_run_id": run.id,
        "state": run.state,
        "attempt": run.attempt,
        "job_id": run.job_id,
        "task_id": run.task_id,
        "cancel_requested": run.cancel_requested,
        "root": run.root_name,
        "git_ref": run.git_ref,
        "sandbox_id": run.sandbox_id,
        "sandbox_type": run.sandbox_type,
        "sandbox_strength": run.sandbox_strength,
        "result_summary": run.result_summary,
        "error": run.error,
        "queued_at": ts(Some(run.queued_at)),
        "started_at": ts(run.started_at),
        "finished_at": ts(run.finished_at),
    })
}
