use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::tools::constants::{
    DEN_JOB_CREATE, DEN_JOB_EVALUATE_CRITERION, DEN_JOB_EXECUTE, DEN_JOB_GET, DEN_JOB_LIST,
    DEN_JOB_UPDATE, DEN_TASK_CREATE, DEN_TASK_LIST, DEN_TASK_LISTS_GET_STATUS, DEN_TASK_LISTS_LIST,
    DEN_TASK_LISTS_UPDATE, DEN_TASK_LIST_CHECKOUT, DEN_TASK_LIST_SYNC, DEN_TASK_UPDATE,
};
use den_docket::{
    self as work_plans, docket_job_status_report, DocketCommitPolicy, DocketCriterionStateUpdate,
    DocketCriterionStatus, DocketEffortHint, DocketJobCreate, DocketJobCriterionInput,
    DocketJobExecuteRequest, DocketJobListFilter, DocketJobStatus, DocketJobUpdate, DocketService,
    DocketTaskCreate, DocketTaskDefinitionPatch, DocketTaskDifficulty, DocketTaskInput,
    DocketTaskKind, DocketTaskListFilter, DocketTaskRunStateUpdate, DocketTaskScope,
    DocketTaskStatus, DocketTaskUpdate, DocketValidationError, PgDocketService,
    TaskListCheckoutRequest, TaskListCheckoutSource, TaskListListFilter, TaskListLookup,
    TaskListProjection, TaskListStatus, TaskListSyncRequest, TaskListUpdate, TaskListUpsert,
    TaskListVisibility,
};

use crate::{
    config::Config,
    core::tools::{
        memory_write::source_client_session_id, session::DenToolInvocationContext,
        support::clean_optional,
    },
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
            | DEN_TASK_LIST_SYNC
            | DEN_TASK_LIST_CHECKOUT
    )
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkPlanListArguments {
    #[serde(default, rename = "status")]
    pub(crate) statuses: Option<Vec<TaskListStatus>>,
    #[serde(default)]
    pub(crate) owner_profile: Option<BearProfile>,
    #[serde(default)]
    pub(crate) include_archived: bool,
    #[serde(default)]
    pub(crate) include_completed: bool,
    #[serde(default)]
    pub(crate) include_plan_mode: Option<bool>,
    #[serde(default)]
    pub(crate) include_artifacts: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkPlanGetStatusArguments {
    #[serde(default)]
    pub(crate) plan_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) source_conversation_id: Option<String>,
    #[serde(default)]
    pub(crate) source_client_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskListUpdateArguments {
    #[serde(default)]
    pub(crate) plan_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) expected_version: Option<i32>,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) summary: String,
    pub(crate) visibility: TaskListVisibility,
    pub(crate) status: TaskListStatus,
    #[serde(default)]
    pub(crate) items: Vec<work_plans::TaskListUpdateItem>,
    #[serde(default = "empty_json_object")]
    pub(crate) workspace_context: Value,
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

pub(crate) fn empty_json_object() -> Value {
    json!({})
}

pub(crate) async fn list_work_plans(
    pool: &PgPool,
    _config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
    activity_payload: fn(Option<&work_plans::TaskListLocalProjection>) -> Value,
    plan_mode_workplan_payload: fn(&plan_mode::PlanModeSessionRow) -> Value,
) -> Result<Value, CustomError> {
    let args: WorkPlanListArguments = serde_json::from_value(arguments)?;
    let include_plan_mode = args.include_plan_mode.unwrap_or(true);
    let include_artifacts = args.include_artifacts.unwrap_or(true);
    let statuses = args.statuses.or_else(|| {
        (!args.include_completed).then(|| vec![TaskListStatus::Active, TaskListStatus::Blocked])
    });
    let activity_rows = PgDocketService::from_pool(pool)
        .list_visible_work_plans(
            context.bear_id,
            role,
            context.user_id,
            TaskListListFilter {
                statuses,
                owner_profile: args.owner_profile,
                include_archived: args.include_archived,
            },
        )
        .await?;
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
    let activity_plans = activity_rows
        .iter()
        .map(|plan| activity_payload(Some(plan)))
        .collect::<Vec<_>>();
    let task_lists = activity_rows
        .iter()
        .map(work_plans::task_list_projection_from_local)
        .collect::<Vec<_>>();
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
        "plans": activity_rows,
        "activity_rows": activity_rows,
        "workplans": workplans,
        "plan_mode_gates": plan_mode_gates,
        "plan_artifacts": plan_artifacts,
        "linked_plan_artifact_paths": linked_artifact_paths,
        "notes": [
            "list_task_lists is a Bear-level task-list/planning view. It includes live session task lists, submitted/active workplan gates, and saved pair workplan artifacts when available.",
            "A workplan artifact in pair/plans/ may exist even when there is no active live activity plan; this is workplan-domain state, not semantic memory.",
            "Role fields are provenance and policy hints, not product ownership. Cross-role visibility is not cross-role execution authority."
        ],
    }))
}

