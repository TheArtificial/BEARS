use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        acp_sessions,
        bears::{db as bears_db, db::role_is_bear_admin, BearProfile},
        memory::MemoryStoreManager,
        tools::descriptor::{builtin_den_tool_descriptors, builtin_den_tool_descriptors_for_role},
        user,
    },
    errors::CustomError,
};

use crate::core::tools::{
    preflight::{prevalidate_tool_arguments, tool_warning_payload, ToolPreflight},
    arguments::{DenToolChannelContext, SetConversationTitleArguments},
    constants::{
        DEN_BEAR_ENVIRONMENT, DEN_BEAR_GET_SELF, DEN_BEAR_LIST_MEMBERS,
        DEN_CAPABILITIES_LIST_SELF, DEN_CHANNEL_GET_CONTEXT, DEN_CONVERSATION_SET_TITLE,
        DEN_CORE_WRITE_RESULT_SUMMARY, DEN_MEMORY_APPLY_CORE_UPDATE,
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD, DEN_MEMORY_LIST_PROPOSALS,
        DEN_MEMORY_ORIENT_WORK_SURFACE, DEN_MEMORY_READ, DEN_MEMORY_READ_PROPOSAL,
        DEN_MEMORY_REQUEST_REVIEW, DEN_MEMORY_RESOLVE_PROPOSAL, DEN_MEMORY_SEARCH,
        DEN_MEMORY_STATUS, DEN_MEMORY_TREE, DEN_MEMORY_WRITE_ENTRY, DEN_OBSERVATION_WRITE,
        DEN_PLAN_MODE_CANCEL, DEN_PLAN_MODE_ENTER, DEN_PLAN_MODE_EXIT,
        DEN_PLAN_MODE_RECORD_APPROVAL, DEN_PLAN_MODE_STATUS, DEN_POLICY_GET_SELF,
        DEN_PROMPT_MEMORY_LIST, DEN_PROMPT_MEMORY_PATCH, DEN_PROMPT_MEMORY_UPSERT,
        DEN_RUN_WRITE_RESULT, DEN_SITUATION_GET, DEN_SITUATION_GET_PROVIDER,
        DEN_SKILL_APPROVE_PROPOSAL, DEN_SKILL_PROPOSE, DEN_SKILL_REJECT_PROPOSAL,
        DEN_TASK_APPROVE_INTENT, DEN_TASK_REJECT_INTENT, DEN_TASK_WRITE_INTENT,
        DEN_USER_GET_CURRENT, DEN_WEB_FETCH, DEN_WEB_SEARCH, DEN_WORK_PLAN_GET_STATUS,
        DEN_WORK_PLAN_LIST, DEN_WORK_PLAN_REQUEST_HANDOFF, DEN_WORK_PLAN_UPDATE,
    },
    memfs::{fetch_role_memory_tree, memfs_http_client},
    environment::{bear_environment, session_info},
    memory_read::{memory_browse, memory_read, memory_search, memory_status},
    memory_review::{
        apply_core_update, list_memory_proposals, read_memory_proposal,
        request_memory_review, resolve_memory_proposal,
    },
    memory_write::write_memory_entry,
    observations::write_observation,
    plan_mode::{
        cancel_plan_mode, enter_plan_mode, exit_plan_mode, plan_mode_status,
        record_plan_approval,
    },
    prompt_memory::{prompt_memory_list, prompt_memory_patch, prompt_memory_upsert},
    web::{web_fetch, web_search},
    workflow::{get_work_plan_status, list_work_plans, update_work_plan},
    work_surface::{
        build_work_surface_orientation_payload, collect_memory_tree_paths,
        create_work_surface_scaffold, infer_work_surface_hint, work_surface_candidate_slug,
    },
};

fn clean_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

