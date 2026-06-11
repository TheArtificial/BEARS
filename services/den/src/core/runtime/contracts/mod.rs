use crate::{config::Config, errors::CustomError};
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRuntimeBinding {
    /// Den-owned opaque handle for the configured compatibility/runtime binding for a Bear role.
    pub binding_id: String,
    /// Transitional compatibility backend name (for diagnostics and migration only).
    /// Prefer treating this as a runtime/provider label rather than a concrete vendor name.
    pub compatibility_backend: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConversationRef {
    /// Den-owned opaque runtime conversation handle. Backends may currently back this with a
    /// Letta `conv-*` id, but ACP should treat it as opaque.
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTurnRef {
    /// Den-owned opaque runtime turn handle. Backends may back this with a provider run id.
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnsureConversationRequest {
    pub bear_id: uuid::Uuid,
    pub role: String,
    pub acp_session_id: String,
    pub requested_selection: Option<String>,
    pub binding: RoleRuntimeBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnsureConversationResult {
    pub conversation: RuntimeConversationRef,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHistoryRecord {
    pub message_id: Option<String>,
    pub role: String,
    pub content: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHistoryPage {
    pub records: Vec<RuntimeHistoryRecord>,
    pub raw_payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartTurnRequest {
    pub conversation: RuntimeConversationRef,
    pub binding: RoleRuntimeBinding,
    pub human_message: String,
    pub runtime_context: Option<String>,
    pub acp_session_id: Option<String>,
    pub client_tools: Option<serde_json::Value>,
    pub stream_tokens: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeToolResultStatus {
    Ok,
    Error,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeApprovalDecision {
    Approve,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeContinuation {
    ToolResult {
        tool_call_id: String,
        approval_request_id: Option<String>,
        status: RuntimeToolResultStatus,
        content: String,
    },
    ApprovalDecision {
        approval_request_id: String,
        tool_call_id: Option<String>,
        decision: RuntimeApprovalDecision,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinueTurnRequest {
    pub conversation: RuntimeConversationRef,
    pub turn: Option<RuntimeTurnRef>,
    pub binding: RoleRuntimeBinding,
    pub continuation: RuntimeContinuation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelTurnRequest {
    pub conversation: RuntimeConversationRef,
    pub turn: Option<RuntimeTurnRef>,
    pub reason: Option<String>,
    pub binding: Option<RoleRuntimeBinding>,
    pub run_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartTurnResult {
    pub turn: Option<RuntimeTurnRef>,
    pub stream: RuntimeStreamContinuation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinueTurnResult {
    pub turn: Option<RuntimeTurnRef>,
    pub stream: RuntimeStreamContinuation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeContinuationEnvelope {
    pub stream: RuntimeStreamContinuation,
    pub turn: Option<RuntimeTurnRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeStreamContinuation {
    Deferred,
    BytesSse,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeEventParser {
    pub parse_json_event: RuntimeParserFn,
}

pub type RuntimeByteStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, crate::errors::CustomError>> + Send + 'static>>;
pub type RuntimeEventStream = Pin<
    Box<dyn Stream<Item = Result<RuntimeStreamEvent, crate::errors::CustomError>> + Send + 'static>,
>;

pub type RuntimeParserFn = fn(&serde_json::Value) -> Option<RuntimeStreamEvent>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelTurnResult {
    pub skipped: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCleanupRequest {
    pub conversation: RuntimeConversationRef,
    pub binding: RoleRuntimeBinding,
    pub acp_session_id: String,
    pub bear_id: uuid::Uuid,
    pub run_ids: Vec<String>,
    pub reason: String,
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCleanupResult {
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuntimeSemanticEvent {
    AssistantTextDelta {
        text: String,
    },
    StatusText {
        text: String,
    },
    RunProgress {
        kind: String,
        text: Option<String>,
        phase: Option<String>,
        detail: Option<serde_json::Value>,
    },
    RunPaused {
        reason: String,
        resume_token: Option<String>,
        expires_at: Option<String>,
    },
    ToolCallRequested {
        tool_call_id: String,
        tool_name: String,
        title: Option<String>,
        kind: Option<String>,
        arguments: serde_json::Value,
        approval_request_id: Option<String>,
        approval_required: bool,
        approval_reason: Option<String>,
        run_id: Option<String>,
    },
    Error {
        message: String,
        detail: Option<String>,
        error_type: Option<String>,
        request_id: Option<String>,
        context: Option<serde_json::Value>,
    },
    ConversationResolved {
        conversation: RuntimeConversationRef,
    },
    TurnCompleted {
        turn: Option<RuntimeTurnRef>,
    },
    TurnFailed {
        turn: Option<RuntimeTurnRef>,
        category: RuntimeErrorCategory,
        message: String,
    },
    TurnCancelled {
        turn: Option<RuntimeTurnRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuntimeStreamEvent {
    Semantic(RuntimeSemanticEvent),
    UntranslatedProviderEvent {
        value: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeErrorCategory {
    Unavailable,
    Misconfigured,
    InvalidIdentity,
    PermissionDenied,
    ConflictPendingApproval,
    Cancelled,
    Timeout,
    BackendProtocol,
    Internal,
}

#[allow(async_fn_in_trait)]
pub trait RuntimeHealthCheck {
    fn compatibility_backend_name(&self) -> &'static str;
    fn enabled(&self) -> bool;
    async fn check_health(&self) -> Result<String, CustomError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStartupCapabilities {
    pub acp_gateway_enabled: bool,
    pub runtime_required_for_acp: bool,
}

impl RuntimeStartupCapabilities {
    pub fn from_config(config: &Config) -> Self {
        Self {
            acp_gateway_enabled: config.acp_gateway_enabled,
            runtime_required_for_acp: config.acp_gateway_enabled,
        }
    }
}

pub fn acp_requires_runtime(config: &Config) -> bool {
    RuntimeStartupCapabilities::from_config(config).runtime_required_for_acp
}

#[allow(async_fn_in_trait)]
pub trait RoleProfileRegistry {
    async fn resolve_compatibility_binding(
        &self,
        bear_id: uuid::Uuid,
        profile: &str,
    ) -> Result<Option<RoleRuntimeBinding>, CustomError>;
}

#[allow(async_fn_in_trait)]
pub trait AcpConversationRuntime {
    async fn ensure_session_conversation(
        &self,
        request: EnsureConversationRequest,
    ) -> Result<EnsureConversationResult, CustomError>;

    async fn load_history(
        &self,
        binding: &RoleRuntimeBinding,
        conversation: &RuntimeConversationRef,
    ) -> Result<RuntimeHistoryPage, CustomError>;
}

/// ACP/native conversation materialization backend (create, verify, load_history).
/// Distinct from `runtime_conversations::RuntimeLettaConversationQueryBackend`,
/// which covers Letta HTTP list/query operations for operator UI and escape hatch.
#[allow(async_fn_in_trait)]
pub trait RuntimeConversationBackend {
    async fn create_conversation(
        &self,
        binding: &RoleRuntimeBinding,
    ) -> Result<RuntimeConversationRef, CustomError>;

    async fn verify_conversation_belongs_to_binding(
        &self,
        binding: &RoleRuntimeBinding,
        conversation_id: &str,
    ) -> Result<(), CustomError>;

    async fn load_history(
        &self,
        binding: &RoleRuntimeBinding,
        conversation: &RuntimeConversationRef,
    ) -> Result<RuntimeHistoryPage, CustomError>;
}

#[allow(async_fn_in_trait)]
pub trait RuntimeCancellationBackend {
    async fn cancel_turn(&self, request: CancelTurnRequest)
    -> Result<CancelTurnResult, CustomError>;

    async fn cleanup_stale_runtime(
        &self,
        request: RuntimeCleanupRequest,
    ) -> Result<RuntimeCleanupResult, CustomError>;
}

#[allow(async_fn_in_trait)]
pub trait RoleRunner {
    async fn check_health(&self) -> Result<String, CustomError>;
}

#[allow(async_fn_in_trait)]
pub trait InteractionRunStore {
    async fn check_health(&self) -> Result<String, CustomError>;
}

pub trait ToolActuatorRegistry {}

#[allow(async_fn_in_trait)]
pub trait RetrievalService {
    async fn check_health(&self) -> Result<String, CustomError>;
}

/// Classify a runtime-facing error into a stable Den-owned category for ACP/runner policy.
pub fn classify_runtime_error(err: &CustomError) -> RuntimeErrorCategory {
    let message = err.to_string().to_ascii_lowercase();
    if message.contains("waiting on an unresolved tool approval")
        || message.contains("waiting for approval")
        || message.contains("please approve or deny")
        || message.contains("requires_approval")
    {
        RuntimeErrorCategory::ConflictPendingApproval
    } else if message.contains("no active runs to cancel") {
        RuntimeErrorCategory::Cancelled
    } else if message.contains("not configured") || message.contains("letta is not configured") {
        RuntimeErrorCategory::Misconfigured
    } else if message.contains("not found for this bear") {
        RuntimeErrorCategory::InvalidIdentity
    } else if matches!(err, CustomError::Authorization(_)) {
        RuntimeErrorCategory::PermissionDenied
    } else if message.contains("timed out") || message.contains("timeout") {
        RuntimeErrorCategory::Timeout
    } else if message.contains("unavailable") {
        RuntimeErrorCategory::Unavailable
    } else {
        RuntimeErrorCategory::Internal
    }
}

pub fn runtime_error_is_conflict_pending_approval(err: &CustomError) -> bool {
    classify_runtime_error(err) == RuntimeErrorCategory::ConflictPendingApproval
}

pub fn runtime_error_is_no_active_runs_cancel(err: &CustomError) -> bool {
    err.to_string()
        .to_ascii_lowercase()
        .contains("no active runs to cancel")
}
