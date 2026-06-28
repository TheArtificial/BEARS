pub use den_protocol::{
    classify_runtime_error, edge_gateway_requires_runtime, runtime_error_is_conflict_pending_approval,
    runtime_error_is_no_active_runs_cancel, CancelTurnRequest,
    CancelTurnResult, ContinueTurnRequest, ContinueTurnResult, EnsureConversationRequest,
    EnsureConversationResult, InteractionRunStore, RetrievalService, RoleProfileRegistry,
    RoleRunner, RoleRuntimeBinding, RuntimeApprovalDecision, RuntimeByteStream,
    RuntimeContinuation, RuntimeConversationRef, RuntimeErrorCategory, RuntimeEventStream,
    RuntimeHealthCheck, RuntimeHistoryPage, RuntimeHistoryRecord, RuntimeSemanticEvent,
    RuntimeStartupCapabilities, RuntimeStreamContinuation, RuntimeStreamEvent,
    RuntimeToolResultStatus, RuntimeTurnRef, StartTurnRequest, StartTurnResult,
    SessionConversationRuntime, ToolActuatorRegistry,
};

#[cfg(test)]
mod tests;
