use crate::{
    config::Config,
    core::runtime::provider::{
        acp_requires_runtime, classify_runtime_error, runtime_error_is_conflict_pending_approval,
        AcpTurnRunner, CancelTurnRequest, ContinueTurnRequest, ContinueTurnResult,
        InteractionRunStore, RetrievalService, RoleProfileRegistry, RoleRunner,
        RoleRuntimeBinding, RuntimeApprovalDecision, RuntimeContinuation, RuntimeConversationRef,
        RuntimeErrorCategory, RuntimeStartupCapabilities, RuntimeStreamContinuation,
        RuntimeToolResultStatus, ToolActuatorRegistry,
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

struct NoopRegistry;

impl ToolActuatorRegistry for NoopRegistry {}

impl RoleProfileRegistry for NoopRegistry {
    async fn resolve_compatibility_binding(
        &self,
        _bear_id: uuid::Uuid,
        _role: &str,
    ) -> Result<Option<RoleRuntimeBinding>, CustomError> {
        Ok(None)
    }
}

impl RoleRunner for NoopRegistry {
    async fn check_health(&self) -> Result<String, CustomError> {
        Ok("ok".to_string())
    }
}

impl InteractionRunStore for NoopRegistry {
    async fn check_health(&self) -> Result<String, CustomError> {
        Ok("ok".to_string())
    }
}

impl RetrievalService for NoopRegistry {
    async fn check_health(&self) -> Result<String, CustomError> {
        Ok("ok".to_string())
    }
}

impl AcpTurnRunner for NoopRegistry {
    async fn preflight_hygiene(
        &self,
        _binding: &RoleRuntimeBinding,
        _conversation: Option<&RuntimeConversationRef>,
        _reason: &str,
    ) -> Result<(), CustomError> {
        Ok(())
    }

    async fn start_turn(
        &self,
        _request: crate::core::runtime::provider::StartTurnRequest,
    ) -> Result<crate::core::runtime::provider::StartTurnResult, CustomError> {
        Ok(crate::core::runtime::provider::StartTurnResult {
            turn: None,
            stream: RuntimeStreamContinuation::Deferred,
        })
    }

    async fn continue_turn(
        &self,
        request: ContinueTurnRequest,
    ) -> Result<ContinueTurnResult, CustomError> {
        match request.continuation {
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
        }
        Ok(ContinueTurnResult {
            turn: None,
            stream: RuntimeStreamContinuation::Deferred,
        })
    }

    async fn cancel_turn(&self, _request: CancelTurnRequest) -> Result<crate::core::runtime::provider::CancelTurnResult, CustomError> {
        Ok(crate::core::runtime::provider::CancelTurnResult {
            skipped: false,
            detail: "cancelled".to_string(),
        })
    }
}

#[tokio::test]
async fn runtime_runner_contract_is_mockable_for_tool_result_continuations() {
    let registry = NoopRegistry;
    let result = registry
        .continue_turn(ContinueTurnRequest {
            conversation: RuntimeConversationRef {
                id: "conv-1".to_string(),
            },
            turn: None,
            binding: RoleRuntimeBinding {
                binding_id: "binding-1".to_string(),
                compatibility_backend: Some("test".to_string()),
            },
            continuation: RuntimeContinuation::ToolResult {
                tool_call_id: "tool-1".to_string(),
                approval_request_id: Some("approval-1".to_string()),
                status: RuntimeToolResultStatus::Ok,
                content: "ok".to_string(),
            },
        })
        .await
        .expect("tool result continuation should succeed");
    assert_eq!(result.stream, RuntimeStreamContinuation::Deferred);
}

#[tokio::test]
async fn runtime_runner_contract_is_mockable_for_approval_decisions() {
    let registry = NoopRegistry;
    let result = registry
        .continue_turn(ContinueTurnRequest {
            conversation: RuntimeConversationRef {
                id: "conv-1".to_string(),
            },
            turn: None,
            binding: RoleRuntimeBinding {
                binding_id: "binding-1".to_string(),
                compatibility_backend: Some("test".to_string()),
            },
            continuation: RuntimeContinuation::ApprovalDecision {
                approval_request_id: "approval-1".to_string(),
                tool_call_id: Some("tool-1".to_string()),
                decision: RuntimeApprovalDecision::Approve,
                reason: Some("looks good".to_string()),
            },
        })
        .await
        .expect("approval continuation should succeed");
    assert_eq!(result.stream, RuntimeStreamContinuation::Deferred);
}
