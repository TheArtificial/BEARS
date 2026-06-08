use crate::core::{
    acp_tools::{acp_tool_policy, acp_tool_policy_json_for_provider, AcpToolName},
    agent_loop::approvals::{create_native_approval, decide_native_approval, NativeApprovalDecision},
    runtime_contracts::RuntimeSemanticEvent,
};
use sqlx::PgPool;
use uuid::Uuid;

pub fn provider_tool_requires_approval(provider_name: &str) -> bool {
    acp_tool_policy_json_for_provider(provider_name)
        .get("approval_required")
        .and_then(|value| value.as_bool())
        .unwrap_or_else(|| {
            AcpToolName::from_provider_alias(provider_name)
                .map(|tool| acp_tool_policy(tool).approval_required)
                .unwrap_or(false)
        })
}

pub async fn maybe_pause_for_tool_approval(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    acp_session_id: &str,
    tool_call_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Option<RuntimeSemanticEvent> {
    if !provider_tool_requires_approval(tool_name) {
        return None;
    }
    let approval_id = create_native_approval(
        pool,
        bear_id,
        conversation_id,
        acp_session_id,
        tool_call_id,
        tool_name,
        arguments,
    )
    .await
    .ok()?;
    Some(RuntimeSemanticEvent::RunPaused {
        reason: "requires_approval".to_string(),
        resume_token: Some(approval_id),
        expires_at: None,
    })
}

pub async fn record_approval_decision(
    pool: &PgPool,
    approval_id: &str,
    approve: bool,
    reason: Option<&str>,
) -> Result<(), crate::errors::CustomError> {
    decide_native_approval(
        pool,
        approval_id,
        if approve {
            NativeApprovalDecision::Approve
        } else {
            NativeApprovalDecision::Deny
        },
        reason,
    )
    .await
}
