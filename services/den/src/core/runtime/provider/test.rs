use crate::{
    config::Config,
    core::runtime::provider::{
        acp_requires_runtime, classify_runtime_error, runtime_error_is_conflict_pending_approval,
        ContinueTurnRequest, ContinueTurnResult, RoleRuntimeBinding, RuntimeApprovalDecision,
        RuntimeContinuation, RuntimeConversationRef, RuntimeErrorCategory,
        RuntimeStartupCapabilities, RuntimeStreamContinuation, RuntimeToolResultStatus,
    },
    errors::CustomError,
};

#[test]
fn runtime_error_categories_are_stable_for_acp_policy() {
    let approval = CustomError::System(
        "Letta continue HTTP 409: waiting on an unresolved tool approval".to_string(),
    );
    assert_eq!(
        classify_runtime_error(&approval),
        RuntimeErrorCategory::ConflictPendingApproval
    );
    assert!(runtime_error_is_conflict_pending_approval(&approval));

    let misconfigured = CustomError::System("Letta is not configured (set LETTA_BASE_URL)".to_string());
    assert_eq!(
        classify_runtime_error(&misconfigured),
        RuntimeErrorCategory::Misconfigured
    );
}

#[test]
fn acp_requires_runtime_when_gateway_enabled() {
    let mut config = Config::test_stub();
    config.acp_gateway_enabled = true;
    assert!(acp_requires_runtime(&config));
}

#[test]
fn acp_does_not_require_letta_runtime_when_gateway_disabled() {
    let mut config = Config::test_stub();
    config.acp_gateway_enabled = false;
    assert!(!acp_requires_runtime(&config));
}

#[test]
fn startup_capabilities_reflect_current_acp_to_letta_requirement() {
    let mut config = Config::test_stub();
    config.acp_gateway_enabled = true;
    let caps = RuntimeStartupCapabilities::from_config(&config);
    assert!(caps.acp_gateway_enabled);
    assert!(caps.runtime_required_for_acp);

    config.acp_gateway_enabled = false;
    let caps = RuntimeStartupCapabilities::from_config(&config);
    assert!(!caps.acp_gateway_enabled);
    assert!(!caps.runtime_required_for_acp);
}

fn sample_tool_result_continuation() -> RuntimeContinuation {
    RuntimeContinuation::ToolResult {
        tool_call_id: "tool-1".to_string(),
        approval_request_id: Some("approval-1".to_string()),
        status: RuntimeToolResultStatus::Ok,
        content: "ok".to_string(),
    }
}

fn sample_approval_continuation() -> RuntimeContinuation {
    RuntimeContinuation::ApprovalDecision {
        approval_request_id: "approval-1".to_string(),
        tool_call_id: Some("tool-1".to_string()),
        decision: RuntimeApprovalDecision::Approve,
        reason: Some("looks good".to_string()),
    }
}

#[test]
fn runtime_continuation_tool_result_shape_is_stable() {
    match sample_tool_result_continuation() {
        RuntimeContinuation::ToolResult {
            tool_call_id,
            approval_request_id,
            status,
            content,
        } => {
            assert_eq!(tool_call_id, "tool-1");
            assert_eq!(approval_request_id.as_deref(), Some("approval-1"));
            assert_eq!(status, RuntimeToolResultStatus::Ok);
            assert_eq!(content, "ok");
        }
        _ => panic!("expected tool result continuation"),
    }
}

#[test]
fn runtime_continuation_approval_decision_shape_is_stable() {
    match sample_approval_continuation() {
        RuntimeContinuation::ApprovalDecision {
            approval_request_id,
            tool_call_id,
            decision,
            reason,
        } => {
            assert_eq!(approval_request_id, "approval-1");
            assert_eq!(tool_call_id.as_deref(), Some("tool-1"));
            assert_eq!(decision, RuntimeApprovalDecision::Approve);
            assert_eq!(reason.as_deref(), Some("looks good"));
        }
        _ => panic!("expected approval continuation"),
    }
}

#[tokio::test]
async fn runtime_continuation_request_round_trips_for_tool_results() {
    let _request = ContinueTurnRequest {
        conversation: RuntimeConversationRef {
            id: "conv-1".to_string(),
        },
        turn: None,
        binding: RoleRuntimeBinding {
            binding_id: "binding-1".to_string(),
            compatibility_backend: Some("test".to_string()),
        },
        continuation: sample_tool_result_continuation(),
    };
    let result = ContinueTurnResult {
        turn: None,
        stream: RuntimeStreamContinuation::Deferred,
    };
    assert_eq!(result.stream, RuntimeStreamContinuation::Deferred);
}

#[tokio::test]
async fn runtime_continuation_request_round_trips_for_approval_decisions() {
    let _request = ContinueTurnRequest {
        conversation: RuntimeConversationRef {
            id: "conv-1".to_string(),
        },
        turn: None,
        binding: RoleRuntimeBinding {
            binding_id: "binding-1".to_string(),
            compatibility_backend: Some("test".to_string()),
        },
        continuation: sample_approval_continuation(),
    };
    let result = ContinueTurnResult {
        turn: None,
        stream: RuntimeStreamContinuation::Deferred,
    };
    assert_eq!(result.stream, RuntimeStreamContinuation::Deferred);
}
