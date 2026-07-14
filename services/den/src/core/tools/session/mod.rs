use serde_json::Value;
use sqlx::PgPool;

use crate::{
    config::Config,
    core::tools::{context::DenToolContext, workflow},
    errors::CustomError,
};
use den_core::tools::constants::{
    DEN_JOB_CREATE, DEN_JOB_EVALUATE_CRITERION, DEN_JOB_EXECUTE, DEN_JOB_GET, DEN_JOB_LIST,
    DEN_JOB_UPDATE, DEN_TASK_CREATE, DEN_TASK_LIST, DEN_TASK_LISTS_GET_STATUS, DEN_TASK_LISTS_LIST,
    DEN_TASK_LISTS_UPDATE, DEN_TASK_LIST_CHECKOUT, DEN_TASK_LIST_SYNC, DEN_TASK_UPDATE,
    DEN_TASK_UPDATE_CURRENT_STATUS, DEN_WORK_CATALOG, DEN_WORK_DISPATCH, DEN_WORK_RUN_CANCEL,
    DEN_WORK_RUN_GET, DEN_WORK_RUN_LIST,
};
use den_memory::MemoryStoreManager;
use den_service::bears::BearProfile;
use den_service::conversation::persistence as conversation_persistence;

// The per-call context value now lives in `den-tools` (it is data, not a
// capability), so tool executors can move there. Re-exported here so existing
// `core::tools::session::DenToolInvocationContext` paths and the ~17 in-`den`
// construction sites keep resolving unchanged.
pub use den_core::tools::context::DenToolInvocationContext;

pub async fn invoke_den_tool(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    tool_name: &str,
    arguments: Value,
    context: DenToolInvocationContext,
) -> Result<Value, CustomError> {
    if workflow::is_workflow_tool(tool_name) {
        return invoke_workflow_tool(pool, config, stores, tool_name, arguments, &context).await;
    }

    let ctx = DenToolContext::new(pool, config, stores);
    den_core::tools::dispatch::invoke_den_tool(&ctx, tool_name, arguments, context)
        .await
        .map_err(CustomError::from)
}

async fn invoke_workflow_tool(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    tool_name: &str,
    arguments: Value,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    reject_closed_freeform_task_definition(tool_name, context)?;

    let role = context.profile.unwrap_or(BearProfile::Pair);
    let value = match tool_name {
        DEN_TASK_LISTS_LIST => {
            workflow::list_task_lists(
                pool,
                config,
                stores,
                context,
                role,
                arguments,
                crate::core::tools::activity_payloads::activity_payload,
                crate::core::tools::activity_payloads::plan_mode_workplan_payload,
            )
            .await?
        }
        DEN_TASK_LISTS_GET_STATUS => {
            workflow::get_task_list_status(
                pool,
                context,
                role,
                arguments,
                crate::core::tools::activity_payloads::activity_payload,
            )
            .await?
        }
        DEN_TASK_LISTS_UPDATE => {
            workflow::update_task_list(
                pool,
                context,
                role,
                arguments,
                crate::core::tools::activity_payloads::activity_payload,
            )
            .await?
        }
        DEN_JOB_CREATE => workflow::create_job(pool, context, role, arguments).await?,
        DEN_JOB_LIST => workflow::list_jobs(pool, context, arguments).await?,
        DEN_JOB_GET => workflow::get_job(pool, context, arguments).await?,
        DEN_JOB_UPDATE => workflow::update_job(pool, context, role, arguments).await?,
        DEN_JOB_EXECUTE => workflow::execute_job(pool, context, role, arguments).await?,
        DEN_JOB_EVALUATE_CRITERION => {
            workflow::evaluate_criterion(pool, context, role, arguments).await?
        }
        DEN_TASK_CREATE => workflow::create_task(pool, context, role, arguments).await?,
        DEN_TASK_LIST => workflow::list_tasks(pool, context, arguments).await?,
        DEN_TASK_UPDATE => workflow::update_task(pool, context, role, arguments).await?,
        DEN_TASK_UPDATE_CURRENT_STATUS => {
            workflow::update_current_task_status(pool, context, role, arguments).await?
        }
        DEN_TASK_LIST_SYNC => workflow::sync_task_list(pool, arguments).await?,
        DEN_TASK_LIST_CHECKOUT => {
            workflow::checkout_task_list(pool, context, role, arguments).await?
        }
        DEN_WORK_DISPATCH => workflow::dispatch_work(pool, context, role, arguments).await?,
        DEN_WORK_RUN_LIST => workflow::list_work_runs(pool, context, arguments).await?,
        DEN_WORK_RUN_GET => workflow::get_work_run(pool, context, arguments).await?,
        DEN_WORK_RUN_CANCEL => workflow::cancel_work_run(pool, context, role, arguments).await?,
        DEN_WORK_CATALOG => workflow::get_work_catalog(pool, config, context, arguments).await?,
        _ => {
            return Err(CustomError::NotFound(format!(
                "unknown workflow tool: {tool_name}"
            )));
        }
    };

    Ok(value)
}

fn reject_closed_freeform_task_definition(
    tool_name: &str,
    context: &DenToolInvocationContext,
) -> Result<(), CustomError> {
    if closed_freeform_disallows_task_definition(tool_name, context.runtime.as_ref()) {
        return Err(CustomError::ValidationError(
            "task definition tools are unavailable in closed freeform orientation; focus or orient to a task before defining durable work"
                .to_string(),
        ));
    }

    Ok(())
}

fn closed_freeform_disallows_task_definition(tool_name: &str, runtime: Option<&Value>) -> bool {
    if !matches!(
        tool_name,
        DEN_JOB_CREATE | DEN_TASK_CREATE | DEN_TASK_UPDATE | DEN_TASK_LIST_SYNC
    ) {
        return false;
    }

    let Some(orientation) = runtime.and_then(|runtime| runtime.get("objective_orientation")) else {
        return false;
    };

    orientation.get("kind").and_then(Value::as_str) == Some("freeform")
        && orientation
            .get("policy")
            .and_then(|policy| policy.get("may_define_task"))
            .and_then(Value::as_bool)
            == Some(false)
}

pub(crate) struct DenConversationTitleOps<'a> {
    pub(crate) pool: &'a PgPool,
}

impl den_core::tools::conversation::ConversationTitleOps for DenConversationTitleOps<'_> {
    async fn set_title(
        &self,
        bear_id: uuid::Uuid,
        conversation_id: &str,
        title: &str,
    ) -> Result<u64, crate::errors::DenError> {
        conversation_persistence::set_conversation_title_and_sync_client_sessions(
            self.pool,
            bear_id,
            conversation_id,
            title,
        )
        .await
    }
}

#[cfg(test)]
mod test;

#[cfg(test)]
mod freeform_policy_tests {
    use super::*;

    fn closed_freeform_runtime() -> Value {
        serde_json::json!({
            "objective_orientation": {
                "kind": "freeform",
                "policy": { "may_define_task": false }
            }
        })
    }

    #[test]
    fn closed_freeform_rejects_task_definition_but_allows_status_updates() {
        let runtime = closed_freeform_runtime();

        assert!(closed_freeform_disallows_task_definition(
            DEN_TASK_CREATE,
            Some(&runtime)
        ));
        assert!(!closed_freeform_disallows_task_definition(
            DEN_TASK_UPDATE_CURRENT_STATUS,
            Some(&runtime)
        ));
    }
}

