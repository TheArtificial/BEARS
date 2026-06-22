use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::tools::workflow::WorkPlanOps;

use crate::{
    config::Config,
    core::{
        docket::{DocketService, PgDocketService},
        tools::{
            activity_payloads::{activity_payload, plan_mode_workplan_payload},
            memory_write::source_acp_session_id,
            session::DenToolInvocationContext,
            support::clean_optional,
        },
        work_plans::{
            self, WorkPlanListFilter, WorkPlanLookup, WorkPlanStatus, WorkPlanUpdate,
            WorkPlanUpsert, WorkPlanVisibility,
        },
    },
    errors::{CustomError, DenError},
};
use den_runtime::{
    bears::BearProfile,
    memory::{MemoryStoreManager, tools as sqlite_memory},
    plan_mode,
};

/// Concrete [`WorkPlanOps`] over the runtime pool/config/stores.
pub(crate) struct DenWorkPlanOps<'a> {
    pub(crate) pool: &'a PgPool,
    pub(crate) config: &'a Config,
    pub(crate) stores: &'a MemoryStoreManager,
}

impl WorkPlanOps for DenWorkPlanOps<'_> {
    async fn list_work_plans(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError> {
        list_work_plans(
            self.pool,
            self.config,
            self.stores,
            context,
            role,
            arguments,
            activity_payload,
            plan_mode_workplan_payload,
        )
        .await
        .map_err(CustomError::into_den)
    }

    async fn get_work_plan_status(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError> {
        get_work_plan_status(self.pool, context, role, arguments, activity_payload)
            .await
            .map_err(CustomError::into_den)
    }

    async fn update_work_plan(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError> {
        update_work_plan(self.pool, context, role, arguments, activity_payload)
            .await
            .map_err(CustomError::into_den)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkPlanListArguments {
    #[serde(default, rename = "status")]
    pub(crate) statuses: Option<Vec<WorkPlanStatus>>,
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
    pub(crate) source_acp_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkPlanUpdateArguments {
    #[serde(default)]
    pub(crate) plan_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) expected_version: Option<i32>,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) summary: String,
    pub(crate) visibility: WorkPlanVisibility,
    pub(crate) status: WorkPlanStatus,
    #[serde(default)]
    pub(crate) items: Vec<work_plans::WorkPlanItem>,
    #[serde(default = "empty_json_object")]
    pub(crate) workspace_context: Value,
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
    activity_payload: fn(Option<&work_plans::WorkPlanProjection>) -> Value,
    plan_mode_workplan_payload: fn(&plan_mode::PlanModeSessionRow) -> Value,
) -> Result<Value, CustomError> {
    let args: WorkPlanListArguments = serde_json::from_value(arguments)?;
    let include_plan_mode = args.include_plan_mode.unwrap_or(true);
    let include_artifacts = args.include_artifacts.unwrap_or(true);
    let statuses = args.statuses.or_else(|| {
        (!args.include_completed).then(|| vec![WorkPlanStatus::Active, WorkPlanStatus::Blocked])
    });
    let activity_rows = PgDocketService::from_pool(pool)
        .list_visible_work_plans(
            context.bear_id,
            role,
            context.user_id,
            WorkPlanListFilter {
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
        .map(work_plans::task_list_projection_from_work_plan)
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
                "acp_session_id": context.acp_session_id,
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
    activity_payload: fn(Option<&work_plans::WorkPlanProjection>) -> Value,
) -> Result<Value, CustomError> {
    let args: WorkPlanGetStatusArguments = serde_json::from_value(arguments)?;
    let lookup = WorkPlanLookup {
        plan_id: args.plan_id,
        source_conversation_id: args
            .source_conversation_id
            .or_else(|| clean_optional(&context.conversation_id)),
        source_acp_session_id: args
            .source_acp_session_id
            .or_else(|| source_acp_session_id(context)),
    };
    let plan = PgDocketService::from_pool(pool)
        .get_visible_work_plan(context.bear_id, role, context.user_id, lookup)
        .await?;
    let task_list = plan
        .as_ref()
        .map(work_plans::task_list_projection_from_work_plan);
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
    activity_payload: fn(Option<&work_plans::WorkPlanProjection>) -> Value,
) -> Result<Value, CustomError> {
    let mut args: WorkPlanUpdateArguments = serde_json::from_value(arguments)?;
    work_plans::normalize_work_plan_item_ids(&mut args.items);
    let row = PgDocketService::from_pool(pool)
        .upsert_work_plan(WorkPlanUpsert {
            bear_id: context.bear_id,
            owner_profile: role,
            owner_agent_id: clean_optional(&context.binding_id),
            created_by_user_id: Some(context.user_id),
            source_conversation_id: clean_optional(&context.conversation_id),
            source_acp_session_id: source_acp_session_id(context),
            source_channel: serde_json::to_value(&context.channel)?,
            plan_id: args.plan_id,
            expected_version: args.expected_version,
            update: WorkPlanUpdate {
                title: args.title,
                summary: args.summary,
                visibility: args.visibility,
                status: args.status,
                items: args.items,
                workspace_context: args.workspace_context,
            },
        })
        .await?;
    let plan = row
        .project_for_profile(role, context.user_id)?
        .ok_or_else(|| {
            CustomError::System("updated work plan was not visible to its owner".to_string())
        })?;
    let task_list = work_plans::task_list_projection_from_work_plan(&plan);
    Ok(json!({
        "domain": "activity",
        "bear_id": context.bear_id,
        "task_list": task_list,
        "activity": activity_payload(Some(&plan)),
        "plan": plan,
    }))
}

#[cfg(test)]
mod test;
