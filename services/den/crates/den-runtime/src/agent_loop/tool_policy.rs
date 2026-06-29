use den_core::client_tools::{
    client_tool_policy, client_tool_policy_json_for_provider, ClientToolName,
};
use den_core::tools::descriptor::builtin_den_tool_descriptor_for_provider_name;
use den_protocol::RuntimeSemanticEvent;
use sqlx::PgPool;
use uuid::Uuid;

use crate::agent_loop::approvals::{
    create_native_approval, decide_native_approval, NativeApprovalDecision,
};

/// Whether web chat (and other surfaces without interactive approval UI) may run this tool
/// without an explicit human approval step.
pub fn provider_tool_supports_unilateral_execution(provider_name: &str) -> bool {
    if let Some(descriptor) = builtin_den_tool_descriptor_for_provider_name(provider_name) {
        return descriptor.approval_policy == "never";
    }
    !provider_tool_requires_approval(provider_name)
}

pub fn provider_tool_requires_approval(provider_name: &str) -> bool {
    if let Some(descriptor) = builtin_den_tool_descriptor_for_provider_name(provider_name) {
        return descriptor.approval_policy != "never";
    }
    client_tool_policy_json_for_provider(provider_name)
        .get("approval_required")
        .and_then(|value| value.as_bool())
        .unwrap_or_else(|| {
            ClientToolName::from_provider_alias(provider_name)
                .map(|tool| client_tool_policy(tool).approval_required)
                .unwrap_or(false)
        })
}

pub async fn maybe_pause_for_tool_approval(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    client_session_id: &str,
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
        client_session_id,
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
) -> Result<(), den_core::DenError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn den_capabilities_list_self_supports_unilateral_execution() {
        assert!(provider_tool_supports_unilateral_execution("den_capabilities_list_self"));
        assert!(provider_tool_supports_unilateral_execution("den.capabilities.list_self"));
        assert!(!provider_tool_requires_approval("den_capabilities_list_self"));
    }

    #[test]
    fn armature_mutating_tools_do_not_support_unilateral_execution() {
        assert!(!provider_tool_supports_unilateral_execution("fs_edit_file"));
        assert!(provider_tool_requires_approval("fs_edit_file"));
    }
}
