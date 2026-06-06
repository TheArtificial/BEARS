use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        acp_plan_mode::{self, AcpPlanModeRequestedBy, EnterPlanModeParams, SubmitPlanModeParams},
        acp_sessions,
        bears::BearAgentRole,
        memory_manager_head::MemfsWriteRoleMemoryEntryRequest,
        tools::{
            memfs::memfs_http_client,
            memory_write::source_acp_session_id,
            session::DenToolInvocationContext,
            support::{clean_optional, validate_bounded_text},
        },
        turn_state,
    },
    errors::CustomError,
};

#[derive(Debug, Deserialize)]
pub(crate) struct PlanModeEnterArguments {
    #[serde(default)]
    pub(crate) reason: String,
    #[serde(default)]
    pub(crate) previous_permission_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlanModeRecordApprovalArguments {
    pub(crate) approval_text: String,
    #[serde(default)]
    pub(crate) plan_mode_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlanModeExitArguments {
    pub(crate) title: String,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) plan_mode_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlanModeCancelArguments {
    #[serde(default)]
    pub(crate) plan_mode_id: Option<Uuid>,
}

pub(crate) async fn enter_plan_mode(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
    plan_mode_workplan_payload: fn(&acp_plan_mode::AcpPlanModeSessionRow) -> Value,
) -> Result<Value, CustomError> {
    let args: PlanModeEnterArguments = serde_json::from_value(arguments)?;
    let acp_session_id = source_acp_session_id(context).ok_or_else(|| {
        CustomError::ValidationError("ACP session id is required for plan mode".to_string())
    })?;
    let row = acp_plan_mode::enter_plan_mode(
        pool,
        EnterPlanModeParams {
            user_id: context.user_id,
            bear_id: context.bear_id,
            bear_slug: context.bear_slug.clone(),
            acp_session_id: acp_session_id.clone(),
            reason: args.reason,
            requested_by: AcpPlanModeRequestedBy::Pair,
            previous_permission_mode: args.previous_permission_mode,
        },
    )
    .await?;
    acp_sessions::set_current_mode(
        pool,
        context.user_id,
        context.bear_id,
        &acp_session_id,
        "plan",
    )
    .await?;
    Ok(json!({
        "domain": "workplan",
        "workplan": plan_mode_workplan_payload(&row),
        "plan_mode": row,
        "workflow_state": turn_state::turn_state_json(&crate::core::acp_tools::AcpResolvedSessionPolicy {
            mode_label: "Plan",
            tool_enablement: crate::core::acp_tools::AcpToolEnablementState::ReadOnly,
            plan_mode_state: Some(row.state.clone()),
        }, None),
        "mode_update": "plan",
        "instructions": [
            "Plan mode is active for this ACP session.",
            "Inspect, read, search, and use read-only Den tools as needed.",
            "Do not mutate workspace files, run non-read-only shell commands, or perform external side effects until the submitted plan is approved.",
            "Call den.plan_mode.exit with a concise markdown implementation plan when ready for user approval."
        ]
    }))
}

pub(crate) async fn plan_mode_status(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    plan_mode_workplan_payload: fn(&acp_plan_mode::AcpPlanModeSessionRow) -> Value,
    no_active_workplan_payload: fn() -> Value,
) -> Result<Value, CustomError> {
    let acp_session_id = source_acp_session_id(context).ok_or_else(|| {
        CustomError::ValidationError("ACP session id is required for plan mode".to_string())
    })?;
    let row =
        acp_plan_mode::active_for_session(pool, context.user_id, context.bear_id, &acp_session_id)
            .await?;
    let workplan = row
        .as_ref()
        .map(plan_mode_workplan_payload)
        .unwrap_or_else(no_active_workplan_payload);
    Ok(json!({
        "domain": "workplan",
        "bear_id": context.bear_id,
        "acp_session_id": acp_session_id,
        "workplan": workplan,
        "plan_mode": row,
        "active": row.is_some(),
    }))
}

pub(crate) async fn record_plan_approval(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
    plan_mode_workplan_payload: fn(&acp_plan_mode::AcpPlanModeSessionRow) -> Value,
) -> Result<Value, CustomError> {
    let args: PlanModeRecordApprovalArguments = serde_json::from_value(arguments)?;
    let approval_text = validate_bounded_text("approval_text", &args.approval_text, 1, 1000)?;
    let acp_session_id = source_acp_session_id(context).ok_or_else(|| {
        CustomError::ValidationError("ACP session id is required for plan approval".to_string())
    })?;
    let current = acp_plan_mode::get_for_session(
        pool,
        context.user_id,
        context.bear_id,
        &acp_session_id,
        args.plan_mode_id,
    )
    .await?
    .ok_or_else(|| {
        CustomError::NotFound("submitted ACP plan mode session not found".to_string())
    })?;
    if current.state != "submitted" {
        return Err(CustomError::ValidationError(format!(
            "plan approval requires a submitted plan; current state is {}",
            current.state
        )));
    }
    let row = acp_plan_mode::approve_plan_mode(
        pool,
        context.user_id,
        context.bear_id,
        &acp_session_id,
        current.id,
    )
    .await?;
    acp_sessions::set_current_mode(
        pool,
        context.user_id,
        context.bear_id,
        &acp_session_id,
        "write",
    )
    .await?;
    Ok(json!({
        "domain": "workplan",
        "ok": true,
        "workplan": plan_mode_workplan_payload(&row),
        "plan_mode": row,
        "workflow_state": turn_state::turn_state_json(&crate::core::acp_tools::AcpResolvedSessionPolicy {
            mode_label: "Write",
            tool_enablement: crate::core::acp_tools::AcpToolEnablementState::AllTools,
            plan_mode_state: Some(row.state.clone()),
        }, None),
        "mode_update": "write",
        "approval_text": approval_text,
        "content": "Plan approved by the authenticated human. Write mode is now enabled; implementation may proceed subject to normal ACP tool approvals.",
    }))
}

pub(crate) async fn exit_plan_mode(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    arguments: Value,
    plan_mode_workplan_payload: fn(&acp_plan_mode::AcpPlanModeSessionRow) -> Value,
) -> Result<Value, CustomError> {
    let args: PlanModeExitArguments = serde_json::from_value(arguments)?;
    let acp_session_id = source_acp_session_id(context).ok_or_else(|| {
        CustomError::ValidationError("ACP session id is required for plan mode".to_string())
    })?;
    let title = validate_bounded_text("title", &args.title, 1, 200)?;
    let body = validate_bounded_text("body", &args.body, 1, 50_000)?;
    let markdown = acp_plan_mode::render_plan_artifact_markdown(&title, &body);
    let memory_request = MemfsWriteRoleMemoryEntryRequest {
        kind: "plan".to_string(),
        title: title.clone(),
        body: markdown,
        tags: vec!["plan-mode".to_string(), "implementation-plan".to_string()],
        refs: None,
        lifecycle: Some(json!({ "scope": "role-local", "retention": "durable" })),
        source: Some(json!({
            "tool": crate::core::den_tools::DEN_PLAN_MODE_EXIT,
            "acp_session_id": acp_session_id,
            "conversation_id": clean_optional(&context.conversation_id),
        })),
        author: context.username.clone(),
        conversation_id: clean_optional(&context.conversation_id),
        session_id: Some(acp_session_id.clone()),
        acp_session_id: Some(acp_session_id.clone()),
        conversation_selection: context.conversation_selection.clone(),
        runtime_target: context.runtime_target.clone(),
        role_agent_id: Some(context.role_agent_id.clone()),
        agent_role: Some(BearAgentRole::Pair.as_str().to_string()),
        request_id: context.request_id.clone(),
    };
    let http = memfs_http_client("MemFS plan artifact client build failed")?;
    let memfs_response = crate::core::memory_manager_head::write_memfs_role_memory_entry(
        &http,
        &config.letta_memfs_service_url,
        context.bear_id,
        BearAgentRole::Pair.as_str(),
        &memory_request,
    )
    .await?;
    let Some(memfs_response) = memfs_response else {
        return Err(CustomError::System(
            "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)".to_string(),
        ));
    };
    let current_plan = acp_plan_mode::get_for_session(
        pool,
        context.user_id,
        context.bear_id,
        &acp_session_id,
        args.plan_mode_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("active ACP plan mode session not found".to_string()))?;
    let row = acp_plan_mode::submit_plan_artifact(
        pool,
        SubmitPlanModeParams {
            user_id: context.user_id,
            bear_id: context.bear_id,
            acp_session_id: acp_session_id.clone(),
            plan_mode_id: Some(current_plan.id),
            title,
            body,
            artifact_path: memfs_response.path.clone(),
            approval_request_id: Some(format!("plan-mode-{}", current_plan.id)),
        },
    )
    .await?;
    acp_sessions::set_current_mode(
        pool,
        context.user_id,
        context.bear_id,
        &acp_session_id,
        "plan",
    )
    .await?;
    Ok(json!({
        "domain": "workplan",
        "workplan": plan_mode_workplan_payload(&row),
        "plan_mode": row,
        "workflow_state": turn_state::turn_state_json(&crate::core::acp_tools::AcpResolvedSessionPolicy {
            mode_label: "Plan",
            tool_enablement: crate::core::acp_tools::AcpToolEnablementState::ReadOnly,
            plan_mode_state: Some(row.state.clone()),
        }, None),
        "artifact": {
            "domain": "workplan",
            "content_class": "workplan_artifact",
            "path": memfs_response.path,
            "entry_id": memfs_response.entry_id,
            "commit": memfs_response.commit,
        },
        "approval_required": false,
        "mode_update": "plan",
        "submitted_plan": {
            "title": row.plan_title,
            "body": row.plan_body,
            "artifact_path": row.plan_artifact_path,
        },
        "instructions": [
            "Present this plan artifact to the user if useful.",
            "If the authenticated human clearly approves the plan in chat, call record_plan_approval. Tool use remains governed by Den policy and ACP client approval."
        ]
    }))
}

pub(crate) async fn cancel_plan_mode(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
    plan_mode_workplan_payload: fn(&acp_plan_mode::AcpPlanModeSessionRow) -> Value,
) -> Result<Value, CustomError> {
    let args: PlanModeCancelArguments = serde_json::from_value(arguments)?;
    let acp_session_id = source_acp_session_id(context).ok_or_else(|| {
        CustomError::ValidationError("ACP session id is required for plan mode".to_string())
    })?;
    let row = acp_plan_mode::cancel_plan_mode(
        pool,
        context.user_id,
        context.bear_id,
        &acp_session_id,
        args.plan_mode_id,
    )
    .await?;
    acp_sessions::set_current_mode(
        pool,
        context.user_id,
        context.bear_id,
        &acp_session_id,
        "ask",
    )
    .await?;
    Ok(json!({
        "domain": "workplan",
        "workplan": plan_mode_workplan_payload(&row),
        "plan_mode": row,
        "workflow_state": turn_state::turn_state_json(&crate::core::acp_tools::AcpResolvedSessionPolicy {
            mode_label: "Ask",
            tool_enablement: crate::core::acp_tools::AcpToolEnablementState::ReadOnly,
            plan_mode_state: Some(row.state.clone()),
        }, None),
        "mode_update": "ask"
    }))
}