async fn memory_orient_work_surface(
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearProfile,
) -> Result<Value, CustomError> {
    let hint_payload = infer_work_surface_hint(context, role);
    let candidate_slug = work_surface_candidate_slug(context);
    if config.uses_native_agent_runtime() {
        let stores = MemoryStoreManager::new(config);
        let store = stores.store_for_bear(context.bear_id).await?;
        let files =
            crate::core::memory::tools::sqlite_collect_role_logical_paths(&store, role.as_str())
                .await?;
        let orientation =
            build_work_surface_orientation_payload(role, &hint_payload, &files, candidate_slug);
        return Ok(json!({
            "ok": true,
            "configured": true,
            "storage": "sqlite",
            "bear_id": context.bear_id,
            "profile": role.as_str(),
            "orientation": orientation,
        }));
    }
    let http = memfs_http_client("MemFS work-surface orientation client build failed")?;
    let tree = fetch_role_memory_tree(&http, &config.letta_memfs_service_url, context.bear_id, role.as_str()).await?;
    let Some(tree) = tree else {
        return Ok(json!({
            "ok": false,
            "configured": false,
            "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)",
            "orientation": build_work_surface_orientation_payload(role, &hint_payload, &[], candidate_slug),
        }));
    };
    let mut files = Vec::new();
    collect_memory_tree_paths(&tree.files, &mut files);
    let orientation =
        build_work_surface_orientation_payload(role, &hint_payload, &files, candidate_slug);
    Ok(json!({
        "ok": tree.ok,
        "configured": true,
        "bear_id": context.bear_id,
        "profile": role.as_str(),
        "canonical_tip": tree.canonical_tip,
        "orientation": orientation,
    }))
}

fn format_rfc3339(value: time::OffsetDateTime) -> String {
    value.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| value.to_string())
}