pub(crate) async fn get_work_plan_status(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
    activity_payload: fn(Option<&work_plans::TaskListLocalProjection>) -> Value,
) -> Result<Value, CustomError> {
    let args: WorkPlanGetStatusArguments = serde_json::from_value(arguments)?;
    let lookup = TaskListLookup {
        plan_id: args.plan_id,
        source_conversation_id: args
            .source_conversation_id
            .or_else(|| clean_optional(&context.conversation_id)),
        source_client_session_id: args
            .source_client_session_id
            .or_else(|| source_client_session_id(context)),
    };
    let plan = PgDocketService::from_pool(pool)
        .get_visible_work_plan(context.bear_id, role, context.user_id, lookup)
        .await?;
    let task_list = plan
        .as_ref()
        .map(work_plans::task_list_projection_from_local);
    Ok(json!({
        "domain": "activity",
        "bear_id": context.bear_id,
        "task_list": task_list,
        "activity": activity_payload(plan.as_ref()),
        "plan": plan,
    }))
}

pub(crate) async fn update_work_plan(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
    activity_payload: fn(Option<&work_plans::TaskListLocalProjection>) -> Value,
) -> Result<Value, CustomError> {
    let mut args: TaskListUpdateArguments = serde_json::from_value(arguments)?;
    work_plans::normalize_task_list_item_ids(&mut args.items);
    let row = PgDocketService::from_pool(pool)
        .upsert_work_plan(TaskListUpsert {
            bear_id: context.bear_id,
            owner_profile: role,
            owner_agent_id: clean_optional(&context.binding_id),
            created_by_user_id: Some(context.user_id),
            source_conversation_id: clean_optional(&context.conversation_id),
            source_acp_session_id: None,
            source_client_session_id: source_client_session_id(context),
            source_channel: serde_json::to_value(&context.channel)?,
            plan_id: args.plan_id,
            expected_version: args.expected_version,
            update: TaskListUpdate {
                title: args.title,
                summary: args.summary,
                visibility: args.visibility,
                status: args.status,
                items: args.items,
                workspace_context: args.workspace_context,
            },
        })
        .await?;
    let plan = row;
    let task_list = work_plans::task_list_projection_from_local(&plan);
    Ok(json!({
        "domain": "activity",
        "bear_id": context.bear_id,
        "task_list": task_list,
        "activity": activity_payload(Some(&plan)),
        "plan": plan,
    }))
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
    let task_list = work_plans::task_list_projection_from_docket_job(&job, None);
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "job": job,
        "task_list": task_list,
        "notes": [
            "Created durable Docket job state; execution is not started by this tool.",
            "Use get_job for canonical job/task state and update_task_list for session focus."
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
        .map(|job| work_plans::task_list_projection_from_docket_job(job, None));
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
    let run_state = match (args.run_id, args.status) {
        (Some(run_id), Some(status)) => Some(DocketTaskRunStateUpdate {
            run_id,
            status,
            result_refs: args.result_refs,
            result_summary: args.result_summary,
        }),
        (None, Some(_)) => {
            return Err(DenError::ValidationError(
                "update_task status requires run_id".to_string(),
            )
            .into());
        }
        _ => None,
    };
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
            run_state,
        })
        .await?;
    Ok(json!({
        "domain": "docket",
        "bear_id": context.bear_id,
        "task": task,
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
