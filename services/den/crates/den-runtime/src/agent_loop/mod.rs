//! Den-native in-process ReAct agent loop ([ADR-0035](../../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).

mod approvals;
mod assembler;
mod context;
mod key_memory_projection;
#[cfg(test)]
mod key_memory_projection_tests;
mod overflow_retry;
mod pending_tools;
mod policy;
mod runtime_context;
mod session_store;
mod session_stream;
mod step;
mod strategy;
mod tool_outcome;
mod tool_policy;
mod transcript;

pub use tool_outcome::{
    is_incomplete_tool_result, is_legacy_synthetic_interrupted_tool_result,
    tool_call_finished_event, tool_call_finished_event_for_content,
    tool_call_finished_event_for_incomplete, tool_message_counts_toward_llm_resolution,
    tool_result_content_indicates_error, tool_result_persistence_status,
    user_visible_tool_error_summary, user_visible_tool_summary, INCOMPLETE_TOOL_RESULT_MARK,
    LEGACY_SYNTHETIC_TOOL_RESULT_UNAVAILABLE,
};

pub use approvals::{
    create_native_approval, decide_native_approval, NativeApprovalDecision, NativeApprovalRow,
};
pub use assembler::{
    assemble_native_turn, assemble_native_turn_for_bear, assemble_native_turn_messages,
    assemble_native_turn_messages_for_bear, AssembleTurnContext, AssembledNativeTurn,
};
pub use context::{
    assemble_agent_messages, load_transcript_grouping_rows, load_transcript_messages,
    prune_messages_for_native_chat, repair_tool_call_message_chain,
};
pub use key_memory_projection::{
    project_key_memory, KeyMemoryProjectionCacheKey, KeyMemoryProjectionResult,
};
pub use overflow_retry::compact_session_messages_for_overflow;
pub use pending_tools::pending_tool_calls;
pub use policy::{select_strategy_profile, StrategyPolicyInput};
pub use session_store::{agent_loop_session_key, AgentLoopSession, AgentLoopSessionStore};
pub use session_stream::{NativeToolDispatchMode, SessionTrackingStream};
pub use step::{run_agent_step_stream, AgentStepOverflowContext};
pub use strategy::StrategyProfile;
pub use tool_policy::{
    maybe_pause_for_tool_approval, provider_tool_requires_approval,
    provider_tool_supports_unilateral_execution, record_approval_decision,
};
pub use transcript::{
    spawn_persist_incomplete_acp_tool_results, spawn_persist_web_chat_interrupted_turn,
    spawn_persist_web_chat_turn,
};
