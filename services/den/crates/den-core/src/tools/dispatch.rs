//! The Den tool dispatcher (composition root).
//!
//! [`invoke_den_tool`] runs preflight argument validation, authorizes the caller
//! (membership + role + per-tool profile gating), and routes the tool to its
//! executor. Every capability the executors need is bundled into the
//! [`ToolContext`] supertrait so the dispatcher can take a single `&impl
//! ToolContext`; individual executors stay generic over the minimal sub-trait set
//! they consume. The `den` crate provides one `DenToolContext` implementing every
//! sub-trait. See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B — dispatcher).

use serde_json::Value;

use crate::DenError;

use crate::tools::{
    constants::{
        DEN_BEAR_ENVIRONMENT, DEN_BEAR_GET_SELF, DEN_BEAR_LIST_MEMBERS, DEN_CAPABILITIES_LIST_SELF,
        DEN_CHANNEL_GET_CONTEXT, DEN_CONVERSATION_SET_TITLE, DEN_CORE_WRITE_RESULT_SUMMARY,
        DEN_ENTITY_BROWSE, DEN_ENTITY_BROWSE_PROVIDER, DEN_ENTITY_LINK_MEMORY,
        DEN_ENTITY_LINK_MEMORY_PROVIDER, DEN_ENTITY_MERGE, DEN_ENTITY_MERGE_PROVIDER,
        DEN_ENTITY_RESOLVE, DEN_ENTITY_RESOLVE_PROVIDER, DEN_ENTITY_SPLIT, DEN_ENTITY_SPLIT_PROVIDER,
        DEN_ENTITY_WRITE_ACCESS_RULE, DEN_ENTITY_WRITE_ACCESS_RULE_PROVIDER, DEN_ENTITY_WRITE_ANCHOR,
        DEN_ENTITY_WRITE_ANCHOR_PROVIDER, DEN_JOB_CREATE, DEN_JOB_EVALUATE_CRITERION,
        DEN_JOB_EXECUTE, DEN_JOB_GET, DEN_JOB_LIST, DEN_JOB_UPDATE, DEN_MEMORY_APPLY_CORE_UPDATE,
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD, DEN_MEMORY_LIST_PROPOSALS,
        DEN_MEMORY_ORIENT_WORK_SURFACE, DEN_MEMORY_READ, DEN_MEMORY_READ_PROPOSAL,
        DEN_MEMORY_REQUEST_REVIEW, DEN_MEMORY_RESOLVE_PROPOSAL, DEN_MEMORY_SEARCH,
        DEN_MEMORY_STATUS, DEN_MEMORY_TREE, DEN_MEMORY_WRITE_ENTRY, DEN_OBSERVATION_WRITE,
        DEN_PLAN_MODE_CANCEL, DEN_PLAN_MODE_ENTER, DEN_PLAN_MODE_EXIT, DEN_PLAN_MODE_RECORD_APPROVAL,
        DEN_PLAN_MODE_STATUS, DEN_POLICY_GET_SELF, DEN_PROMPT_MEMORY_LIST, DEN_PROMPT_MEMORY_PATCH,
        DEN_PROMPT_MEMORY_UPSERT, DEN_RUN_WRITE_RESULT, DEN_SITUATION_GET, DEN_SITUATION_GET_PROVIDER,
        DEN_SKILL_APPROVE_PROPOSAL, DEN_SKILL_PROPOSE, DEN_SKILL_REJECT_PROPOSAL,
        DEN_TASK_APPROVE_INTENT, DEN_TASK_CREATE, DEN_TASK_LIST, DEN_TASK_LIST_CHECKOUT,
        DEN_TASK_LIST_SYNC, DEN_TASK_REJECT_INTENT, DEN_TASK_UPDATE, DEN_TASK_WRITE_INTENT,
        DEN_TOOL_OUTPUT_READ, DEN_USER_GET_CURRENT, DEN_WEB_FETCH, DEN_WEB_SEARCH,
        DEN_WORK_PLAN_GET_STATUS,
        DEN_WORK_PLAN_LIST, DEN_WORK_PLAN_REQUEST_HANDOFF, DEN_WORK_PLAN_UPDATE,
    },
    context::DenToolInvocationContext,
    conversation::ConversationTitleOps,
    environment::EnvironmentOps,
    identity::{self, BearDirectory},
    preflight::{prevalidate_tool_arguments, tool_warning_payload, ToolPreflight},
    web::WebFetcher,
    work_surface::WorkSurfaceOps,
    workflow::WorkPlanOps,
    {conversation, entity, environment, memory, plan_mode, prompt_memory, review, web, work_surface},
};

