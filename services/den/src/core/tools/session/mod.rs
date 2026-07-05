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
    let role = context.profile.unwrap_or(BearProfile::Pair);
    let value = match tool_name {
        DEN_TASK_LISTS_LIST => {
            workflow::list_work_plans(
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
            workflow::get_work_plan_status(
                pool,
                context,
                role,
                arguments,
                crate::core::tools::activity_payloads::activity_payload,
            )
            .await?
        }
        DEN_TASK_LISTS_UPDATE => {
            workflow::update_work_plan(
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
        DEN_TASK_LIST_SYNC => workflow::sync_task_list(pool, arguments).await?,
        DEN_TASK_LIST_CHECKOUT => {
            workflow::checkout_task_list(pool, context, role, arguments).await?
        }
        _ => {
            return Err(CustomError::NotFound(format!(
                "unknown workflow tool: {tool_name}"
            )));
        }
    };

    Ok(value)
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
