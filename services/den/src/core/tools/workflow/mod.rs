use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::tools::constants::{
    DEN_JOB_CREATE, DEN_JOB_EVALUATE_CRITERION, DEN_JOB_EXECUTE, DEN_JOB_GET, DEN_JOB_LIST,
    DEN_JOB_UPDATE, DEN_TASK_CREATE, DEN_TASK_LIST, DEN_TASK_LISTS_GET_STATUS, DEN_TASK_LISTS_LIST,
    DEN_TASK_LISTS_UPDATE, DEN_TASK_LIST_CHECKOUT, DEN_TASK_LIST_SYNC, DEN_TASK_UPDATE,
    DEN_TASK_UPDATE_CURRENT_STATUS, DEN_WORK_DISPATCH, DEN_WORK_RUN_CANCEL, DEN_WORK_RUN_GET,
    DEN_WORK_RUN_LIST,
};
use den_docket::{
    self as docket, docket_job_status_report, DocketCommitPolicy, DocketCriterionStateUpdate,
    DocketCriterionStatus, DocketEffortHint, DocketExecutionLookup, DocketJobCreate,
    DocketJobCriterionInput, DocketJobExecuteRequest, DocketJobListFilter, DocketJobStatus,
    DocketJobUpdate, DocketService, DocketTaskCreate, DocketTaskDefinitionPatch,
    DocketTaskDifficulty, DocketTaskInput, DocketTaskKind, DocketTaskListFilter,
    DocketTaskRunStateUpdate, DocketTaskScope, DocketTaskStatus, DocketTaskUpdate,
    DocketValidationError, PgDocketService, TaskListCheckoutRequest, TaskListCheckoutSource,
    TaskListProjection, TaskListSyncRequest, TaskListVisibility,
};

use crate::{
    config::Config,
    core::tools::{session::DenToolInvocationContext, support::clean_optional},
    errors::{CustomError, DenError},
};
use den_memory::{tools as sqlite_memory, MemoryStoreManager};
use den_runtime::plan_mode;
use den_service::bears::BearProfile;

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
    _pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
    activity_payload: fn(Option<&docket::TaskListLocalProjection>) -> Value,
) -> Result<Value, CustomError> {
    let _ignored_arguments: Value = serde_json::from_value(arguments)?;
    Ok(json!({
        "domain": "activity",
        "bear_id": context.bear_id,
        "viewer_role": role.as_str(),
        "task_list": null,
        "activity": activity_payload(None),
        "plan": null,
        "notes": [
            "Session-local task-list storage has been retired.",
            "Use create_job, get_job, checkout_task_list, sync_task_list, or task tools for durable task/job state."
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
        "update_task_list no longer writes session-local task lists; use create_job, checkout_task_list, sync_task_list, or task tools for durable task/job state".to_string(),
    )
    .into())
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
    let job = PgDocketService::from_pool(pool)
        .create_job(DocketJobCreate {
            bear_id: context.bear_id,
            created_by_user_id: context.user_id,
            created_by_role: role.as_str().to_string(),
            goal: args.goal,
            work_surface_ref: args.work_surface_ref,
            commit_policy: args.commit_policy,
            status: args.status,
            visibility: args.visibility,
            criteria: args.criteria,
            tasks: args.tasks,
        })
        .await?;
    let task_list = docket::task_list_projection_from_docket_job(&job, None);
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
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
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "job": job,
        "task_list": task_list,
        "status_report": status_report,
    }))
}

pub(crate) async fn update_job(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketJobUpdateArguments = serde_json::from_value(arguments)?;
    let job = PgDocketService::from_pool(pool)
        .update_job(DocketJobUpdate {
            bear_id: context.bear_id,
            job_id: args.job_id,
            actor_role: role,
            actor_user_id: Some(context.user_id),
            actor_agent_id: clean_optional(&context.binding_id),
            goal: args.goal,
            work_surface_ref: args
                .clear_work_surface_ref
                .then_some(None)
                .or_else(|| args.work_surface_ref.map(Some)),
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

pub(crate) async fn create_task(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: DocketTaskCreateArguments = serde_json::from_value(arguments)?;
    let task = PgDocketService::from_pool(pool)
        .create_task(DocketTaskCreate {
            bear_id: context.bear_id,
            job_id: args.job_id,
            session_anchor_id: args.session_anchor_id,
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
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "task": task,
        "notes": [
            "Created durable Docket task definition. Status and results remain run-scoped."
        ]
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
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
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
    let lookup = DocketExecutionLookup {
        session_id: Some(context.session_id.clone()),
        source_conversation_id: clean_optional(&context.conversation_id),
        source_client_session_id: context.client_session_id.clone(),
    };
    let Some(execution) = PgDocketService::from_pool(pool)
        .get_active_execution_session(context.bear_id, role, lookup)
        .await?
    else {
        return Err(DenError::ValidationError(
            "update_current_task_status needs an active Docket run for this session; call execute_job or checkout_task_list for the job first".to_string(),
        )
        .into());
    };
    let task = PgDocketService::from_pool(pool)
        .update_task(DocketTaskUpdate {
            bear_id: context.bear_id,
            task_id: args.task_id,
            actor_role: role,
            actor_user_id: Some(context.user_id),
            actor_agent_id: clean_optional(&context.binding_id),
            definition: DocketTaskDefinitionPatch::default(),
            run_state: Some(DocketTaskRunStateUpdate {
                run_id: execution.run_id,
                status: args.status,
                result_refs: args.result_refs,
                result_summary: args.result_summary,
            }),
        })
        .await?;
    let task_list = PgDocketService::from_pool(pool)
        .get_job(context.bear_id, execution.job_id)
        .await?
        .map(|job| den_docket::task_list_projection_from_docket_job(&job, None));
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "task": task,
        "docket": {
            "active_job_id": execution.job_id,
            "active_run_id": execution.run_id,
            "active_task_id": execution.task_id,
            "source": "docket_execution_session"
        },
        "task_list": task_list,
    }))
}

pub(crate) async fn sync_task_list(pool: &PgPool, arguments: Value) -> Result<Value, CustomError> {
    let args: TaskListSyncArguments = serde_json::from_value(arguments)?;
    let outcome = PgDocketService::from_pool(pool)
        .sync_task_list(TaskListSyncRequest {
            task_list: args.task_list,
        })
        .await?;
    Ok(json!({
        "domain": "docket",
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
            "checkout_task_list requires job_id; legacy work-plan lookup has been retired"
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
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "task_list": task_list,
    }))
}

#[cfg(test)]
mod test;

#[derive(Debug, Deserialize)]
pub(crate) struct WorkDispatchArguments {
    task_id: Uuid,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    git_ref: Option<String>,
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
            requested_by_user_id: Some(context.user_id),
        },
    )
    .await?;
    Ok(json!({
        "ok": true,
        "work_run_id": run.id,
        "state": run.state,
        "attempt": run.attempt,
        "task_id": run.task_id,
        "job_id": run.job_id,
        "note": "queued for the dispatch worker; inspect progress with get_work_run",
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
    let items: Vec<Value> = runs.iter().map(work_run_summary_json).collect();
    Ok(json!({ "ok": true, "work_runs": items }))
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

fn work_run_summary_json(run: &den_docket::work_runs::WorkRunRow) -> Value {
    fn ts(value: Option<time::OffsetDateTime>) -> Value {
        value
            .and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok())
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