/// Composed bundle so the dispatcher can take one `&impl ToolContext`. Each
/// executor stays generic over only the sub-trait(s) it actually uses.
pub trait ToolContext:
    BearDirectory
    + ConversationTitleOps
    + EnvironmentOps
    + entity::EntityOps
    + WorkPlanOps
    + WorkSurfaceOps
    + WebFetcher
    + memory::RoleMemoryStore
    + prompt_memory::PromptMemoryStore
    + review::MemoryReviewStore
    + plan_mode::PlanModeOps
    + Send
    + Sync
{
}

pub async fn invoke_den_tool(
    ctx: &impl ToolContext,
    tool_name: &str,
    arguments: Value,
    context: DenToolInvocationContext,
) -> Result<Value, DenError> {
    match prevalidate_tool_arguments(tool_name, &arguments, &context)? {
        ToolPreflight::Proceed => {}
        ToolPreflight::Warning(warning) => {
            return Ok(tool_warning_payload(tool_name, warning));
        }
    }
    let role = identity::authorize_context(ctx, &context).await?;
    identity::authorize_tool_for_profile(tool_name, role)?;
    match tool_name {
        DEN_BEAR_GET_SELF => identity::get_bear_self(ctx, &context).await,
        DEN_USER_GET_CURRENT => identity::get_current_user(ctx, &context).await,
        DEN_BEAR_LIST_MEMBERS => identity::list_bear_members(ctx, &context).await,
        DEN_CAPABILITIES_LIST_SELF => Ok(identity::list_capabilities_self(&context, role)),
        DEN_CHANNEL_GET_CONTEXT => Ok(identity::channel_context(&context)),
        DEN_POLICY_GET_SELF => identity::policy_self(ctx, &context).await,
        DEN_SITUATION_GET | DEN_SITUATION_GET_PROVIDER => {
            environment::session_info(ctx, ctx, &context, role).await
        }
        DEN_CONVERSATION_SET_TITLE => {
            conversation::set_conversation_title(ctx, &context, arguments).await
        }
        DEN_WEB_FETCH => web::web_fetch(ctx, context.bear_id, &context.session_id, arguments).await,
        DEN_WEB_SEARCH => web::web_search(ctx, Some(context.bear_id), arguments).await,
        DEN_TOOL_OUTPUT_READ => Err(DenError::System(
            "tool_output_read is handled by the native runtime artifact store".to_string(),
        )),
        DEN_MEMORY_WRITE_ENTRY => {
            let current_user = ctx.current_user(context.user_id).await.ok();
            memory::write_memory_entry(
                ctx,
                &context,
                role,
                arguments,
                current_user.as_ref().map(|user| user.username.clone()),
                current_user.as_ref().and_then(|user| user.display_name.clone()),
            )
            .await
        }
        DEN_MEMORY_STATUS => memory::memory_status(ctx, ctx, context.bear_id, role).await,
        DEN_MEMORY_TREE => memory::memory_browse(ctx, context.bear_id, role).await,
        DEN_MEMORY_READ => memory::memory_read(ctx, context.bear_id, role, arguments).await,
        DEN_MEMORY_SEARCH => memory::memory_search(ctx, context.bear_id, role, arguments).await,
        DEN_ENTITY_BROWSE | DEN_ENTITY_BROWSE_PROVIDER => {
            entity::entity_browse(ctx, &context, role, arguments).await
        }
        DEN_ENTITY_RESOLVE | DEN_ENTITY_RESOLVE_PROVIDER => {
            entity::entity_resolve(ctx, &context, role, arguments).await
        }
        DEN_ENTITY_LINK_MEMORY | DEN_ENTITY_LINK_MEMORY_PROVIDER => {
            entity::entity_link_memory(ctx, &context, role, arguments).await
        }
        DEN_ENTITY_MERGE | DEN_ENTITY_MERGE_PROVIDER => {
            entity::entity_merge(ctx, &context, role, arguments).await
        }
        DEN_ENTITY_SPLIT | DEN_ENTITY_SPLIT_PROVIDER => {
            entity::entity_split(ctx, &context, role, arguments).await
        }
        DEN_ENTITY_WRITE_ACCESS_RULE | DEN_ENTITY_WRITE_ACCESS_RULE_PROVIDER => {
            entity::entity_write_access_rule(ctx, &context, role, arguments).await
        }
        DEN_ENTITY_WRITE_ANCHOR | DEN_ENTITY_WRITE_ANCHOR_PROVIDER => {
            entity::entity_write_anchor(ctx, &context, role, arguments).await
        }
        DEN_MEMORY_ORIENT_WORK_SURFACE => {
            work_surface::orient_work_surface(ctx, &context, role).await
        }
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD => {
            work_surface::create_work_surface_scaffold(ctx, &context, role, arguments).await
        }
        DEN_PROMPT_MEMORY_UPSERT => {
            prompt_memory::prompt_memory_upsert(ctx, context.bear_id, context.user_id, role, arguments)
                .await
        }
        DEN_PROMPT_MEMORY_LIST => {
            prompt_memory::prompt_memory_list(ctx, context.bear_id, role, arguments).await
        }
        DEN_PROMPT_MEMORY_PATCH => prompt_memory::prompt_memory_patch(ctx, role, arguments).await,
        DEN_MEMORY_REQUEST_REVIEW => {
            review::request_memory_review(ctx, &context, role, arguments).await
        }
        DEN_MEMORY_LIST_PROPOSALS => {
            review::list_memory_proposals(ctx, &context, role, arguments).await
        }
        DEN_MEMORY_READ_PROPOSAL => {
            review::read_memory_proposal(ctx, &context, role, arguments).await
        }
        DEN_MEMORY_RESOLVE_PROPOSAL => {
            review::resolve_memory_proposal(ctx, &context, role, arguments).await
        }
        DEN_MEMORY_APPLY_CORE_UPDATE => {
            review::apply_core_update(ctx, &context, role, arguments).await
        }
        DEN_WORK_PLAN_LIST => ctx.list_work_plans(&context, role, arguments).await,
        DEN_WORK_PLAN_GET_STATUS => ctx.get_work_plan_status(&context, role, arguments).await,
        DEN_WORK_PLAN_UPDATE => ctx.update_work_plan(&context, role, arguments).await,
        DEN_JOB_CREATE => ctx.create_job(&context, role, arguments).await,
        DEN_JOB_LIST => ctx.list_jobs(&context, role, arguments).await,
        DEN_JOB_GET => ctx.get_job(&context, role, arguments).await,
        DEN_JOB_UPDATE => ctx.update_job(&context, role, arguments).await,
        DEN_JOB_EXECUTE => ctx.execute_job(&context, role, arguments).await,
        DEN_JOB_EVALUATE_CRITERION => ctx.evaluate_criterion(&context, role, arguments).await,
        DEN_TASK_CREATE => ctx.create_task(&context, role, arguments).await,
        DEN_TASK_LIST => ctx.list_tasks(&context, role, arguments).await,
        DEN_TASK_UPDATE => ctx.update_task(&context, role, arguments).await,
        DEN_TASK_LIST_SYNC => ctx.sync_task_list(&context, role, arguments).await,
        DEN_TASK_LIST_CHECKOUT => ctx.checkout_task_list(&context, role, arguments).await,
        DEN_PLAN_MODE_ENTER => plan_mode::enter_plan_mode(ctx, &context, arguments).await,
        DEN_PLAN_MODE_STATUS => plan_mode::plan_mode_status(ctx, &context).await,
        DEN_PLAN_MODE_RECORD_APPROVAL => {
            plan_mode::record_plan_approval(ctx, &context, arguments).await
        }
        DEN_PLAN_MODE_EXIT => plan_mode::exit_plan_mode(ctx, &context, arguments).await,
        DEN_PLAN_MODE_CANCEL => plan_mode::cancel_plan_mode(ctx, &context, arguments).await,
        DEN_BEAR_ENVIRONMENT => environment::bear_environment(ctx, ctx, &context, role).await,
        DEN_OBSERVATION_WRITE => review::write_observation(ctx, &context, role, arguments).await,
        DEN_SKILL_PROPOSE
        | DEN_SKILL_APPROVE_PROPOSAL
        | DEN_SKILL_REJECT_PROPOSAL
        | DEN_WORK_PLAN_REQUEST_HANDOFF
        | DEN_TASK_WRITE_INTENT
        | DEN_TASK_APPROVE_INTENT
        | DEN_TASK_REJECT_INTENT
        | DEN_CORE_WRITE_RESULT_SUMMARY
        | DEN_RUN_WRITE_RESULT => Err(DenError::System(format!(
            "Den tool `{tool_name}` is registered and role-authorized but not implemented in this session module"
        ))),
        _ => Err(DenError::NotFound(format!("unknown Den tool: {tool_name}"))),
    }
}