async fn patch_letta_conversation_summary(
    config: &Config,
    conversation_id: &str,
    summary: &str,
) -> Result<(), CustomError> {
    let base_url = config.letta_base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return Err(CustomError::System(
            "Letta is not configured (set LETTA_BASE_URL)".to_string(),
        ));
    }
    let url = format!("{base_url}/v1/conversations/{conversation_id}");
    let mut request = reqwest::Client::new()
        .patch(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&json!({ "summary": summary }));
    let key = config.letta_api_key.trim();
    if !key.is_empty() {
        request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = request
        .send()
        .await
        .map_err(|err| CustomError::System(format!("Letta patch conversation failed: {err}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(CustomError::System(format!(
            "Letta patch conversation HTTP {status}: {text}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DenToolInvocationContext {
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub binding_id: String,
    pub profile: Option<BearProfile>,
    pub user_id: i32,
    pub username: Option<String>,
    pub membership_role: Option<String>,
    pub conversation_id: String,
    pub session_id: String,
    #[serde(default)]
    pub acp_session_id: Option<String>,
    #[serde(default)]
    pub conversation_selection: Option<String>,
    #[serde(default)]
    pub runtime_target: Option<String>,
    #[serde(default)]
    pub workspace_roots: Vec<String>,
    #[serde(default)]
    pub session_policy: Option<Value>,
    #[serde(default)]
    pub activity: Option<Value>,
    #[serde(default)]
    pub runtime: Option<Value>,
    #[serde(default)]
    pub context_budget: Option<Value>,
    pub request_id: Option<String>,
    #[serde(default)]
    pub channel: DenToolChannelContext,
}

pub async fn invoke_den_tool(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    tool_name: &str,
    arguments: Value,
    context: DenToolInvocationContext,
) -> Result<Value, CustomError> {
    match prevalidate_tool_arguments(tool_name, &arguments, &context)? {
        ToolPreflight::Proceed => {}
        ToolPreflight::Warning(warning) => {
            return Ok(tool_warning_payload(tool_name, warning));
        }
    }
    let role = authorize_context(pool, &context).await?;
    authorize_tool_for_role(tool_name, role)?;
    match tool_name {
        DEN_BEAR_GET_SELF => get_bear_self(pool, &context).await,
        DEN_USER_GET_CURRENT => get_current_user(pool, &context).await,
        DEN_BEAR_LIST_MEMBERS => list_bear_members(pool, &context).await,
        DEN_CAPABILITIES_LIST_SELF => list_capabilities_self(pool, &context).await,
        DEN_CHANNEL_GET_CONTEXT => Ok(channel_context(&context)),
        DEN_POLICY_GET_SELF => policy_self(pool, &context).await,
        DEN_SITUATION_GET | DEN_SITUATION_GET_PROVIDER => {
            session_info(pool, config, &context, role).await
        }
        DEN_CONVERSATION_SET_TITLE => {
            set_conversation_title(pool, config, &context, arguments).await
        }
        DEN_WEB_FETCH => web_fetch(pool, &context, arguments).await,
        DEN_WEB_SEARCH => web_search(pool, config, &context, arguments).await,
        DEN_MEMORY_WRITE_ENTRY => {
            write_memory_entry(pool, config, &context, role, arguments).await
        }
        DEN_MEMORY_STATUS => memory_status(pool, config, &context, role).await,
        DEN_MEMORY_TREE => memory_browse(config, &context, role).await,
        DEN_MEMORY_READ => memory_read(config, &context, role, arguments).await,
        DEN_MEMORY_SEARCH => memory_search(config, &context, role, arguments).await,
        DEN_MEMORY_ORIENT_WORK_SURFACE => {
            memory_orient_work_surface(config, &context, role).await
        }
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD => {
            create_work_surface_scaffold(config, stores, &context, role, arguments).await
        }
        DEN_PROMPT_MEMORY_UPSERT => prompt_memory_upsert(pool, &context, role, arguments).await,
        DEN_PROMPT_MEMORY_LIST => prompt_memory_list(pool, &context, role, arguments).await,
        DEN_PROMPT_MEMORY_PATCH => prompt_memory_patch(pool, &context, role, arguments).await,
        DEN_MEMORY_REQUEST_REVIEW => {
            request_memory_review(pool, config, stores, &context, role, arguments).await
        }
        DEN_MEMORY_LIST_PROPOSALS => {
            list_memory_proposals(pool, config, stores, &context, role, arguments).await
        }
        DEN_MEMORY_READ_PROPOSAL => {
            read_memory_proposal(pool, config, stores, &context, role, arguments).await
        }
        DEN_MEMORY_RESOLVE_PROPOSAL => {
            resolve_memory_proposal(pool, config, stores, &context, role, arguments).await
        }
        DEN_MEMORY_APPLY_CORE_UPDATE => {
            apply_core_update(pool, config, stores, &context, role, arguments).await
        }
        DEN_WORK_PLAN_LIST => {
            list_work_plans(
                pool,
                config,
                stores,
                &context,
                role,
                arguments,
                crate::core::tools::activity_payloads::activity_payload,
                crate::core::tools::activity_payloads::plan_mode_workplan_payload,
            )
            .await
        }
        DEN_WORK_PLAN_GET_STATUS => {
            get_work_plan_status(pool, &context, role, arguments, crate::core::tools::activity_payloads::activity_payload).await
        }
        DEN_WORK_PLAN_UPDATE => {
            update_work_plan(pool, &context, role, arguments, crate::core::tools::activity_payloads::activity_payload).await
        }
        DEN_PLAN_MODE_ENTER => {
            enter_plan_mode(pool, &context, arguments, crate::core::tools::activity_payloads::plan_mode_workplan_payload).await
        }
        DEN_PLAN_MODE_STATUS => {
            plan_mode_status(
                pool,
                &context,
                crate::core::tools::activity_payloads::plan_mode_workplan_payload,
                crate::core::tools::activity_payloads::no_active_workplan_payload,
            )
            .await
        }
        DEN_PLAN_MODE_RECORD_APPROVAL => {
            record_plan_approval(pool, &context, arguments, crate::core::tools::activity_payloads::plan_mode_workplan_payload).await
        }
        DEN_PLAN_MODE_EXIT => {
            exit_plan_mode(pool, config, stores, &context, arguments, crate::core::tools::activity_payloads::plan_mode_workplan_payload).await
        }
        DEN_PLAN_MODE_CANCEL => {
            cancel_plan_mode(pool, &context, arguments, crate::core::tools::activity_payloads::plan_mode_workplan_payload).await
        }
        DEN_BEAR_ENVIRONMENT => bear_environment(pool, config, &context, role).await,
        DEN_OBSERVATION_WRITE => {
            write_observation(pool, config, stores, &context, role, arguments).await
        }
        DEN_SKILL_PROPOSE
        | DEN_SKILL_APPROVE_PROPOSAL
        | DEN_SKILL_REJECT_PROPOSAL
        | DEN_WORK_PLAN_REQUEST_HANDOFF
        | DEN_TASK_WRITE_INTENT
        | DEN_TASK_APPROVE_INTENT
        | DEN_TASK_REJECT_INTENT
        | DEN_CORE_WRITE_RESULT_SUMMARY
        | DEN_RUN_WRITE_RESULT => Err(CustomError::System(format!(
            "Den tool `{tool_name}` is registered and role-authorized but not implemented in this session module"
        ))),
        _ => Err(CustomError::NotFound(format!(
            "unknown Den tool: {tool_name}"
        ))),
    }
}

async fn authorize_context(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<BearProfile, CustomError> {
    if !bears_db::user_may_use_bear(pool, context.user_id, context.bear_id).await? {
        return Err(CustomError::Authorization(
            "user is not a member of this bear".to_string(),
        ));
    }
    context_role(pool, context).await
}

async fn context_role(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<BearProfile, CustomError> {
    let agent_id = context.binding_id.trim();
    if agent_id.is_empty() {
        return Err(CustomError::Authorization(
            "Den tool context is missing binding_id".to_string(),
        ));
    }

    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT profile
        FROM bear_profile_bindings
        WHERE bear_id = $1
          AND binding_id = $2
        "#,
    )
    .bind(context.bear_id)
    .bind(agent_id)
    .fetch_optional(pool)
    .await?;
    let registered_profile: BearProfile = row
        .ok_or_else(|| {
            CustomError::Authorization("binding_id is not registered for this bear".to_string())
        })?
        .0
        .parse()
        .map_err(CustomError::System)?;
    if let Some(declared_profile) = context.profile {
        if declared_profile != registered_profile {
            return Err(CustomError::Authorization(format!(
                "Den tool context profile `{declared_profile}` does not match registered profile `{registered_profile}` for binding_id"
            )));
        }
    }
    Ok(registered_profile)
}

pub(crate) fn authorize_tool_for_role(tool_name: &str, role: BearProfile) -> Result<(), CustomError> {
    let descriptor = builtin_den_tool_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.name == tool_name)
        .ok_or_else(|| CustomError::NotFound(format!("unknown Den tool: {tool_name}")))?;
    if descriptor.allows_role(role) {
        Ok(())
    } else {
        Err(CustomError::Authorization(format!(
            "Den tool `{tool_name}` is not available to the `{role}` role"
        )))
    }
}

async fn get_bear_self(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    let bear = bears_db::get_bear(pool, context.bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    let member_count = bears_db::count_bear_members(pool, bear.id).await?;
    Ok(json!({
        "bear": {
            "bear_id": bear.id,
            "slug": bear.slug,
            "name": bear.name,
            "description": bear.description,
            "default_model": bear.default_model,
            "letta_agent_type": bear.letta_agent_type,
            "member_count": member_count,
            "created_at": format_rfc3339(bear.created_at),
            "updated_at": format_rfc3339(bear.updated_at)
        },
        "viewer": {
            "user_id": context.user_id,
            "username": context.username,
            "membership_role": context.membership_role,
            "is_bear_admin": role_is_bear_admin(context.membership_role.as_deref())
        }
    }))
}

async fn get_current_user(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    let current = user::user_by_id(pool, context.user_id).await?;
    Ok(json!({
        "user": {
            "user_id": current.id,
            "username": current.username,
            "display_name": current.display_name,
            "email_verified": current.email_verified.unwrap_or(false),
            "created_at": format_rfc3339(current.created.assume_utc())
        },
        "bear_membership": {
            "bear_id": context.bear_id,
            "role": context.membership_role,
            "is_bear_admin": role_is_bear_admin(context.membership_role.as_deref())
        }
    }))
}

async fn list_bear_members(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    let members = bears_db::list_members_for_bear(pool, context.bear_id).await?;
    let can_manage_members = role_is_bear_admin(context.membership_role.as_deref());
    let member_values: Vec<Value> = members
        .into_iter()
        .map(|member| {
            json!({
                "user_id": member.user_id,
                "username": member.username,
                "display_name": member.display_name,
                "role": member.role,
            })
        })
        .collect();
    Ok(json!({
        "bear_id": context.bear_id,
        "members": member_values,
        "policy": {
            "viewer_role": context.membership_role,
            "can_manage_members": can_manage_members,
            "redacted_fields": ["email"]
        }
    }))
}

async fn list_capabilities_self(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    let role = context_role(pool, context).await?;
    let descriptors = builtin_den_tool_descriptors_for_role(role);
    Ok(json!({
        "bear_id": context.bear_id,
        "channel": context.channel,
        "capabilities": descriptors,
    }))
}

fn channel_context(context: &DenToolInvocationContext) -> Value {
    json!({
        "bear_id": context.bear_id,
        "binding_id": context.binding_id,
        "profile": context.profile,
        "user_id": context.user_id,
        "conversation_id": context.conversation_id,
        "session_id": context.session_id,
        "request_id": context.request_id,
        "channel": context.channel,
    })
}

async fn policy_self(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    let member_count = bears_db::count_bear_members(pool, context.bear_id).await?;
    let is_bear_admin = role_is_bear_admin(context.membership_role.as_deref());
    Ok(json!({
        "bear_id": context.bear_id,
        "user_id": context.user_id,
        "membership_role": context.membership_role,
        "is_bear_admin": is_bear_admin,
        "can_chat": true,
        "can_read_bear_profile": true,
        "can_list_members": true,
        "can_manage_members": is_bear_admin,
        "can_list_capabilities": true,
        "can_read_channel_context": true,
        "member_count": member_count,
        "policy_notes": [
            "Den tool calls are scoped to the current trusted bear/user context.",
            "Emails and authentication internals are not exposed through these tools."
        ]
    }))
}

async fn set_conversation_title(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: SetConversationTitleArguments = serde_json::from_value(arguments)?;
    let title = args.title.trim().chars().take(120).collect::<String>();
    if title.is_empty() {
        return Err(CustomError::ValidationError(
            "conversation title cannot be empty".to_string(),
        ));
    }
    let conversation_id = clean_optional(&context.conversation_id).ok_or_else(|| {
        CustomError::ValidationError(
            "current conversation is not saved yet; send a message before setting its title"
                .to_string(),
        )
    })?;
    if conversation_id == "default" || conversation_id.starts_with("new-") {
        return Err(CustomError::ValidationError(
            "current conversation is not saved yet; send a message before setting its title"
                .to_string(),
        ));
    }
    patch_letta_conversation_summary(config, &conversation_id, &title).await?;
    let synced_acp_sessions = acp_sessions::set_title_for_bear_conversation(
        pool,
        context.bear_id,
        &conversation_id,
        &title,
    )
    .await?;
    Ok(json!({
        "ok": true,
        "conversation_id": conversation_id,
        "title": title,
        "synced_acp_sessions": synced_acp_sessions,
        "content": format!("Conversation title set to {title:?}."),
    }))
}

#[cfg(test)]
mod test;
